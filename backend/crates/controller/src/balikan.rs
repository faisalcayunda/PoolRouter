//! Penjadwal controller data balikan (ARC-122, R7 T5, AC-23).
//!
//! Controller hanya **menerbitkan** `provider_id` ke antrean `pr.jobs.balikan`
//! (WB6: `pr_controller` tidak boleh membaca `data_balikan_*`). Penyusunan
//! batch di worker (AC-24–AC-28) adalah ARC-181.
//!
//! Kadens WB6 adalah 60 s, lebih jarang dari [`crate::jadwal::INTERVAL_SIKLUS`]
//! (10 s, dipakai job lain). Jadi job ini menjaga jam terbit terakhirnya
//! sendiri dan melewati siklus scheduler yang belum genap 60 s, daripada
//! menumpangi interval global.

use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use poolrouter_core::queue::JobPublisher;
use sqlx::PgPool;
use uuid::Uuid;

pub const ANTREAN_BALIKAN: &str = "pr.jobs.balikan";
/// Kadens penerbitan (WB6): tiap provider eligible dapat satu pesan per 60 s.
pub const INTERVAL_TERBIT: Duration = Duration::from_secs(60);

/// Penjadwal controller: satu [`crate::jadwal::Pekerjaan`] yang menerbitkan
/// satu pesan per provider `aktif AND data_balikan_aktif` setiap
/// [`INTERVAL_TERBIT`].
pub struct Balikan<P> {
    pool: PgPool,
    publisher: P,
    terakhir: Mutex<Option<DateTime<Utc>>>,
}

impl<P> Balikan<P> {
    pub fn baru(pool: PgPool, publisher: P) -> Self {
        Self {
            pool,
            publisher,
            terakhir: Mutex::new(None),
        }
    }
}

impl<P> crate::jadwal::Pekerjaan for Balikan<P>
where
    P: JobPublisher + Send + Sync + 'static,
{
    fn nama(&self) -> &'static str {
        "balikan"
    }

    fn jalankan(
        &self,
        sekarang: DateTime<Utc>,
    ) -> crate::jadwal::KotakFutur<'_, Result<bool, String>> {
        Box::pin(async move {
            {
                let mut terakhir = self.terakhir.lock().unwrap();
                if let Some(sebelumnya) = *terakhir
                    && sekarang - sebelumnya < chrono::Duration::from_std(INTERVAL_TERBIT).unwrap()
                {
                    return Ok(false);
                }
                *terakhir = Some(sekarang);
            }
            jalankan_siklus_balikan(&self.pool, &self.publisher, sekarang).await
        })
    }
}

/// Satu siklus penjadwal: baca provider aktif, terbitkan `provider_id`.
///
/// Standby tidak pernah masuk sini: [`crate::jadwal::satu_siklus`] memeriksa
/// `Leader::masih_leader` sebelum menjalankan `pekerjaan` apa pun (AC-23).
pub async fn jalankan_siklus_balikan<P>(
    pool: &PgPool,
    publisher: &P,
    _sekarang: DateTime<Utc>,
) -> Result<bool, String>
where
    P: JobPublisher,
{
    let provider_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT provider_id FROM provider WHERE aktif = true AND data_balikan_aktif = true ORDER BY provider_id",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut ada = false;
    for pid in provider_ids {
        publisher
            .publish(ANTREAN_BALIKAN, pid)
            .await
            .map_err(|e| e.to_string())?;
        ada = true;
    }
    Ok(ada)
}

#[cfg(test)]
mod uji {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};

    struct PublisherTiruan(Arc<StdMutex<Vec<(String, Uuid)>>>);

    impl JobPublisher for PublisherTiruan {
        async fn publish(
            &self,
            queue: &str,
            request_id: Uuid,
        ) -> Result<(), poolrouter_core::queue::QueueError> {
            self.0.lock().unwrap().push((queue.to_owned(), request_id));
            Ok(())
        }
    }

    #[tokio::test]
    async fn kadens_60s_melewati_siklus_yang_belum_genap() {
        // Tanpa PgPool nyata: hanya menguji gerbang kadens, bukan query.
        // Pool lazy tidak pernah membuka koneksi sampai dipakai.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://x:x@localhost/x")
            .unwrap();
        let pesan = Arc::new(StdMutex::new(Vec::new()));
        let balikan = Balikan::baru(pool, PublisherTiruan(pesan));

        let t0 = Utc::now();
        // Jalankan tanpa akses DB: gerbang kadens harus menolak siklus kedua
        // sebelum menyentuh `jalankan_siklus_balikan`.
        {
            let mut terakhir = balikan.terakhir.lock().unwrap();
            *terakhir = Some(t0);
        }
        let hasil = <Balikan<PublisherTiruan> as crate::jadwal::Pekerjaan>::jalankan(
            &balikan,
            t0 + chrono::Duration::seconds(30),
        )
        .await;
        assert_eq!(hasil, Ok(false), "siklus 30 s kemudian harus dilewati");
    }
}
