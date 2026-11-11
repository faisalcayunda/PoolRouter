//! Job metrik DLQ (ARC-228, ADR-0001 rev 15 bagian 5a.9): menjumlahkan pesan
//! di semua DLQ upstream pool ke gauge `poolrouter_dlq_pesan` (tanpa label)
//! setiap siklus. Alert-nya `poolrouter_dlq_pesan > 0` (docs/runbook/dlq.md).
//!
//! Hanya leader yang menjalankannya (penjadwal), jadi gauge punya satu
//! sumber. Pool dibaca dari semua baris `upstream_pool`, bukan hanya yang
//! `aktif`: pool yang baru dinonaktifkan masih bisa menyimpan pesan di DLQ.
//! Satu pool gagal dibaca berarti total tidak lengkap; gauge tidak diubah
//! (nilai lama lebih jujur daripada angka rendah yang salah) dan siklus
//! melapor galat.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use poolrouter_core::queue::{metrik::dlq_diukur, nama_antrean_upstream};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::jadwal::Pekerjaan;
use crate::leader::KotakFutur;

/// Pembaca panjang DLQ. Dipisah dari broker supaya penjumlahan teruji tanpa
/// RabbitMQ.
pub trait UkurDlq: Send + Sync {
    /// Jumlah pesan di DLQ milik `antrean` (0 bila DLQ belum ada).
    fn jumlah_dlq(&self, antrean: &str) -> KotakFutur<'_, Result<u32, String>>;
}

/// Total pesan di DLQ semua `antrean`; galat pertama menggagalkan seluruhnya.
pub async fn total_dlq<U: UkurDlq + ?Sized>(antrean: &[String], ukur: &U) -> Result<u64, String> {
    let mut total = 0;
    for nama in antrean {
        total += u64::from(
            ukur.jumlah_dlq(nama)
                .await
                .map_err(|galat| format!("DLQ {nama}: {galat}"))?,
        );
    }
    Ok(total)
}

const SQL_POOL: &str = "SELECT upstream_pool_id FROM upstream_pool ORDER BY upstream_pool_id";

pub struct DlqMetrik<U: UkurDlq> {
    pool: PgPool,
    ukur: Arc<U>,
}

impl<U: UkurDlq> DlqMetrik<U> {
    pub fn baru(pool: PgPool, ukur: Arc<U>) -> Self {
        Self { pool, ukur }
    }
}

impl<U: UkurDlq> Pekerjaan for DlqMetrik<U> {
    fn nama(&self) -> &'static str {
        "dlq_metrik"
    }

    /// `true` bila ada pesan di DLQ.
    fn jalankan(&self, _sekarang: DateTime<Utc>) -> KotakFutur<'_, Result<bool, String>> {
        Box::pin(async move {
            let baris = sqlx::query(SQL_POOL)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| format!("baca upstream_pool: {e}"))?;
            let mut antrean = Vec::with_capacity(baris.len());
            for b in &baris {
                let id: Uuid = b.try_get("upstream_pool_id").map_err(|e| e.to_string())?;
                antrean.push(nama_antrean_upstream(id));
            }
            let total = total_dlq(&antrean, &*self.ukur).await?;
            dlq_diukur(total);
            if total > 0 {
                tracing::warn!(pesan = total, "DLQ berisi pesan; lihat docs/runbook/dlq.md");
            }
            Ok(total > 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Tiruan(HashMap<&'static str, Result<u32, &'static str>>);

    impl UkurDlq for Tiruan {
        fn jumlah_dlq(&self, antrean: &str) -> KotakFutur<'_, Result<u32, String>> {
            let hasil = self.0[antrean].map_err(str::to_owned);
            Box::pin(async move { hasil })
        }
    }

    fn nama(daftar: &[&str]) -> Vec<String> {
        daftar.iter().map(|n| (*n).to_owned()).collect()
    }

    #[tokio::test]
    async fn total_menjumlahkan_semua_dlq() {
        let ukur = Tiruan(HashMap::from([("a", Ok(2)), ("b", Ok(0)), ("c", Ok(5))]));
        assert_eq!(total_dlq(&nama(&["a", "b", "c"]), &ukur).await, Ok(7));
        assert_eq!(total_dlq(&[], &ukur).await, Ok(0));
    }

    #[tokio::test]
    async fn satu_dlq_gagal_dibaca_menggagalkan_total() {
        let ukur = Tiruan(HashMap::from([("a", Ok(2)), ("b", Err("putus"))]));
        let galat = total_dlq(&nama(&["a", "b"]), &ukur).await.unwrap_err();
        assert!(
            galat.contains("DLQ b") && galat.contains("putus"),
            "{galat}"
        );
    }
}
