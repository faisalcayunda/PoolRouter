//! Worker handler: menyusun batch data balikan per job (AC-24–AC-28, ARC-123).
//!
//! Satu job `pr.jobs.balikan` membawa `provider_id`. Handler memilih request
//! eligible, memutuskan pemicu (AC-16), lalu menulis satu batch `disusun` beserta
//! itemnya. Pengiriman adalah tugas [`super::pengirim_handler`] (T6), sehingga
//! batch yang belum terkirim tetap terlihat di tabel dan bisa dikirim ulang.
//!
//! **Satu batch aktif per provider (AC-26)**: bila masih ada batch `disusun`,
//! `dikirim`, atau `gagal` untuk provider ini, job tidak menyusun batch baru.
//!
//! **Item sekali pakai (AC-25)**: request yang sudah punya baris
//! `data_balikan_item` tidak pernah dipilih lagi; `data_balikan_item_aktif_uq`
//! menjaga aturan itu di database juga.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::konfigurasi::KonfigurasiDataBalikan;
use super::metrik;
use super::pemicu::{PemicuBatch, evaluasi_pemicu};

/// Error jenis penyusun batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GalatPenyusun {
    /// Galat database (tidak stabil, bisa dicoba ulang).
    Database(String),
    /// Data balikan tidak aktif atau konfigurasinya tidak terbaca.
    KonfigTidakAda,
    /// Tidak ada item eligible dan tidak ada batch yang perlu disusun.
    TidakAdaItem,
    /// Provider masih punya batch yang belum selesai (AC-26).
    BatchAktif,
}

impl std::fmt::Display for GalatPenyusun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(e) => write!(f, "database: {e}"),
            Self::KonfigTidakAda => write!(f, "konfigurasi data balikan tidak ada"),
            Self::TidakAdaItem => write!(f, "tidak ada item eligible"),
            Self::BatchAktif => write!(f, "provider masih punya batch aktif"),
        }
    }
}

impl std::error::Error for GalatPenyusun {}

fn db(e: sqlx::Error) -> GalatPenyusun {
    GalatPenyusun::Database(e.to_string())
}

/// Membaca blok `data_balikan` milik provider, bila provider aktif dan
/// mengaktifkan data balikan (H-01).
///
/// Dipakai juga oleh pengirim (T6) supaya angka backoff dan batas percobaan
/// hanya dibaca di satu tempat.
pub(crate) async fn baca_konfigurasi(
    pool: &PgPool,
    provider_id: Uuid,
) -> Result<KonfigurasiDataBalikan, GalatPenyusun> {
    let baris: Option<(serde_json::Value, i32)> = sqlx::query_as(
        "SELECT v.dokumen, v.config_schema_version
           FROM provider p
           JOIN provider_config_versi v
             ON v.provider_id = p.provider_id AND v.versi = p.config_versi_aktif
          WHERE p.provider_id = $1 AND p.aktif AND p.data_balikan_aktif",
    )
    .bind(provider_id)
    .fetch_optional(pool)
    .await
    .map_err(db)?;
    let (dokumen, skema) = baris.ok_or(GalatPenyusun::KonfigTidakAda)?;
    // Dokumen provider menyimpan blok `data_balikan` di dalam objeknya; fixture
    // dan uji unit memberi bloknya langsung.
    let blok = dokumen.get("data_balikan").unwrap_or(&dokumen);
    KonfigurasiDataBalikan::parse(skema as u32, blok).map_err(|_| GalatPenyusun::KonfigTidakAda)
}

