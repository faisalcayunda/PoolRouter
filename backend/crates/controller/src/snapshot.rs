//! Snapshot of `upstream_attempt_daily` (ARC-69, AC-6, KP-06, ADR-0001 rev 15
//! section 5).
//!
//! The table is the durable half of the daily counter: Redis holds the live
//! `attempts:{pool}:{periode}` value, and this job copies the period's count to
//! PostgreSQL so a Redis restart can rebuild it (section 5a.2). Two properties
//! matter:
//!
//! - **Idempotent.** The primary key is `(upstream_pool_id, periode_mulai)` and
//!   the write is an upsert of an absolute value, not an increment. Running the
//!   job twice for the same instant leaves the same row (AC-6). An increment
//!   would double-count as soon as a pass retried, and the rebuild reads
//!   `jumlah` as an absolute (see [`poolrouter_core::kuota::pemulihan::KeadaanPool`]).
//! - **Final value equals `COUNT` of the log.** `jumlah` comes from
//!   `transaction_log` over the same period window as DQ-05, so once the period
//!   closes the snapshot converges on the log count (AC-6, last clause). Reading
//!   Redis instead would make the snapshot depend on the very state the rebuild
//!   is meant to restore *from* PostgreSQL.
//!
//! The clock is a parameter, so tests move time forward instead of sleeping.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use poolrouter_core::kuota::sumber_pg::periode_dari_baris;

/// Result of one snapshot pass, for logging and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingkasanSnapshot {
    /// Pools written this pass.
    pub pool: usize,
}

/// Writes one row per active pool for the period that contains `sekarang`.
///
/// A pool with no log row in the period is still written with `jumlah = 0`.
/// The rebuild treats a missing snapshot as 0, so writing the 0 explicitly only
/// makes "nothing spent yet" visible to an operator.
pub async fn jalankan(
    pool: &PgPool,
    sekarang: DateTime<Utc>,
) -> Result<RingkasanSnapshot, GalatSnapshot> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| GalatSnapshot::BasisData(e.to_string()))?;

    let pools = sqlx::query(
        "SELECT upstream_pool_id,
                reset_kuota_jam, reset_kuota_zona, reset_kuota_berlaku_mulai,
                reset_kuota_jam_berikut, reset_kuota_zona_berikut
           FROM upstream_pool
          WHERE aktif
          ORDER BY upstream_pool_id",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| GalatSnapshot::BasisData(e.to_string()))?;

    let mut jumlah_pool = 0;
    for baris in pools {
        let pool_id: Uuid = baris
            .try_get("upstream_pool_id")
            .map_err(|e| GalatSnapshot::BasisData(format!("upstream_pool_id: {e}")))?;
        // Same function the rebuild uses, so the row written here is the row
        // the rebuild will find (M-02).
        let periode = periode_dari_baris(&baris, sekarang)
            .map_err(|e| GalatSnapshot::Sumber(e.to_string()))?;

        // `COUNT(kode_transaksi)`, not `COUNT(*)`: `pr_controller` has column
        // grants only — no whole-row SELECT — so `*` is refused (grant matrix,
        // ADR-0001 section 7.2).
        let jumlah: i64 = sqlx::query_scalar(
            "SELECT COUNT(kode_transaksi)
               FROM transaction_log
              WHERE upstream_pool_id = $1
                AND requested_at >= $2
                AND requested_at < $3",
        )
        .bind(pool_id)
        .bind(periode.mulai)
        .bind(periode.selesai)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| GalatSnapshot::BasisData(e.to_string()))?;

        sqlx::query(
            "INSERT INTO upstream_attempt_daily
                    (upstream_pool_id, periode_mulai, periode_selesai,
                     periode_transisi, tanggal, jumlah, snapshot_at)
             VALUES ($1, $2, $3, false, $4, $5, $6)
             ON CONFLICT (upstream_pool_id, periode_mulai) DO UPDATE
                SET periode_selesai = EXCLUDED.periode_selesai,
                    tanggal = EXCLUDED.tanggal,
                    jumlah = EXCLUDED.jumlah,
                    snapshot_at = EXCLUDED.snapshot_at",
        )
        .bind(pool_id)
        .bind(periode.mulai)
        .bind(periode.selesai)
        .bind(periode.tanggal_lokal)
        .bind(jumlah)
        .bind(sekarang)
        .execute(&mut *tx)
        .await
        .map_err(|e| GalatSnapshot::BasisData(e.to_string()))?;

        jumlah_pool += 1;
    }

    tx.commit()
        .await
        .map_err(|e| GalatSnapshot::BasisData(e.to_string()))?;
    Ok(RingkasanSnapshot { pool: jumlah_pool })
}

/// Errors of the snapshot job. Messages carry the SQL detail, never a DSN
/// (ADR-0001 section 7.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GalatSnapshot {
    BasisData(String),
    /// A pool's reset configuration could not be turned into a period.
    Sumber(String),
}

impl std::fmt::Display for GalatSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BasisData(pesan) => write!(f, "snapshot: basis data: {pesan}"),
            Self::Sumber(pesan) => write!(f, "snapshot: sumber: {pesan}"),
        }
    }
}

impl std::error::Error for GalatSnapshot {}

/// Where the snapshot reads its facts from. A trait so the scheduler test can
/// drive the job without a database; production uses [`SumberSnapshotPg`].
pub trait SumberSnapshot: Send + Sync {
    fn tulis(
        &self,
        sekarang: DateTime<Utc>,
    ) -> crate::leader::KotakFutur<'_, Result<RingkasanSnapshot, GalatSnapshot>>;
}

/// Production source: the `pr_controller` pool.
#[derive(Debug, Clone)]
pub struct SumberSnapshotPg {
    pool: PgPool,
}

impl SumberSnapshotPg {
    pub fn baru(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl SumberSnapshot for SumberSnapshotPg {
    fn tulis(
        &self,
        sekarang: DateTime<Utc>,
    ) -> crate::leader::KotakFutur<'_, Result<RingkasanSnapshot, GalatSnapshot>> {
        Box::pin(async move { jalankan(&self.pool, sekarang).await })
    }
}

/// The scheduler job around [`SumberSnapshot`].
pub struct Snapshot<S: SumberSnapshot = SumberSnapshotPg> {
    sumber: S,
}

impl<S: SumberSnapshot> Snapshot<S> {
    pub fn baru(sumber: S) -> Self {
        Self { sumber }
    }
}

impl<S: SumberSnapshot> crate::jadwal::Pekerjaan for Snapshot<S> {
    fn nama(&self) -> &'static str {
        "snapshot"
    }

    fn jalankan(
        &self,
        sekarang: DateTime<Utc>,
    ) -> crate::leader::KotakFutur<'_, Result<bool, String>> {
        Box::pin(async move {
            self.sumber
                .tulis(sekarang)
                .await
                .map(|r| {
                    if r.pool > 0 {
                        tracing::debug!(pool = r.pool, "snapshot upstream_attempt_daily ditulis");
                    }
                    true
                })
                .map_err(|e| e.to_string())
        })
    }
}
