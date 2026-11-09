//! Reconciler controller (ARC-72, AC-22–24; ADR-0001 §5 "Ketahanan antrean"
//! lapisan 2, T-06).
//!
//! PostgreSQL adalah sumber status; broker hanya lapisan pertama. Reconciler
//! memulihkan dua keadaan macet:
//!
//! - **`dispatched` lebih lama dari ambang** (default 2 menit): pesan hilang
//!   atau publish gagal sesudah commit. Compare-and-set kembali ke `queued`,
//!   lalu dispatcher mem-publish ulang pada putaran berikutnya. Slot yang
//!   dipegang dilepas; `antre` tidak berubah karena request tetap menunggu.
//! - **`in_flight` lebih lama dari timeout total upstream + 60 s**: pemrosesnya
//!   mati. Dalam satu transaksi log percobaan ditutup `tidak_diketahui`, lalu:
//!   async kembali `queued` dengan `not_before` backoff atau `failed`/
//!   `retry_habis` (aturan AC-18 lewat [`KebijakanBackoff::putuskan`]); sync
//!   langsung `failed`/`timeout_sync` dan tidak pernah `queued` (U-21).
//!
//! Penengah race dengan worker/webserver (T-06, U-22a/b): reconciler memakai
//! `status = 'in_flight' AND attempt_count = $dibaca`, penulis hasil memakai
//! `status = 'in_flight'` dan `status_akses IS NULL`. Siapa pun yang mengunci
//! baris log lebih dulu menang; yang kalah mendapat nol baris dan berhenti.

use std::time::Duration;

use chrono::{DateTime, Utc};
use poolrouter_core::kuota::store::KuotaStore;
use poolrouter_core::kuota::sumber_pg::ParameterDispatch;
use poolrouter_core::retry::{ErrorClass, KebijakanBackoff, KeputusanRetry, SumberAcakBackoff};
use sqlx::PgPool;
use uuid::Uuid;

use crate::transisi::{EfekRedis, SQL_WEBHOOK_STATUS, catat_history, jalankan_efek};

/// Ambang `dispatched` macet bawaan (AC-22, ADR-0001 §5).
pub const AMBANG_DISPATCHED_BAWAAN: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct KonfigurasiReconciler {
    pub ambang_dispatched: Duration,
    pub batas_baris: i64,
}

