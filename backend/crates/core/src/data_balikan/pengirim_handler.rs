//! Worker handler: mengirim batch data balikan (ARC-123, T6, AC-29–AC-35).
//!
//! Batch disusun oleh [`super::penyusun`] dengan status `disusun`; handler ini
//! memilikinya lewat klaim CAS, mengirim body ke provider lewat port
//! [`PengirimDataBalikan`], lalu menulis hasilnya. Port yang sama dipakai
//! adapter tiruan (uji) dan adapter asli (T8, ARC-125).
//!
//! **Klaim (AC-29)** hanya untuk batch `disusun` atau `gagal` yang
//! `kirim_berikutnya_at <= sekarang`, memakai `UPDATE … WHERE attempt_count = n`.
//! Klaim yang menulis `status = 'dikirim'` tanpa `provider_ack_ref` lalu
//! meninggalkan proses adalah sisa crash: setelah
//! [`BATAS_KLAIM_DETIK`] detik, sisa itu dipulihkan menjadi `gagal` dengan
//! jadwal "sekarang" (AC-31) — dan tetap di bawah batas percobaan yang sama.
//!
//! **Batas percobaan (AC-32)** dihitung dari `attempt_count` yang sudah
//! tersimpan, bukan dari jumlah proses hidup. Batch yang mencapai
//! `kirim_batas_percobaan` klaim berakhir `gagal_permanen` tanpa kirim lagi, dan
//! trigger database menonaktifkan itemnya (H-01).
//!
//! **Log tanpa data pribadi (AC-34)**: yang dicatat hanya `batch_id`,
//! `provider_id`, percobaan, dan kelas hasil. NIK dan body tidak pernah menjadi
//! field log.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use futures::stream;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, pool::PoolConnection};
use uuid::Uuid;

use super::item::ItemDataBalikan;
use super::konfigurasi::KonfigurasiDataBalikan;
use super::metrik::{self, HasilBatch};
use super::pemicu::{jeda_batch, kebijakan_batch};
use super::pengirim::{BatchKeluar, HasilAck, HasilKirimBatch, PengirimDataBalikan};
use super::penyusun::{GalatPenyusun, baca_konfigurasi, db as penyusun_db};
use crate::admin::Nik;
use crate::retry::{KebijakanBackoff, SumberAcakBackoff};

/// Umur klaim `dikirim` tanpa ack ref sebelum dianggap sisa crash (AC-31).
pub const BATAS_KLAIM_DETIK: i64 = 300;

/// Batas batch yang dilayani satu job, supaya satu provider tidak mengunci
/// worker tanpa henti.
const BATCH_PER_JOB: usize = 8;

/// Kelas kegagalan yang disimpan di `error_terakhir` (AC-35).
///
/// Kolomnya hanya boleh berisi kelas, bukan pesan provider: pesan bisa memuat
/// apa pun yang dikirim pihak lain.
const KELAS_SEMENTARA: &str = "sementara";
const KELAS_DITOLAK: &str = "ditolak";
const KELAS_DITOLAK_ACK: &str = "ditolak_ack";
const KELAS_BATAS: &str = "batas_percobaan";
const KELAS_KLAIM_KEDALUWARSA: &str = "klaim_kedaluwarsa";

/// Error jenis pengirim batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GalatPengirim {
    /// Galat database (tidak stabil, bisa dicoba ulang).
    Database(String),
    /// Konfigurasi data balikan tidak ada/`data_balikan_aktif` mati.
    KonfigTidakAda,
    /// Data item tidak bisa dibaca utuh (mis. NIK rusak).
    Data(String),
}

impl std::fmt::Display for GalatPengirim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(e) => write!(f, "database: {e}"),
            Self::KonfigTidakAda => write!(f, "konfigurasi data balikan tidak ada"),
            Self::Data(e) => write!(f, "data item tidak sah: {e}"),
        }
    }
}

impl std::error::Error for GalatPengirim {}

impl From<GalatPenyusun> for GalatPengirim {
    fn from(e: GalatPenyusun) -> Self {
        match e {
            GalatPenyusun::Database(m) => Self::Database(m),
            _ => Self::KonfigTidakAda,
        }
    }
}

