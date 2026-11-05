//! The leader-gated scheduler (ARC-69, AC-4, CW10 seam b).
//!
//! One loop runs every periodic job of the controller, and the leader check sits
//! at the top of it. That placement is the whole point of AC-4's second clause:
//! a leader that has lost the lock must stop **before** the next cycle runs, so
//! two instances cannot both snapshot or both rebuild against the same Redis.
//!
//! The jobs themselves are plain async functions; the scheduler only decides
//! *whether* and *when* to call them. Keeping the clock as a parameter lets a
//! test drive several cycles without sleeping.

use std::time::Duration;

use chrono::{DateTime, Utc};

pub use crate::leader::KotakFutur;
use crate::leader::Leader;

/// Default interval between two scheduler cycles. ADR-0001 section 5 only says
/// "berkala"; 10 s matches the sweeper/reconciler default of the spec so the
/// whole controller has one cadence (CW6).
pub const INTERVAL_SIKLUS: Duration = Duration::from_secs(10);

/// One job the controller runs while it is leader.
///
/// `jalankan` returns whether the job did work; the scheduler counts the cycles
/// but leaves the reporting to the job so each one can emit its own metric.
pub trait Pekerjaan: Send + Sync {
    fn nama(&self) -> &'static str;
    fn jalankan(&self, sekarang: DateTime<Utc>) -> KotakFutur<'_, Result<bool, String>>;
}

/// What happened in one cycle, for logging: did it run, and why not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HasilSiklus {
    /// Not leader: the cycle was skipped without touching a job.
    BukanLeader,
    /// Leader: every job ran.
    Jalan,
}

/// Runs one cycle: checks the lock, then runs each job in order.
///
/// A failing job does not stop the others — one broken query must not also cost
/// the controller its other duties — but the error is returned so the caller can
/// log it.
pub async fn satu_siklus(
    leader: &dyn Leader,
    pekerjaan: &[&dyn Pekerjaan],
    sekarang: DateTime<Utc>,
) -> (HasilSiklus, Vec<(String, String)>) {
    if !leader.masih_leader().await {
        return (HasilSiklus::BukanLeader, Vec::new());
    }
    let mut galat = Vec::new();
    for p in pekerjaan {
        if let Err(pesan) = p.jalankan(sekarang).await {
            galat.push((p.nama().to_owned(), pesan));
        }
    }
    (HasilSiklus::Jalan, galat)
}

/// Supplies a leader when the scheduler needs one.
///
/// Separate from [`Leader`] because acquiring is async and may wait: a standby
/// blocks here until the lock is free. Returning `None` means "shut down".
pub trait SumberLeader: Send + Sync {
    fn ambil(&self) -> KotakFutur<'_, Option<Box<dyn Leader>>>;
}

/// The scheduler loop: takes a leader, then runs cycles until shutdown.
///
/// A cycle that reports [`HasilSiklus::BukanLeader`] drops the handle and asks
/// [`SumberLeader`] again, which is how a deposed leader stops running jobs
/// before the next cycle (AC-4).
pub async fn jalankan_loop<S, F>(
    sumber: S,
    pekerjaan: Vec<Box<dyn Pekerjaan>>,
    interval: Duration,
    berhenti: tokio_util::sync::CancellationToken,
    sekarang: F,
) where
    S: SumberLeader,
    F: Fn() -> DateTime<Utc>,
{
    let refs: Vec<&dyn Pekerjaan> = pekerjaan.iter().map(AsRef::as_ref).collect();
    let mut leader: Option<Box<dyn Leader>> = None;

    loop {
        if leader.is_none() {
            match sumber.ambil().await {
                Some(baru) => leader = Some(baru),
                None => return,
            }
        }

        let saat_ini = sekarang();
        let (hasil, galat) = match leader.as_ref() {
            Some(l) => satu_siklus(l.as_ref(), &refs, saat_ini).await,
            None => return,
        };
        for (nama, pesan) in galat {
            tracing::error!(job = %nama, galat = %pesan, "job controller gagal");
        }
        if hasil == HasilSiklus::BukanLeader {
            tracing::warn!("controller kehilangan leader lock; berhenti menjalankan job");
            leader = None;
        }

        tokio::select! {
            () = berhenti.cancelled() => return,
            () = tokio::time::sleep(interval) => {}
        }
    }
}
