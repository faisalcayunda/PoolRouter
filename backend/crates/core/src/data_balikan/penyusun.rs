//! Worker handler: menyusun batch data balikan (AC-24–AC-28, ARC-181/T5b).
//!
//! Spec: products/poolrouter/specs/f1-webhook-retry-data-balikan/t5b-penyusun.md (ARC-195)

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::data_balikan::KonfigurasiDataBalikan;

/// Hasil satu putaran. Semua varian berarti pesan di-ack; hanya `Err` yang dicoba ulang.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HasilPenyusun {
    /// Kunci provider dipegang handler lain (penyusun atau pengirim). Tidak membaca atau menulis apa pun.
    TidakDapatKunci,
    /// Provider tidak aktif, `data_balikan_aktif = false`, tanpa versi aktif, dokumen skema 1, atau tanpa blok `data_balikan`.
    KonfigTidakAda,
    /// Blok ada tetapi `KonfigurasiDataBalikan::parse` menolak (dicatat dengan `provider_id` dan nama field).
    KonfigTidakValid,
    /// Ada batch `disusun`/`dikirim`/`gagal` (AC-26).
    BatchAktif,
    /// `evaluasi_pemicu` = `Tidak`.
    TidakAdaPemicu,
    Disusun {
        batch_id: Uuid,
        jumlah_item: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GalatPenyusun {
    /// Hanya SQLSTATE (atau jenis galat sqlx), tidak pernah pesan atau DETAIL Postgres, karena DETAIL
    /// pelanggaran CHECK memuat isi baris, termasuk NIK.
    #[error("database: {0}")]
    Database(String),
    /// Jumlah baris INSERT item ≠ jumlah request yang dipilih Q4. Transaksi di-rollback.
    #[error("item tertulis {tertulis}, dipilih {dipilih}")]
    ItemTidakCocok { dipilih: u64, tertulis: u64 },
}

fn db(e: sqlx::Error) -> GalatPenyusun {
    let msg = e
        .as_database_error()
        .and_then(|d| d.code())
        .map(|c| c.to_string())
        .unwrap_or_else(|| e.to_string());
    GalatPenyusun::Database(msg)
}

pub async fn jalankan_job_balikan(
    pool: &PgPool,
    provider_id: Uuid,
    sekarang: DateTime<Utc>,
) -> Result<HasilPenyusun, GalatPenyusun> {
    let mut tx = pool.begin().await.map_err(db)?;

    // Q1 kunci — statement pertama dalam transaksi.
    let kunci_dipegang: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("poolrouter.balikan.{}", provider_id))
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;

    if !kunci_dipegang {
        tx.rollback().await.map_err(db)?;
        return Ok(HasilPenyusun::TidakDapatKunci);
    }

    // Q2 konfigurasi — jalur baca: dokumen -> 'data_balikan' dengan config_schema_version diteruskan ke parse.
    let konfigurasi_row: Option<(i32, Option<serde_json::Value>, i32)> = sqlx::query_as(
        "SELECT v.config_schema_version, v.dokumen -> 'data_balikan' AS blok, p.config_versi_aktif
           FROM provider p
           JOIN provider_config_versi v ON v.provider_id = p.provider_id AND v.versi = p.config_versi_aktif
          WHERE p.provider_id = $1 AND p.aktif AND p.data_balikan_aktif"
    )
    .bind(provider_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;

    let (konfigurasi, config_ref_version) = match konfigurasi_row {
        None => {
            tx.rollback().await.map_err(db)?;
            return Ok(HasilPenyusun::KonfigTidakAda);
        }
        Some((schema_ver, blok, config_versi_aktif)) => {
            let blok = match blok {
                None => {
                    tx.rollback().await.map_err(db)?;
                    return Ok(HasilPenyusun::KonfigTidakAda);
                }
                Some(b) => b,
            };
            match KonfigurasiDataBalikan::parse(schema_ver as u32, &blok) {
                Ok(k) => (k, config_versi_aktif),
                Err(e) => {
                    tracing::warn!(%provider_id, galat = %e, "konfigurasi tidak valid");
                    tx.rollback().await.map_err(db)?;
                    return Ok(HasilPenyusun::KonfigTidakValid);
                }
            }
        }
    };

    // Q3 batch aktif (AC-26).
    let batch_aktif: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM data_balikan_batch
                        WHERE provider_id = $1 AND status IN ('disusun', 'dikirim', 'gagal'))",
    )
    .bind(provider_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;

    if batch_aktif {
        // Set gauge sebelum rollback (AC-28).
        let tertunda = hitung_tertunda(&mut tx, provider_id).await?;

        tx.rollback().await.map_err(db)?;
        metrics::gauge!("poolrouter_data_balikan_tertunda").set(tertunda as f64);
        return Ok(HasilPenyusun::BatchAktif);
    }

    // Q4 pilih eligible (WB8, AC-25).
    #[derive(sqlx::FromRow, Clone)]
    struct EligibleRow {
        request_id: Uuid,
        #[allow(dead_code)]
        completed_at: DateTime<Utc>,
        requested_at: DateTime<Utc>,
        jumlah_eligible: i64,
    }

    let eligible: Vec<EligibleRow> = sqlx::query_as(
        "SELECT r.request_id, r.completed_at, l.requested_at, count(*) OVER () AS jumlah_eligible
           FROM request r
           JOIN transaction_log l ON l.kode_transaksi = r.final_kode_transaksi
          WHERE r.provider_id = $1
            AND r.status = 'done'
            AND r.sumber_hasil = 'provider'
            AND r.input_payload IS NOT NULL
            AND l.nik IS NOT NULL
            AND NOT r.data_balikan_tersusun
          ORDER BY r.completed_at, r.request_id
          LIMIT $2",
    )
    .bind(provider_id)
    .bind(konfigurasi.ukuran_batch_maks as i64)
    .fetch_all(&mut *tx)
    .await
    .map_err(db)?;

    // Q5 dilewati (AC-28, QG NIK NULL).
    let dilewati: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT r.request_id, CASE WHEN r.input_payload IS NULL THEN 'input_payload_kosong'::text
                                     WHEN l.nik IS NULL THEN 'nik_kosong'::text END AS alasan
           FROM request r
           JOIN transaction_log l ON l.kode_transaksi = r.final_kode_transaksi
          WHERE r.provider_id = $1
            AND r.status = 'done'
            AND r.sumber_hasil = 'provider'
            AND (r.input_payload IS NULL OR l.nik IS NULL)
            AND NOT r.data_balikan_tersusun
          ORDER BY r.completed_at, r.request_id
          LIMIT 100",
    )
    .bind(provider_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(db)?;

    for (request_id, alasan) in dilewati {
        tracing::warn!(%request_id, %alasan, "data balikan: request eligible dilewati");
    }

    if eligible.is_empty() {
        // Set gauge sebelum rollback.
        let tertunda = hitung_tertunda(&mut tx, provider_id).await?;

        tx.rollback().await.map_err(db)?;
        metrics::gauge!("poolrouter_data_balikan_tertunda").set(tertunda as f64);
        return Ok(HasilPenyusun::TidakAdaPemicu);
    }

    // Q6 periode terakhir (untuk evaluasi_pemicu). WB18: ambil max(disusun_at) dari batch periode terakhir.
    // max() selalu mengembalikan satu baris, NULL bila belum ada batch periode: dekode sebagai Option lewat
    // fetch_one. fetch_optional dengan tipe DateTime gagal di batch pertama (NULL tidak bisa didekode).
    let periode_terakhir: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT max(disusun_at) FROM data_balikan_batch WHERE provider_id = $1 AND pemicu = 'periode'"
    )
    .bind(provider_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;

    // Q9 tertunda (AC-28 gauge) — hitung sebelum keputusan untuk consistency.
    let tertunda = hitung_tertunda(&mut tx, provider_id).await?;

    let jumlah_eligible = eligible[0].jumlah_eligible as u32;
    let completed_at_tertua = eligible[0].completed_at;

    // Evaluasi pemicu sesuai spec (c) langkah 6.
    let hasil_pemicu = crate::data_balikan::evaluasi_pemicu(
        jumlah_eligible,
        Some(completed_at_tertua),
        sekarang,
        &konfigurasi,
        periode_terakhir,
    );

    use crate::data_balikan::PemicuBatch;
    // Spec (c) langkah 6: evaluasi hasil, set pemicu & jadwal_kirim_at per WB18.
    let (pemicu, jadwal_kirim_at) = match hasil_pemicu {
        PemicuBatch::Tidak => {
            tx.rollback().await.map_err(db)?;
            metrics::gauge!("poolrouter_data_balikan_tertunda").set(tertunda as f64);
            return Ok(HasilPenyusun::TidakAdaPemicu);
        }
        PemicuBatch::JumlahN => ("jumlah_n", sekarang),
        PemicuBatch::Periode(batas) => ("periode", batas),
    };

    // Q7 INSERT batch (WB18).
    let batch_id = Uuid::now_v7();
    let periode_mulai_at = eligible
        .iter()
        .map(|r| r.requested_at)
        .min()
        .unwrap_or(sekarang);
    let periode_selesai_at = eligible
        .iter()
        .map(|r| r.requested_at)
        .max()
        .unwrap_or(sekarang);
    let jumlah_item = eligible.len() as i32;

    sqlx::query(
        "INSERT INTO data_balikan_batch
                (batch_id, provider_id, pemicu, periode_mulai_at, periode_selesai_at, jumlah_item,
                 status, jadwal_kirim_at, attempt_count, format_version, disusun_at, config_ref_version)
         VALUES ($1, $2, $3, $4, $5, $6, 'disusun', $7, 0, $8, $9, $10)"
    )
    .bind(batch_id)
    .bind(provider_id)
    .bind(pemicu)
    .bind(periode_mulai_at)
    .bind(periode_selesai_at)
    .bind(jumlah_item)
    .bind(jadwal_kirim_at)
    .bind(&konfigurasi.format_version)
    .bind(sekarang)
    .bind(config_ref_version)
    .execute(&mut *tx)
    .await
    .map_err(db)?;

    // Q8 INSERT item (WB9).
    let request_ids: Vec<Uuid> = eligible.iter().map(|r| r.request_id).collect();
    let rows_affected = sqlx::query(
        "INSERT INTO data_balikan_item
                (batch_id, request_id, kode_transaksi, nik, field_input, field_metadata, requested_at, aktif)
         SELECT $1, r.request_id, l.kode_transaksi, l.nik, r.input_payload,
                coalesce((SELECT jsonb_object_agg(f.nama_field, r.metadata -> f.nama_field)
                            FROM endpoint_metadata_field f
                           WHERE f.downstream_endpoint_id = r.downstream_endpoint_id
                             AND f.kirim_ke_data_balikan
                             AND f.klasifikasi <> 'Rahasia'
                             AND r.metadata ? f.nama_field), '{}'::jsonb),
                l.requested_at, true
           FROM request r
           JOIN transaction_log l ON l.kode_transaksi = r.final_kode_transaksi
          WHERE r.request_id = ANY ($2)"
    )
    .bind(batch_id)
    .bind(&request_ids)
    .execute(&mut *tx)
    .await
    .map_err(db)?
    .rows_affected();

    if rows_affected != request_ids.len() as u64 {
        return Err(GalatPenyusun::ItemTidakCocok {
            dipilih: request_ids.len() as u64,
            tertulis: rows_affected,
        });
    }

    // Q8 penanda (ARC-199): request yang baru punya item aktif keluar dari indeks parsial tertunda di
    // transaksi yang sama. Jumlah baris harus sama dengan pilihan Q4; kalau tidak, transaksi di-rollback
    // (tx di-drop tanpa commit).
    let ditandai = sqlx::query(
        "UPDATE request SET data_balikan_tersusun = true
          WHERE request_id = ANY ($1) AND NOT data_balikan_tersusun",
    )
    .bind(&request_ids)
    .execute(&mut *tx)
    .await
    .map_err(db)?
    .rows_affected();

    if ditandai != request_ids.len() as u64 {
        return Err(GalatPenyusun::ItemTidakCocok {
            dipilih: request_ids.len() as u64,
            tertulis: ditandai,
        });
    }

    tx.commit().await.map_err(db)?;

    // Set gauge setelah commit.
    metrics::gauge!("poolrouter_data_balikan_tertunda").set(tertunda as f64);

    Ok(HasilPenyusun::Disusun {
        batch_id,
        jumlah_item: jumlah_item as u32,
    })
}