/// Pemetaan yang sama dengan penyusun: hanya SQLSTATE, tidak pernah pesan
/// atau DETAIL Postgres yang bisa memuat isi baris (termasuk NIK).
fn db(e: sqlx::Error) -> GalatPengirim {
    penyusun_db(e).into()
}

#[derive(sqlx::FromRow)]
struct Kandidat {
    batch_id: Uuid,
    attempt_count: i32,
    pemicu: String,
    jumlah_item: i32,
    format_version: String,
}

/// Satu putaran job: pulihkan sisa crash, akhiri batch yang habis percobaan,
/// periksa ack, lalu klaim dan kirim batch yang jatuh tempo.
pub async fn jalankan_job_pengirim<P: PengirimDataBalikan + ?Sized>(
    pool: &PgPool,
    provider_id: Uuid,
    pengirim: &Arc<P>,
    acak: &dyn SumberAcakBackoff,
    sekarang: DateTime<Utc>,
) -> Result<(), GalatPengirim> {
    let konfigurasi = baca_konfigurasi(pool, provider_id)
        .await?
        .ok_or(GalatPengirim::KonfigTidakAda)?;
    let kebijakan = kebijakan_batch(&konfigurasi);

    // AC-24/AC-27: satu pengirim per provider. Kunci sesi supaya seluruh
    // putaran (termasuk panggilan jaringan) berjalan tunggal; instance lain
    // mengakui tanpa menulis apa pun.
    let mut koneksi = pool.acquire().await.map_err(db)?;
    // Kunci sesi hidup selama koneksinya. Bila putaran panik, koneksi yang
    // dikembalikan ke pool membawa kuncinya; ditutup saat drop, kunci ikut lepas.
    koneksi.close_on_drop();
    let kunci = format!("poolrouter.balikan.{provider_id}");
    let dapat: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(hashtextextended($1, 0))")
        .bind(&kunci)
        .fetch_one(&mut *koneksi)
        .await
        .map_err(db)?;
    if !dapat {
        return Ok(());
    }

    let hasil = putaran(
        &mut koneksi,
        provider_id,
        pengirim,
        acak,
        sekarang,
        &konfigurasi,
        &kebijakan,
    )
    .await;

    let _ = sqlx::query("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
        .bind(&kunci)
        .execute(&mut *koneksi)
        .await;

    hasil
}

async fn putaran<P: PengirimDataBalikan + ?Sized>(
    koneksi: &mut PoolConnection<Postgres>,
    provider_id: Uuid,
    pengirim: &Arc<P>,
    acak: &dyn SumberAcakBackoff,
    sekarang: DateTime<Utc>,
    konfigurasi: &KonfigurasiDataBalikan,
    kebijakan: &KebijakanBackoff,
) -> Result<(), GalatPengirim> {
    pulihkan_sisa_crash(koneksi, provider_id, sekarang).await?;
    akhiri_yang_habis(koneksi, provider_id, konfigurasi.kirim_batas_percobaan).await?;
    periksa_ack(koneksi, provider_id, pengirim, sekarang).await?;

    for _ in 0..BATCH_PER_JOB {
        let kandidat: Option<Kandidat> = sqlx::query_as(
            "SELECT batch_id, attempt_count, pemicu, jumlah_item, format_version
               FROM data_balikan_batch
              WHERE provider_id = $1
                AND (status = 'disusun'
                     OR (status = 'gagal' AND kirim_berikutnya_at <= $2))
              ORDER BY jadwal_kirim_at, batch_id
              LIMIT 1",
        )
        .bind(provider_id)
        .bind(sekarang)
        .fetch_optional(&mut **koneksi)
        .await
        .map_err(db)?;
        let Some(kandidat) = kandidat else {
            break;
        };

        kirim_satu(
            koneksi,
            provider_id,
            kandidat,
            pengirim,
            acak,
            sekarang,
            konfigurasi,
            kebijakan,
        )
        .await?;
    }
    Ok(())
}

/// Sisa klaim yang ditinggal proses mati menjadi `gagal` dengan jadwal retry
/// "sekarang" (AC-31). Klaim baru tetap hanya untuk `disusun`/`gagal`, jadi
/// aturan AC-29 tidak dilanggar oleh pemulihan ini.
async fn pulihkan_sisa_crash(
    koneksi: &mut PoolConnection<Postgres>,
    provider_id: Uuid,
    sekarang: DateTime<Utc>,
) -> Result<(), GalatPengirim> {
    let batas = sekarang - Duration::seconds(BATAS_KLAIM_DETIK);
    sqlx::query(
        "UPDATE data_balikan_batch
            SET status = 'gagal', kirim_berikutnya_at = $2, error_terakhir = $3
          WHERE provider_id = $1
            AND status = 'dikirim'
            AND provider_ack_ref IS NULL
            AND dikirim_at IS NOT NULL
            AND dikirim_at <= $4",
    )
    .bind(provider_id)
    .bind(sekarang)
    .bind(KELAS_KLAIM_KEDALUWARSA)
    .bind(batas)
    .execute(&mut **koneksi)
    .await
    .map_err(db)?;
    Ok(())
}

/// AC-32: batch yang sudah memakai seluruh jatah klaim berakhir permanen tanpa
/// kirim lagi. Trigger database menonaktifkan itemnya.
async fn akhiri_yang_habis(
    koneksi: &mut PoolConnection<Postgres>,
    provider_id: Uuid,
    batas: u32,
) -> Result<(), GalatPengirim> {
    let berakhir: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE data_balikan_batch
            SET status = 'gagal_permanen', kirim_berikutnya_at = NULL, error_terakhir = $3
          WHERE provider_id = $1 AND status = 'gagal' AND attempt_count >= $2
          RETURNING batch_id",
    )
    .bind(provider_id)
    .bind(i32::try_from(batas).unwrap_or(i32::MAX))
    .bind(KELAS_BATAS)
    .fetch_all(&mut **koneksi)
    .await
    .map_err(db)?;
    for _ in berakhir {
        metrik::batch(HasilBatch::GagalPermanen);
    }
    Ok(())
}

