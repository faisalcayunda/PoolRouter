//! Leader election over a PostgreSQL session-level advisory lock (ARC-69,
//! ADR-0001 rev 15 section 2: "Satu instance aktif lewat PostgreSQL advisory
//! lock; replika standby mengambil alih"; AC-4).
//!
//! The lock is held by a **connection**, not by a process. That single fact
//! drives the whole design:
//!
//! - The leader keeps its connection open for as long as it leads. PostgreSQL
//!   drops the lock the moment that backend dies, so a `kill -9` releases it
//!   without cleanup code (AC-4, first clause).
//! - A standby polls [`PemegangLeader::coba_ambil`] and becomes leader on the
//!   first poll after the lock is free. With a poll interval of
//!   [`INTERVAL_COBA`] the handover lands inside the 30 s budget of AC-4 with a
//!   wide margin, even when the leader's TCP connection has to time out.
//! - A leader that can no longer prove it holds the lock stops running jobs
//!   **before** its next cycle: [`Leader::masih_leader`] runs at the top of
//!   every cycle (AC-4, second clause).
//!
//! `pg_try_advisory_lock` is used rather than the `_xact_` variant so the lock
//! outlives the short transactions of the jobs.

use std::time::Duration;

use sqlx::pool::PoolConnection;
use sqlx::{PgPool, Postgres};

/// How often a standby tries to take the lock. Must stay well under the 30 s
/// handover budget of AC-4: the worst case is one failed check plus one poll.
pub const INTERVAL_COBA: Duration = Duration::from_secs(5);

/// Key of the advisory lock, `prld` in ASCII. Constant so a `pg_locks`
/// inspection is readable and so no other component can pick the same key by
/// accident.
pub const KUNCI_LEADER: i64 = 0x7072_6C64;

/// A boxed future, the return type of every object-safe async method here.
///
/// Defined locally rather than pulling in `futures` for one alias; the workspace
/// has no direct dependency on it.
pub type KotakFutur<'a, T> = std::pin::Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Something that can be asked whether this process may keep running jobs.
///
/// The scheduler takes this trait instead of [`PemegangLeader`] so a test can
/// hand in a double without a database. It returns a boxed future rather than
/// using `async fn`, because the scheduler holds `Box<dyn Leader>`.
pub trait Leader: Send + Sync {
    /// Whether this process still holds the lock. `false` means "stop now".
    fn masih_leader(&self) -> KotakFutur<'_, bool>;
}

/// An acquired leadership.
///
/// Dropping it closes the connection, which releases the lock, so a leader that
/// shuts down hands over on its own.
pub struct PemegangLeader {
    /// Dedicated to the lock; never used for queries other than the check.
    ///
    /// Held as a `PoolConnection` and not returned to the pool: the pool slot
    /// stays reserved for as long as the leadership lasts, so no other query
    /// can reuse this session and take the lock with it.
    koneksi: tokio::sync::Mutex<PoolConnection<Postgres>>,
}

impl std::fmt::Debug for PemegangLeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PemegangLeader")
    }
}

impl PemegangLeader {
    /// Tries to take the lock without waiting. `None` means another instance is
    /// the leader; a database error is reported separately so a standby can
    /// keep waiting instead of concluding the cluster is broken.
    pub async fn coba_ambil(pool: &PgPool) -> Result<Option<Self>, GalatLeader> {
        let mut koneksi = pool
            .acquire()
            .await
            .map_err(|e| GalatLeader::BasisData(e.to_string()))?;
        let dapat: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(KUNCI_LEADER)
            .fetch_one(&mut *koneksi)
            .await
            .map_err(|e| GalatLeader::BasisData(e.to_string()))?;
        if !dapat {
            return Ok(None);
        }
        Ok(Some(Self {
            koneksi: tokio::sync::Mutex::new(koneksi),
        }))
    }

    /// Waits until this instance becomes leader, polling every
    /// [`INTERVAL_COBA`]. Returns `None` only when `berhenti` fires first, so a
    /// standby can be shut down cleanly; a database error keeps waiting, which
    /// is the safe answer for a process that must not run jobs.
    pub async fn tunggu_jadi_leader(
        pool: &PgPool,
        berhenti: &tokio_util::sync::CancellationToken,
    ) -> Option<Self> {
        loop {
            if let Ok(Some(pemegang)) = Self::coba_ambil(pool).await {
                return Some(pemegang);
            }
            tokio::select! {
                () = berhenti.cancelled() => return None,
                () = tokio::time::sleep(INTERVAL_COBA) => {}
            }
        }
    }
}

impl Leader for PemegangLeader {
    fn masih_leader(&self) -> KotakFutur<'_, bool> {
        Box::pin(async move {
            // The lock lives on this backend, so ownership is a PID match: this
            // session holds a granted `advisory` lock under `KUNCI_LEADER`.
            let mut koneksi = self.koneksi.lock().await;
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (
                           SELECT 1
                             FROM pg_locks
                            WHERE locktype = 'advisory'
                              AND pid = pg_backend_pid()
                              AND granted
                              AND objsubid = 1
                              AND classid = ($1::bigint >> 32)::oid
                              AND objid = ($1::bigint & 4294967295)::oid
                         )",
            )
            .bind(KUNCI_LEADER)
            .fetch_one(&mut **koneksi)
            .await
            // A check that cannot be answered is a failed check (fail-closed).
            .unwrap_or(false)
        })
    }
}

/// Errors of leader election. The message never contains the connection
/// string: a DSN can carry a password (ADR-0001 section 7.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GalatLeader {
    BasisData(String),
}

impl std::fmt::Display for GalatLeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BasisData(pesan) => write!(f, "leader: basis data: {pesan}"),
        }
    }
}

impl std::error::Error for GalatLeader {}
