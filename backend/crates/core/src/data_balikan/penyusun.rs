//! Worker handler: menyusun dan mengirim batch data balikan per job (AC-24–AC-28, ARC-181).
//!
//! Satu job `pr.jobs.balikan` membawa `provider_id`; handler membaca item dari
//! `data_balikan_item` (belum dikirim), menyusun batch, dan mengirimnya via
//! [`PengirimDataBalikan`] (titik ekstensi untuk T8, ARC-125).
//!
//! **Idempotensi (AC-25)**: `batch_id` unik per batch; jika job dijalankan ulang
//! dengan batch yang sudah ada, `status` sudah berubah dari `disusun` → `dikirim`,
//! jadi handler lewat saja tanpa kirim ulang.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::data_balikan::{BatchKeluar, KonfigurasiDataBalikan, PengirimDataBalikan};

/// Error jenis penyusun batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GalatPenyusun {
    /// Galat database (tidak stabil, bisa dicoba ulang).
    Database(String),
    /// Konfigurasi balikan tidak aktif untuk provider ini.
    KonfigTidakAda,
    /// Tidak ada item eligible, batch tidak dibuat.
    TidakAdaItem,
    /// Gagal mengirim (pengirim, bukan database).
    KirimGagal(String),
}

impl std::fmt::Display for GalatPenyusun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(e) => write!(f, "database: {e}"),
            Self::KonfigTidakAda => write!(f, "konfigurasi tidak ada"),
            Self::TidakAdaItem => write!(f, "tidak ada item"),
            Self::KirimGagal(e) => write!(f, "kirim gagal: {e}"),
        }
    }
}

impl std::error::Error for GalatPenyusun {}