/// Batch yang sudah terkirim dan punya referensi ack: tanyakan hasilnya.
/// `BelumAda` bukan kegagalan — provider memang boleh lambat (AC-33).
async fn periksa_ack<P: PengirimDataBalikan + ?Sized>(
    koneksi: &mut PoolConnection<Postgres>,
    provider_id: Uuid,
    pengirim: &Arc<P>,
    sekarang: DateTime<Utc>,
) -> Result<(), GalatPengirim> {
    let menunggu: Vec<(Uuid, Option<String>)> = sqlx::query_as(
        "SELECT batch_id, provider_ack_ref FROM data_balikan_batch
          WHERE provider_id = $1 AND status = 'dikirim' AND provider_ack_ref IS NOT NULL
          ORDER BY dikirim_at
          LIMIT 32",
    )
    .bind(provider_id)
    .fetch_all(&mut **koneksi)
    .await
    .map_err(db)?;

    for (batch_id, referensi) in menunggu {
        let Some(referensi) = referensi else { continue };
        match Arc::clone(pengirim).periksa_ack(batch_id, &referensi).await {
            HasilAck::Diterima { ack_ref } => {
                let berubah = sqlx::query(
                    "UPDATE data_balikan_batch
                        SET status = 'diterima', diterima_at = $2, provider_ack_ref = $3,
                            error_terakhir = NULL
                      WHERE batch_id = $1 AND status = 'dikirim'",
                )
                .bind(batch_id)
                .bind(sekarang)
                .bind(ack_ref)
                .execute(&mut **koneksi)
                .await
                .map_err(db)?;
                if berubah.rows_affected() > 0 {
                    metrik::batch(HasilBatch::Diterima);
                }
            }
            HasilAck::Ditolak { .. } => {
                let berubah = sqlx::query(
                    "UPDATE data_balikan_batch
                        SET status = 'gagal_permanen', kirim_berikutnya_at = NULL,
                            error_terakhir = $2
                      WHERE batch_id = $1 AND status = 'dikirim'",
                )
                .bind(batch_id)
                .bind(KELAS_DITOLAK_ACK)
                .execute(&mut **koneksi)
                .await
                .map_err(db)?;
                if berubah.rows_affected() > 0 {
                    metrik::batch(HasilBatch::GagalPermanen);
                }
            }
            HasilAck::BelumAda => {}
        }
    }
    Ok(())
}