/// Q9: jumlah request tertunda untuk gauge `poolrouter_data_balikan_tertunda` (AC-28).
///
/// Predikat status/sumber/penanda sama persis dengan indeks parsial `request_data_balikan_tertunda_idx`
/// (migrasi 20261108000002, ARC-199), supaya planner memakai index-only scan.
async fn hitung_tertunda(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    provider_id: Uuid,
) -> Result<i64, GalatPenyusun> {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM request r
          WHERE r.provider_id = $1
            AND r.status = 'done'
            AND r.sumber_hasil = 'provider'
            AND NOT r.data_balikan_tersusun",
    )
    .bind(provider_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(db)
}

/// Untuk ARC-123 (`pengirim_handler.rs` mengimpornya). Q2 di luar transaksi; `None` = tidak ada atau tidak valid.
#[allow(dead_code)]
pub(crate) async fn baca_konfigurasi(
    pool: &PgPool,
    provider_id: Uuid,
) -> Result<Option<KonfigurasiDataBalikan>, GalatPenyusun> {
    let konfigurasi_row: Option<(i32, Option<serde_json::Value>)> = sqlx::query_as(
        "SELECT v.config_schema_version, v.dokumen -> 'data_balikan' AS blok
           FROM provider p
           JOIN provider_config_versi v ON v.provider_id = p.provider_id AND v.versi = p.config_versi_aktif
          WHERE p.provider_id = $1 AND p.aktif AND p.data_balikan_aktif"
    )
    .bind(provider_id)
    .fetch_optional(pool)
    .await
    .map_err(db)?;

    match konfigurasi_row {
        None => Ok(None),
        Some((schema_ver, blok)) => {
            let blok = match blok {
                None => return Ok(None),
                Some(b) => b,
            };
            match KonfigurasiDataBalikan::parse(schema_ver as u32, &blok) {
                Ok(k) => Ok(Some(k)),
                Err(e) => {
                    tracing::warn!(%provider_id, galat = %e, "konfigurasi tidak valid");
                    Ok(None)
                }
            }
        }
    }
}
