//! The rebuild job: keeps Redis's quota state in step with PostgreSQL
//! (ARC-69, AC-5, ADR-0001 rev 15 section 5a.2).
//!
//! The job is deliberately thin. Every rule about *how* a rebuild is written —
//! the `pr:siap` marker last, zero tokens while it runs, the cross-process
//! `pr:rebuild` guard that stops two leaders rebuilding at once — lives in
//! [`poolrouter_core::kuota::pemulihan`]. The job only:
//!
//! 1. asks [`pulihkan_bila_perlu`] whether the Redis it is connected to needs a
//!    rebuild and, if so, runs one with the facts from PostgreSQL;
//! 2. reconnects before every cycle, because a promoted replica keeps the old
//!    `run_id` only until the next connection and the whole detection depends on
//!    reading the *live* one.
//!
//! Doing the readiness check here, rather than once at startup, is what makes a
//! mid-life restart recover: the leader notices `pr:siap` no longer matches on
//! its next cycle and rebuilds within one [`super::jadwal::INTERVAL_SIKLUS`].

use chrono::{DateTime, Utc};

use poolrouter_core::kuota::pemulihan::{
    GalatPemulihan, RingkasanRebuild, SumberRebuild, pulihkan_bila_perlu,
};

use crate::jadwal::Pekerjaan;

/// Rebuild job over one Redis and one source of PostgreSQL facts.
///
/// Generic over the source rather than taking `Box<dyn SumberRebuild>`:
/// [`SumberRebuild`] returns `impl Future`, so it is not dyn-compatible.
pub struct Rebuild<S: SumberRebuild + Send + Sync> {
    klien: redis::Client,
    sumber: S,
}

impl<S: SumberRebuild + Send + Sync> Rebuild<S> {
    pub fn baru(klien: redis::Client, sumber: S) -> Self {
        Self { klien, sumber }
    }
}

impl<S: SumberRebuild + Send + Sync> Pekerjaan for Rebuild<S> {
    fn nama(&self) -> &'static str {
        "rebuild"
    }

    fn jalankan(
        &self,
        sekarang: DateTime<Utc>,
    ) -> crate::leader::KotakFutur<'_, Result<bool, String>> {
        Box::pin(async move {
            let mut koneksi = self
                .klien
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| format!("koneksi Redis: {e}"))?;

            let hasil: Result<Option<RingkasanRebuild>, GalatPemulihan> =
                pulihkan_bila_perlu(&mut koneksi, &self.sumber, sekarang).await;

            match hasil {
                Ok(Some(ringkasan)) => {
                    tracing::info!(
                        pool = ringkasan.pool,
                        "state Redis dibangun ulang dari PostgreSQL"
                    );
                    Ok(true)
                }
                Ok(None) => Ok(false),
                Err(e) => Err(e.to_string()),
            }
        })
    }
}