/// Klaim satu batch (CAS), kirim, lalu tulis hasilnya (AC-29–AC-34).
#[allow(clippy::too_many_arguments)]
async fn kirim_satu<P: PengirimDataBalikan + ?Sized>(
    koneksi: &mut PoolConnection<Postgres>,
    provider_id: Uuid,
    kandidat: Kandidat,
    pengirim: &Arc<P>,
    acak: &dyn SumberAcakBackoff,
    sekarang: DateTime<Utc>,
    konfigurasi: &KonfigurasiDataBalikan,
    kebijakan: &KebijakanBackoff,
) -> Result<(), GalatPengirim> {
    #[derive(sqlx::FromRow)]
    struct BarisItem {
        request_id: Uuid,
        nik: String,
        field_input: serde_json::Value,
        field_metadata: serde_json::Value,
    }

    let items: Vec<BarisItem> = sqlx::query_as(
        "SELECT request_id, nik, field_input, field_metadata
           FROM data_balikan_item WHERE batch_id = $1 ORDER BY request_id",
    )
    .bind(kandidat.batch_id)
    .fetch_all(&mut **koneksi)
    .await
    .map_err(db)?;

    // AC-19: NIK hanya hidup di dalam `ItemDataBalikan`, yang `Debug`/`Display`
    // cetaknya `***`.
    let mut item_obyek = Vec::with_capacity(items.len());
    for b in items {
        let nik = Nik::parse(&b.nik).map_err(|e| GalatPengirim::Data(e.to_string()))?;
        let item = ItemDataBalikan::baru(b.request_id, nik, b.field_input);
        item_obyek.push(serde_json::json!({
            "nik": item.nik().expose(),
            "field_input": item.field_input().expose(),
            "field_metadata": b.field_metadata,
        }));
    }
    let body = serde_json::to_vec(&serde_json::json!({ "item": item_obyek }))
        .map_err(|e| GalatPengirim::Data(e.to_string()))?;
    let payload_hash = format!("{:x}", Sha256::digest(&body));

    // Klaim: hanya `disusun` atau `gagal` jatuh tempo, dan hanya bila
    // `attempt_count` belum bergerak sejak kandidat dibaca.
    let klaim: Option<i32> = sqlx::query_scalar(
        "UPDATE data_balikan_batch
            SET status = 'dikirim', attempt_count = attempt_count + 1, dikirim_at = $3,
                kirim_berikutnya_at = NULL, payload_hash = $4,
                provider_ack_ref = NULL, error_terakhir = NULL
          WHERE batch_id = $1 AND attempt_count = $2
            AND (status = 'disusun'
                 OR (status = 'gagal' AND kirim_berikutnya_at <= $3))
          RETURNING attempt_count",
    )
    .bind(kandidat.batch_id)
    .bind(kandidat.attempt_count)
    .bind(sekarang)
    .bind(&payload_hash)
    .fetch_optional(&mut **koneksi)
    .await
    .map_err(db)?;
    let Some(percobaan) = klaim else {
        // Kalah balapan dengan putaran lain; tidak ada yang ditulis.
        return Ok(());
    };

    let batch = BatchKeluar {
        batch_id: kandidat.batch_id,
        provider_id,
        pemicu: kandidat.pemicu,
        jumlah_item: u32::try_from(kandidat.jumlah_item).unwrap_or(0),
        format_version: kandidat.format_version,
    };
    let hasil = Arc::clone(pengirim)
        .kirim(
            batch,
            Box::new(stream::iter(vec![bytes::Bytes::from(body)])),
        )
        .await;

    // AC-30: setiap hasil dipetakan ke satu status. `GagalSementara` yang
    // melewati batas percobaan langsung menjadi permanen (AC-32).
    let label = hasil_sebagai_label(&hasil);
    match hasil {
        HasilKirimBatch::Diterima { ack_ref } => {
            sqlx::query(
                "UPDATE data_balikan_batch
                    SET status = 'diterima', diterima_at = $2, provider_ack_ref = $3,
                        error_terakhir = NULL
                  WHERE batch_id = $1 AND status = 'dikirim'",
            )
            .bind(kandidat.batch_id)
            .bind(sekarang)
            .bind(ack_ref)
            .execute(&mut **koneksi)
            .await
            .map_err(db)?;
            metrik::batch(HasilBatch::Diterima);
        }
        HasilKirimBatch::Dikirim { r#ref } => {
            sqlx::query(
                "UPDATE data_balikan_batch
                    SET provider_ack_ref = $2, dikirim_at = $3
                  WHERE batch_id = $1 AND status = 'dikirim'",
            )
            .bind(kandidat.batch_id)
            .bind(r#ref)
            .bind(sekarang)
            .execute(&mut **koneksi)
            .await
            .map_err(db)?;
        }
        HasilKirimBatch::GagalSementara { .. } => {
            if percobaan >= i32::try_from(konfigurasi.kirim_batas_percobaan).unwrap_or(i32::MAX) {
                sqlx::query(
                    "UPDATE data_balikan_batch
                        SET status = 'gagal_permanen', kirim_berikutnya_at = NULL,
                            error_terakhir = $2
                      WHERE batch_id = $1 AND status = 'dikirim'",
                )
                .bind(kandidat.batch_id)
                .bind(KELAS_BATAS)
                .execute(&mut **koneksi)
                .await
                .map_err(db)?;
                metrik::batch(HasilBatch::GagalPermanen);
            } else {
                let jeda = jeda_batch(percobaan as u32, kebijakan, acak);
                sqlx::query(
                    "UPDATE data_balikan_batch
                        SET status = 'gagal', kirim_berikutnya_at = $2,
                            error_terakhir = $3
                      WHERE batch_id = $1 AND status = 'dikirim'",
                )
                .bind(kandidat.batch_id)
                .bind(sekarang + jeda)
                .bind(KELAS_SEMENTARA)
                .execute(&mut **koneksi)
                .await
                .map_err(db)?;
                metrik::batch(HasilBatch::Gagal);
            }
        }
        HasilKirimBatch::Ditolak { .. } => {
            sqlx::query(
                "UPDATE data_balikan_batch
                    SET status = 'gagal_permanen', kirim_berikutnya_at = NULL,
                        error_terakhir = $2
                  WHERE batch_id = $1 AND status = 'dikirim'",
            )
            .bind(kandidat.batch_id)
            .bind(KELAS_DITOLAK)
            .execute(&mut **koneksi)
            .await
            .map_err(db)?;
            metrik::batch(HasilBatch::GagalPermanen);
        }
    }

    tracing::info!(
        batch_id = %kandidat.batch_id,
        provider_id = %provider_id,
        percobaan = percobaan,
        hasil = label,
        "batch data balikan selesai"
    );
    Ok(())
}

fn hasil_sebagai_label(hasil: &HasilKirimBatch) -> &'static str {
    match hasil {
        HasilKirimBatch::Diterima { .. } => "diterima",
        HasilKirimBatch::Dikirim { .. } => "dikirim",
        HasilKirimBatch::GagalSementara { .. } => "gagal_sementara",
        HasilKirimBatch::Ditolak { .. } => "ditolak",
    }
}

#[cfg(test)]
mod uji {
    use super::*;
    #[test]
    fn kelas_error_terakhir_hanya_kelas() {
        for k in [
            KELAS_SEMENTARA,
            KELAS_DITOLAK,
            KELAS_DITOLAK_ACK,
            KELAS_BATAS,
            KELAS_KLAIM_KEDALUWARSA,
        ] {
            assert!(k.len() <= 500);
            assert!(!k.contains(' '), "kelas tidak boleh memuat pesan: {k}");
        }
    }

    #[test]
    fn label_hasil_stabil() {
        assert_eq!(
            hasil_sebagai_label(&HasilKirimBatch::Diterima {
                ack_ref: String::new()
            }),
            "diterima"
        );
        assert_eq!(
            hasil_sebagai_label(&HasilKirimBatch::GagalSementara {
                kelas: super::super::pengirim::KelasGagal::Sementara
            }),
            "gagal_sementara"
        );
    }
}