/// Menyusun satu batch `disusun` untuk provider ini, bila ada yang perlu dikirim.
pub async fn jalankan_job_balikan(
    pool: &PgPool,
    provider_id: Uuid,
    sekarang: DateTime<Utc>,
) -> Result<(), GalatPenyusun> {
    let konfigurasi = baca_konfigurasi(pool, provider_id).await?;

    #[derive(sqlx::FromRow)]
    struct Baris {
        request_id: Uuid,
        kode_transaksi: Uuid,
        nik: String,
        input_payload: serde_json::Value,
        field_metadata: serde_json::Value,
        completed_at: DateTime<Utc>,
        /// Jumlah seluruh item eligible, di luar `LIMIT` (DQ-20).
        tertunda: i64,
    }

    // AC-25: `done` dari provider, punya NIK dan payload, belum pernah jadi item.
    // Metadata hanya field yang dikonfigurasi `kirim_ke_data_balikan` dan bukan
    // `Rahasia` (AC-25 butir 3, H-02).
    let baris: Vec<Baris> = sqlx::query_as(
        "SELECT r.request_id, l.kode_transaksi, l.nik, r.input_payload,
                coalesce(md.meta, '{}'::jsonb) AS field_metadata,
                r.completed_at,
                count(*) OVER () AS tertunda
           FROM request r
           JOIN transaction_log l ON l.kode_transaksi = r.final_kode_transaksi
           LEFT JOIN LATERAL (
                SELECT jsonb_object_agg(f.nama_field, r.metadata -> f.nama_field) AS meta
                  FROM endpoint_metadata_field f
                 WHERE f.downstream_endpoint_id = r.downstream_endpoint_id
                   AND f.kirim_ke_data_balikan
                   AND f.klasifikasi <> 'Rahasia'
                   AND r.metadata ? f.nama_field
           ) md ON true
          WHERE r.provider_id = $1
            AND r.status = 'done'
            AND r.sumber_hasil = 'provider'
            AND r.input_payload IS NOT NULL
            AND r.payload_purged_at IS NULL
            AND l.nik IS NOT NULL
            AND NOT EXISTS (SELECT 1 FROM data_balikan_item i WHERE i.request_id = r.request_id)
          ORDER BY r.completed_at, r.request_id
          LIMIT $2",
    )
    .bind(provider_id)
    .bind(i64::from(konfigurasi.ukuran_batch_maks))
    .fetch_all(pool)
    .await
    .map_err(db)?;

    // DQ-20: backlog item eligible yang belum masuk batch mana pun.
    metrik::tertunda(baris.first().map_or(0, |b| b.tertunda as u64));

    // AC-26: satu batch aktif per provider. Batch gagal yang menunggu jadwal
    // retry juga aktif: menyusun batch baru di sebelahnya akan menggandakan
    // data yang sama ke provider.
    let aktif: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM data_balikan_batch
                         WHERE provider_id = $1 AND status IN ('disusun', 'dikirim', 'gagal'))",
    )
    .bind(provider_id)
    .fetch_one(pool)
    .await
    .map_err(db)?;
    if aktif {
        return Err(GalatPenyusun::BatchAktif);
    }

    if baris.is_empty() {
        return Err(GalatPenyusun::TidakAdaItem);
    }
    let tertua = baris.first().map(|b| b.completed_at);

    let periode_terakhir: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT max(disusun_at) FROM data_balikan_batch
          WHERE provider_id = $1 AND pemicu = 'periode'",
    )
    .bind(provider_id)
    .fetch_one(pool)
    .await
    .map_err(db)?;

    let pemicu = match evaluasi_pemicu(
        baris.len() as u32,
        tertua,
        sekarang,
        &konfigurasi,
        periode_terakhir,
    ) {
        PemicuBatch::Tidak => return Err(GalatPenyusun::TidakAdaItem),
        lain => lain,
    };

    let batch_id = Uuid::now_v7();
    let (periode_mulai, periode_selesai) = match pemicu {
        PemicuBatch::JumlahN => (sekarang, sekarang),
        PemicuBatch::Periode(batas) => (batas, sekarang),
        PemicuBatch::Tidak => unreachable!("sudah disaring di atas"),
    };

    let mut tx = pool.begin().await.map_err(db)?;

    // AC-24: dua handler bersamaan untuk provider yang sama — yang kalah
    // mengakui tanpa menulis apa pun. Kunci dilepas bersama transaksi.
    let dapat: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("poolrouter.balikan.{provider_id}"))
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
    if !dapat {
        return Ok(());
    }

    let versi_aktif: i32 =
        sqlx::query_scalar("SELECT config_versi_aktif FROM provider WHERE provider_id = $1")
            .bind(provider_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;

    sqlx::query(
        "INSERT INTO data_balikan_batch
                (batch_id, provider_id, pemicu, periode_mulai_at, periode_selesai_at,
                 jumlah_item, status, jadwal_kirim_at, format_version, disusun_at,
                 config_ref_version)
         VALUES ($1, $2, $3, $4, $5, $6, 'disusun', $7, $8, $7, $9)",
    )
    .bind(batch_id)
    .bind(provider_id)
    .bind(pemicu.sebagai_kolom().expect("pemicu bukan Tidak"))
    .bind(periode_mulai)
    .bind(periode_selesai)
    .bind(baris.len() as i32)
    .bind(sekarang)
    .bind(&konfigurasi.format_version)
    .bind(versi_aktif)
    .execute(&mut *tx)
    .await
    .map_err(db)?;

    for b in &baris {
        sqlx::query(
            "INSERT INTO data_balikan_item
                    (batch_id, request_id, kode_transaksi, nik, field_input,
                     field_metadata, requested_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(batch_id)
        .bind(b.request_id)
        .bind(b.kode_transaksi)
        .bind(&b.nik)
        .bind(&b.input_payload)
        .bind(&b.field_metadata)
        .bind(b.completed_at)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    }

    tx.commit().await.map_err(db)?;
    Ok(())
}

#[cfg(test)]
mod uji {
    use super::*;
    use serde_json::json;

    // Unit: blok `data_balikan` boleh bersarang di dokumen provider.
    #[test]
    fn blok_bersarang_dibaca() {
        let dokumen = json!({
            "data_balikan": {
                "endpoint": "https://contoh.test/kirim",
                "rujukan_secret": "{{secret.DUKCAPIL}}",
                "format_version": "tiruan-1"
            }
        });
        let blok = dokumen.get("data_balikan").unwrap();
        let k = KonfigurasiDataBalikan::parse(2, blok).expect("blok sah");
        assert_eq!(k.format_version, "tiruan-1");
    }
}