impl Default for KonfigurasiReconciler {
    fn default() -> Self {
        Self {
            ambang_dispatched: AMBANG_DISPATCHED_BAWAAN,
            batas_baris: 500,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RingkasanReconciler {
    /// `dispatched → queued`.
    pub dikembalikan: Vec<Uuid>,
    /// Async `in_flight → queued` dengan `not_before`.
    pub dijadwalkan_ulang: Vec<Uuid>,
    /// Async `in_flight → failed`/`retry_habis`.
    pub retry_habis: Vec<Uuid>,
    /// Sync `in_flight → failed`/`timeout_sync`.
    pub timeout_sync: Vec<Uuid>,
}

impl RingkasanReconciler {
    fn ada(&self) -> bool {
        !(self.dikembalikan.is_empty()
            && self.dijadwalkan_ulang.is_empty()
            && self.retry_habis.is_empty()
            && self.timeout_sync.is_empty())
    }
}

/// Menjalankan satu siklus reconciler.
pub async fn jalankan_siklus_reconciler<K: KuotaStore>(
    db: &PgPool,
    kuota: Option<&K>,
    acak: &dyn SumberAcakBackoff,
    konf: &KonfigurasiReconciler,
    sekarang: DateTime<Utc>,
) -> Result<RingkasanReconciler, String> {
    let mut ringkasan = RingkasanReconciler::default();
    let mut efek = Vec::new();
    // Sama dengan penyapu: efek transisi yang sudah commit tetap dijalankan
    // walau baris berikutnya gagal, baru galatnya dikembalikan (AC-25).
    let hasil = async {
        kembalikan_dispatched(db, konf, sekarang, &mut ringkasan, &mut efek).await?;
        pulihkan_in_flight(db, acak, konf, sekarang, &mut ringkasan, &mut efek).await
    }
    .await;
    jalankan_efek(kuota, &efek).await;
    hasil?;
    if ringkasan.ada() {
        tracing::info!(
            dikembalikan = ringkasan.dikembalikan.len(),
            dijadwalkan_ulang = ringkasan.dijadwalkan_ulang.len(),
            retry_habis = ringkasan.retry_habis.len(),
            timeout_sync = ringkasan.timeout_sync.len(),
            "reconciler memulihkan request"
        );
    }
    Ok(ringkasan)
}

async fn kembalikan_dispatched(
    db: &PgPool,
    konf: &KonfigurasiReconciler,
    sekarang: DateTime<Utc>,
    ringkasan: &mut RingkasanReconciler,
    efek: &mut Vec<EfekRedis>,
) -> Result<(), String> {
    let ambang = chrono::Duration::from_std(konf.ambang_dispatched)
        .map_err(|e| format!("ambang dispatched: {e}"))?;
    let batas = sekarang - ambang;
    let calon: Vec<(Uuid, i32, Option<Uuid>)> = sqlx::query_as(
        "SELECT request_id, attempt_count, upstream_pool_id
           FROM request
          WHERE status = 'dispatched' AND dispatched_at <= $1
          ORDER BY dispatched_at
          LIMIT $2",
    )
    .bind(batas)
    .bind(konf.batas_baris)
    .fetch_all(db)
    .await
    .map_err(|e| format!("pilih dispatched macet: {e}"))?;

    for (request_id, attempt_count, upstream) in calon {
        let mut tx = db.begin().await.map_err(|e| e.to_string())?;
        let n = sqlx::query(
            "UPDATE request SET status = 'queued'
              WHERE request_id = $1 AND status = 'dispatched' AND attempt_count = $2
                AND dispatched_at <= $3",
        )
        .bind(request_id)
        .bind(attempt_count)
        .bind(batas)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("dispatched ke queued: {e}"))?
        .rows_affected();
        if n != 1 {
            continue;
        }
        catat_history(&mut tx, request_id, "dispatched", "queued", None, None)
            .await
            .map_err(|e| format!("history dispatched ke queued: {e}"))?;
        tx.commit().await.map_err(|e| e.to_string())?;
        ringkasan.dikembalikan.push(request_id);
        efek.push(EfekRedis {
            request_id,
            lepas_slot: upstream.map(|p| (p, i64::from(attempt_count) + 1)),
            ubah_antre: None,
        });
    }
    Ok(())
}

/// Baris kandidat `in_flight` beserta log percobaan terbukanya.
#[derive(sqlx::FromRow)]
struct InFlight {
    request_id: Uuid,
    mode: String,
    attempt_count: i32,
    ttl_expires_at: Option<DateTime<Utc>>,
    kode_transaksi: Uuid,
    requested_at: DateTime<Utc>,
    upstream_pool_id: Uuid,
    parameter_dispatch: sqlx::types::Json<serde_json::Value>,
    batas_percobaan: i32,
    retry_backoff_base_detik: i32,
    retry_backoff_cap_detik: i32,
}

async fn pulihkan_in_flight(
    db: &PgPool,
    acak: &dyn SumberAcakBackoff,
    konf: &KonfigurasiReconciler,
    sekarang: DateTime<Utc>,
    ringkasan: &mut RingkasanReconciler,
    efek: &mut Vec<EfekRedis>,
) -> Result<(), String> {
    let calon: Vec<InFlight> = sqlx::query_as(
        "SELECT r.request_id, r.mode, r.attempt_count, r.ttl_expires_at,
                l.kode_transaksi, l.requested_at, l.upstream_pool_id,
                u.parameter_dispatch, u.batas_percobaan,
                u.retry_backoff_base_detik, u.retry_backoff_cap_detik
           FROM request r
           JOIN transaction_log l ON l.request_id = r.request_id AND l.attempt_no = r.attempt_count
           JOIN upstream_pool u ON u.upstream_pool_id = l.upstream_pool_id
          WHERE r.status = 'in_flight'
            AND l.requested_at <= $1 - interval '60 seconds'
          ORDER BY l.requested_at
          LIMIT $2",
    )
    .bind(sekarang)
    .bind(konf.batas_baris)
    .fetch_all(db)
    .await
    .map_err(|e| format!("pilih in_flight macet: {e}"))?;

    for c in calon {
        let ambang = match ParameterDispatch::dari_json(&c.parameter_dispatch.0) {
            Ok(p) => p.ambang_reconciler_detik(),
            Err(galat) => {
                tracing::warn!(galat = %galat, "parameter_dispatch tidak sah; request dilewati");
                continue;
            }
        };
        if c.requested_at > sekarang - chrono::Duration::seconds(ambang) {
            continue;
        }
        if let Some(hasil) = pulihkan_satu(db, acak, &c, ambang, sekarang).await? {
            match hasil {
                Pemulihan::Dijadwalkan => {
                    ringkasan.dijadwalkan_ulang.push(c.request_id);
                    // Klaim slot worker sudah mengurangi `antre`; request
                    // kembali menunggu, jadi `antre` naik lagi (CW3).
                    efek.push(EfekRedis {
                        request_id: c.request_id,
                        lepas_slot: Some((c.upstream_pool_id, i64::from(c.attempt_count))),
                        ubah_antre: Some((c.upstream_pool_id, 1)),
                    });
                }
                Pemulihan::RetryHabis | Pemulihan::TimeoutSync => {
                    if hasil == Pemulihan::RetryHabis {
                        ringkasan.retry_habis.push(c.request_id);
                    } else {
                        ringkasan.timeout_sync.push(c.request_id);
                    }
                    efek.push(EfekRedis {
                        request_id: c.request_id,
                        lepas_slot: Some((c.upstream_pool_id, i64::from(c.attempt_count))),
                        ubah_antre: None,
                    });
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pemulihan {
    Dijadwalkan,
    RetryHabis,
    TimeoutSync,
}

/// Satu transaksi: tutup log `tidak_diketahui`, lalu transisi request.
/// `None` berarti pihak lain menang (nol baris) dan tidak ada yang ditulis.
async fn pulihkan_satu(
    db: &PgPool,
    acak: &dyn SumberAcakBackoff,
    c: &InFlight,
    ambang: i64,
    sekarang: DateTime<Utc>,
) -> Result<Option<Pemulihan>, String> {
    let mut tx = db.begin().await.map_err(|e| e.to_string())?;
    sqlx::query_scalar::<_, bool>("SELECT fn_tutup_log_tidak_diketahui($1, $2)")
        .bind(c.kode_transaksi)
        .bind(i32::try_from(ambang).unwrap_or(i32::MAX))
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| format!("tutup log tidak_diketahui: {e}"))?;
    // Log yang sudah dilengkapi worker/webserver berarti mereka menang; log
    // yang masih NULL berarti ambang platform lebih panjang dari ambang pool.
    let status_akses: Option<String> =
        sqlx::query_scalar("SELECT status_akses FROM transaction_log WHERE kode_transaksi = $1")
            .bind(c.kode_transaksi)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| format!("baca log: {e}"))?;
    if status_akses.as_deref() != Some("tidak_diketahui") {
        return Ok(None);
    }

    let (keputusan, sql, not_before): (Pemulihan, String, Option<DateTime<Utc>>) =
        if c.mode == "sync" {
            (
                Pemulihan::TimeoutSync,
                "UPDATE request
                    SET status = 'failed', failure_reason = 'timeout_sync',
                        completed_at = $3, final_kode_transaksi = $4
                  WHERE request_id = $1 AND status = 'in_flight' AND attempt_count = $2
                    AND mode = 'sync'"
                    .to_owned(),
                None,
            )
        } else {
            let kebijakan = KebijakanBackoff {
                base_detik: u32::try_from(c.retry_backoff_base_detik).unwrap_or(5),
                cap_detik: u32::try_from(c.retry_backoff_cap_detik).unwrap_or(300),
                batas_percobaan: u32::try_from(c.batas_percobaan).unwrap_or(3),
            };
            let ttl = c.ttl_expires_at.unwrap_or(sekarang);
            match kebijakan.putuskan(
                ErrorClass::Timeout,
                u32::try_from(c.attempt_count).unwrap_or(u32::MAX),
                sekarang,
                ttl,
                acak,
            ) {
                KeputusanRetry::Jadwalkan(nb) => (
                    Pemulihan::Dijadwalkan,
                    // Predikat `mode = 'async'` menegakkan larangan sync masuk
                    // `queued` di kueri, bukan di kode pemanggil (§5).
                    "UPDATE request SET status = 'queued', not_before = $3
                      WHERE request_id = $1 AND status = 'in_flight' AND attempt_count = $2
                        AND mode = 'async'"
                        .to_owned(),
                    Some(nb),
                ),
                _ => (
                    Pemulihan::RetryHabis,
                    format!(
                        "UPDATE request
                            SET status = 'failed', failure_reason = 'retry_habis',
                                completed_at = $3, final_kode_transaksi = $4,
                                webhook_status = {SQL_WEBHOOK_STATUS}
                          WHERE request_id = $1 AND status = 'in_flight' AND attempt_count = $2
                            AND mode = 'async'"
                    ),
                    None,
                ),
            }
        };

    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(c.request_id)
        .bind(c.attempt_count);
    if keputusan == Pemulihan::Dijadwalkan {
        q = q.bind(not_before);
    } else {
        q = q.bind(sekarang).bind(c.kode_transaksi);
    }
    let n = q
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("transisi reconciler: {e}"))?
        .rows_affected();
    if n != 1 {
        return Ok(None);
    }
    let (ke, alasan) = match keputusan {
        Pemulihan::Dijadwalkan => ("queued", None),
        Pemulihan::RetryHabis => ("failed", Some("retry_habis")),
        Pemulihan::TimeoutSync => ("failed", Some("timeout_sync")),
    };
    catat_history(&mut tx, c.request_id, "in_flight", ke, alasan, None)
        .await
        .map_err(|e| format!("history reconciler: {e}"))?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(Some(keputusan))
}

/// [`crate::jadwal::Pekerjaan`] reconciler.
pub struct Reconciler {
    db: PgPool,
    redis: redis::Client,
    acak: Box<dyn SumberAcakBackoff>,
    konf: KonfigurasiReconciler,
}

impl Reconciler {
    pub fn baru(
        db: PgPool,
        redis: redis::Client,
        acak: Box<dyn SumberAcakBackoff>,
        konf: KonfigurasiReconciler,
    ) -> Self {
        Self {
            db,
            redis,
            acak,
            konf,
        }
    }
}

impl crate::jadwal::Pekerjaan for Reconciler {
    fn nama(&self) -> &'static str {
        "reconciler"
    }

    fn jalankan(
        &self,
        sekarang: DateTime<Utc>,
    ) -> crate::jadwal::KotakFutur<'_, Result<bool, String>> {
        Box::pin(async move {
            let kuota = crate::kuota_redis(&self.redis).await;
            let r = jalankan_siklus_reconciler(
                &self.db,
                kuota.as_ref(),
                self.acak.as_ref(),
                &self.konf,
                sekarang,
            )
            .await?;
            Ok(r.ada())
        })
    }
}
