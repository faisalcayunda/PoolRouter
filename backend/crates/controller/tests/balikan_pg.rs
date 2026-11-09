//! Seam test: penjadwal balikan AC-23 dengan database nyata.
//!
//! Fixture provider: aktif+data_balikan_aktif (eligible), aktif saja, nonaktif.
//! Verifikasi: tepat satu pesan per provider eligible, queue pr.jobs.balikan, payload provider_id.
//! Kadens: t0 terbit, t0+30s lewat, t0+61s terbit lagi.
//!
//! Run by `make -C backend uji-pg`; `#[ignore]` because test needs
//! `POOLROUTER_TEST_DATABASE_URL_CONTROLLER`.

use std::sync::{Arc, Mutex as StdMutex};

use poolrouter_controller::balikan::{ANTREAN_BALIKAN, Balikan, jalankan_siklus_balikan};
use poolrouter_controller::jadwal::Pekerjaan;
use poolrouter_core::queue::JobPublisher;
use uuid::Uuid;

fn url() -> Option<String> {
    std::env::var("POOLROUTER_TEST_DATABASE_URL_CONTROLLER").ok()
}

async fn pool() -> sqlx::PgPool {
    let url = url().expect("POOLROUTER_TEST_DATABASE_URL_CONTROLLER");
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("koneksi PostgreSQL uji")
}

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
#[ignore]
async fn menerbitkan_tepat_satu_pesan_per_provider_eligible() {
    let db = pool().await;

    let eligible: Vec<Uuid> = sqlx::query_scalar(
        "SELECT provider_id FROM provider WHERE aktif = true AND data_balikan_aktif = true ORDER BY provider_id"
    )
    .fetch_all(&db)
    .await
    .expect("query provider eligible");

    let ineligible: Vec<Uuid> = sqlx::query_scalar(
        "SELECT provider_id FROM provider WHERE NOT (aktif = true AND data_balikan_aktif = true) ORDER BY provider_id"
    )
    .fetch_all(&db)
    .await
    .expect("query provider ineligible");

    let pesan = Arc::new(StdMutex::new(Vec::new()));
    let publisher = PublisherTiruan(pesan.clone());

    let sekarang = chrono::Utc::now();
    jalankan_siklus_balikan(&db, &publisher, sekarang)
        .await
        .expect("jalankan siklus");

    let hasil = pesan.lock().unwrap();

    assert_eq!(
        hasil.len(),
        eligible.len(),
        "harus ada {} pesan (provider eligible), dapat {}",
        eligible.len(),
        hasil.len()
    );

    for (i, &provider_id) in eligible.iter().enumerate() {
        assert_eq!(
            hasil[i].0, ANTREAN_BALIKAN,
            "pesan {} queue harus {}",
            i, ANTREAN_BALIKAN
        );
        assert_eq!(
            hasil[i].1, provider_id,
            "pesan {} payload harus provider_id {}",
            i, provider_id
        );
    }

    for inelig in ineligible {
        assert!(
            !hasil.iter().any(|(_, pid)| *pid == inelig),
            "provider ineligible {} tidak boleh terbit",
            inelig
        );
    }
}

#[tokio::test]
#[ignore]
async fn kadens_60s_melewati_30s_kemudian_terbit_di_61s() {
    let pesan = Arc::new(StdMutex::new(Vec::new()));
    let balikan = Balikan::baru(pool().await, PublisherTiruan(pesan.clone()));
    let t0 = chrono::Utc::now();

    assert_eq!(balikan.jalankan(t0).await, Ok(true), "t0 harus terbit");
    assert!(!pesan.lock().unwrap().is_empty());

    pesan.lock().unwrap().clear();
    assert_eq!(
        balikan.jalankan(t0 + chrono::Duration::seconds(30)).await,
        Ok(false),
        "t0+30s harus dilewati (belum 60 s)"
    );
    assert!(pesan.lock().unwrap().is_empty());

    assert_eq!(
        balikan.jalankan(t0 + chrono::Duration::seconds(61)).await,
        Ok(true),
        "t0+61s harus terbit lagi"
    );
    assert!(!pesan.lock().unwrap().is_empty());
}