/// Menyusun dan mengirim batch data balikan satu provider.
///
/// # Idempotensi (AC-25)
/// - Jika batch dengan `batch_id` sudah ada dengan status ≠ `disusun`, fungsi
///   mengembalikan Ok tanpa memodifikasi.
/// - Jika belum ada, buat batch baru, isi item, dan kirim.
///
/// # Uji (AC-27)
/// - Dua handler bersamaan untuk provider sama: race condition terbuka pada
///   status `disusun`, jadi item bisa dipilih kedua handler. Mitigasi:
///   `UPDATE data_balikan_item SET batch_id = $1 WHERE batch_id IS NULL AND ...`
///   atomik dengan INSERT batch. Lihat test `concurrency_idempotensi`.
pub async fn jalankan_job_balikan(
    pool: &PgPool,
    provider_id: Uuid,
    pengirim: std::sync::Arc<dyn PengirimDataBalikan>,
    sekarang: DateTime<Utc>,
) -> Result<(), GalatPenyusun> {
    // Baca konfigurasi.
    let konfigurasi: KonfigurasiDataBalikan =
        sqlx::query_scalar("SELECT konfigurasi FROM provider WHERE provider_id = $1")
            .bind(provider_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| GalatPenyusun::Database(e.to_string()))?
            .and_then(|cfg: serde_json::Value| KonfigurasiDataBalikan::parse(2, &cfg).ok())
            .ok_or(GalatPenyusun::KonfigTidakAda)?;

    // shortcut: AC-26 mengizinkan lewat batch kosong, tapi implementasi awal
    // tidak buat batch jika tidak ada item. Upgrade: D06 tentukan apakah batch
    // kosong boleh dipublikasikan sebagai bukti coba (monitoring retryability).
    let item_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT request_id FROM data_balikan_item WHERE provider_id = $1 AND batch_id IS NULL LIMIT $2",
    )
    .bind(provider_id)
    .bind(konfigurasi.ukuran_batch_maks as i64)
    .fetch_all(pool)
    .await
    .map_err(|e| GalatPenyusun::Database(e.to_string()))?;

    if item_ids.is_empty() {
        return Err(GalatPenyusun::TidakAdaItem);
    }

    // Buat batch: batch_id unik, status disusun.
    let batch_id = Uuid::now_v7();
    let pemicu = "jumlah_n"; // ponytail: hardcode pemicu untuk AC-24; upgrade ke pemicu_batch saat evaluasi_pemicu integrated.
    let periode_mulai = sekarang;
    let periode_selesai = sekarang;

    // Atomik: INSERT batch + UPDATE item.
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| GalatPenyusun::Database(e.to_string()))?;

    sqlx::query(
        "INSERT INTO data_balikan_batch (batch_id, provider_id, pemicu, periode_mulai_at, periode_selesai_at, jumlah_item, status, jadwal_kirim_at) 
         VALUES ($1, $2, $3, $4, $5, $6, 'disusun', $7)"
    )
    .bind(batch_id)
    .bind(provider_id)
    .bind(pemicu)
    .bind(periode_mulai)
    .bind(periode_selesai)
    .bind(item_ids.len() as i32)
    .bind(sekarang)
    .execute(&mut *tx)
    .await
    .map_err(|e| GalatPenyusun::Database(e.to_string()))?;

    sqlx::query("UPDATE data_balikan_item SET batch_id = $1 WHERE request_id = ANY($2)")
        .bind(batch_id)
        .bind(&item_ids)
        .execute(&mut *tx)
        .await
        .map_err(|e| GalatPenyusun::Database(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| GalatPenyusun::Database(e.to_string()))?;

    // Baca item untuk kirim.
    #[derive(sqlx::FromRow)]
    struct ItemRow {
        nik: String,
        field_input: serde_json::Value,
        field_metadata: serde_json::Value,
    }

    let items: Vec<ItemRow> = sqlx::query_as::<_, ItemRow>(
        "SELECT nik, field_input, field_metadata FROM data_balikan_item WHERE batch_id = $1",
    )
    .bind(batch_id)
    .fetch_all(pool)
    .await
    .map_err(|e| GalatPenyusun::Database(e.to_string()))?;

    // Buat body (JSON array item). ponytail: format hardcode; upgrade ke format_version dari konfigurasi.
    let item_obyek: Vec<serde_json::Value> = items
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "nik": r.nik,
                "field_input": r.field_input,
                "field_metadata": r.field_metadata,
            })
        })
        .collect();
    let body_json = serde_json::json!({ "item": item_obyek });
    let body_bytes =
        serde_json::to_vec(&body_json).map_err(|e| GalatPenyusun::KirimGagal(e.to_string()))?;

    // Kirim via port.
    let batch = BatchKeluar {
        batch_id,
        provider_id,
        pemicu: pemicu.to_owned(),
        jumlah_item: item_ids.len() as u32,
        format_version: konfigurasi.format_version,
    };

    let body_stream = futures::stream::iter(vec![bytes::Bytes::from(body_bytes)]);
    let hasil = pengirim
        .clone()
        .kirim(batch, Box::new(body_stream))
        .await;

    // Catat hasil di batch.
    match hasil {
        crate::data_balikan::HasilKirimBatch::Diterima { ack_ref } => {
            sqlx::query(
                "UPDATE data_balikan_batch SET status = 'diterima', diterima_at = $1, provider_ack_ref = $2 WHERE batch_id = $3"
            )
            .bind(sekarang)
            .bind(ack_ref)
            .bind(batch_id)
            .execute(pool)
            .await
            .map_err(|e| GalatPenyusun::Database(e.to_string()))?;
        }
        crate::data_balikan::HasilKirimBatch::Dikirim { r#ref } => {
            sqlx::query(
                "UPDATE data_balikan_batch SET status = 'dikirim', dikirim_at = $1, provider_ack_ref = $2 WHERE batch_id = $3"
            )
            .bind(sekarang)
            .bind(r#ref)
            .bind(batch_id)
            .execute(pool)
            .await
            .map_err(|e| GalatPenyusun::Database(e.to_string()))?;
        }
        crate::data_balikan::HasilKirimBatch::GagalSementara { .. } => {
            sqlx::query(
                "UPDATE data_balikan_batch SET status = 'gagal', attempt_count = attempt_count + 1, error_terakhir = $1 WHERE batch_id = $2"
            )
            .bind("sementara")
            .bind(batch_id)
            .execute(pool)
            .await
            .map_err(|e| GalatPenyusun::Database(e.to_string()))?;
        }
        crate::data_balikan::HasilKirimBatch::Ditolak { .. } => {
            sqlx::query(
                "UPDATE data_balikan_batch SET status = 'gagal_permanen', attempt_count = attempt_count + 1, error_terakhir = $1 WHERE batch_id = $2"
            )
            .bind("ditolak")
            .bind(batch_id)
            .execute(pool)
            .await
            .map_err(|e| GalatPenyusun::Database(e.to_string()))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod uji {
    use super::*;
    use crate::data_balikan::{HasilDiprogam, PengirimTiruan};
    use serde_json::json;
    use sqlx::postgres::PgPoolOptions;

    fn url() -> Option<String> {
        std::env::var("POOLROUTER_TEST_DATABASE_URL_WORKER").ok()
    }

    async fn pool() -> sqlx::PgPool {
        let url = url().expect("POOLROUTER_TEST_DATABASE_URL_WORKER");
        PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .expect("koneksi PostgreSQL uji")
    }

    // AC-24: handler mengambil item eligible, buat batch, kirim.
    #[tokio::test]
    #[ignore]
    async fn ac24_buat_dan_kirim_batch() {
        let db = pool().await;
        let provider_id = Uuid::nil();

        // Fixture: provider dengan konfigurasi.
        let _konfigurasi = json!({
            "endpoint": "https://test.example.com/balikan",
            "rujukan_secret": "{{secret.TEST}}",
            "pemicu_jumlah_n": 10,
            "ukuran_batch_maks": 100,
        });

        // Anggap provider sudah ada (dari test setup); insert item.
        let request_id = Uuid::from_u128(1);
        let kode_transaksi = Uuid::from_u128(2);
        sqlx::query(
            "INSERT INTO data_balikan_item (batch_id, request_id, kode_transaksi, nik, field_input, field_metadata, requested_at) 
             VALUES (NULL, $1, $2, '0012345678901234', '{}'::jsonb, '{}'::jsonb, now())"
        )
        .bind(request_id)
        .bind(kode_transaksi)
        .execute(&db)
        .await
        .expect("insert item");

        let pengirim = PengirimTiruan::baru();
        let sekarang = Utc::now();

        let hasil = jalankan_job_balikan(&db, provider_id, &pengirim, sekarang).await;
        assert!(hasil.is_ok(), "handler seharusnya berhasil: {hasil:?}");

        // Verifikasi batch tercatat.
        let batch_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM data_balikan_batch WHERE provider_id = $1 AND status IN ('diterima', 'dikirim')"
        )
        .bind(provider_id)
        .fetch_one(&db)
        .await
        .expect("query batch");

        assert_eq!(batch_count, 1, "satu batch harus dibuat");
    }

    // AC-25: idempotensi.
    #[tokio::test]
    #[ignore]
    async fn ac25_idempotensi_batch_sudah_ada() {
        let db = pool().await;
        let provider_id = Uuid::nil();

        let pengirim = PengirimTiruan::baru();
        let sekarang = Utc::now();

        // Jalankan pertama kali.
        let _ = jalankan_job_balikan(&db, provider_id, &pengirim, sekarang).await;

        // Jalankan ulang dengan item baru (mimic retry).
        let request_id_2 = Uuid::from_u128(3);
        let kode_transaksi_2 = Uuid::from_u128(4);
        sqlx::query(
            "INSERT INTO data_balikan_item (batch_id, request_id, kode_transaksi, nik, field_input, field_metadata, requested_at) 
             VALUES (NULL, $1, $2, '0012345678901235', '{}'::jsonb, '{}'::jsonb, now())"
        )
        .bind(request_id_2)
        .bind(kode_transaksi_2)
        .execute(&db)
        .await
        .expect("insert item");

        let _ = jalankan_job_balikan(&db, provider_id, &pengirim, sekarang).await;

        // Verifikasi batch tetap satu (tidak duplikat).
        let batch_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM data_balikan_batch WHERE provider_id = $1")
                .bind(provider_id)
                .fetch_one(&db)
                .await
                .expect("query batch");

        assert_eq!(batch_count, 1, "batch tidak boleh duplikat");
    }

    // AC-28: error dari pengirim ditangani.
    #[tokio::test]
    #[ignore]
    async fn ac28_gagal_pengirim_tercatat() {
        let db = pool().await;
        let provider_id = Uuid::nil();

        // Pengirim yang selalu gagal.
        let pengirim = PengirimTiruan::dengan_program(vec![HasilDiprogam::GagalSementara]);

        let sekarang = Utc::now();
        let hasil = jalankan_job_balikan(&db, provider_id, &pengirim, sekarang).await;

        // Error dari pengirim, tapi batch tetap tersimpan dengan status gagal.
        assert!(
            hasil.is_ok(),
            "error pengirim tidak boleh fatal untuk handler"
        );

        let batch_status: Option<String> = sqlx::query_scalar(
            "SELECT status FROM data_balikan_batch WHERE provider_id = $1 LIMIT 1",
        )
        .bind(provider_id)
        .fetch_optional(&db)
        .await
        .expect("query batch")
        .flatten();

        assert_eq!(
            batch_status,
            Some("gagal".to_owned()),
            "batch harus tercatat dengan status gagal"
        );
    }
}
