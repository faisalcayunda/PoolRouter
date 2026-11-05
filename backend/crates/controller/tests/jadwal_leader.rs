//! The leader-gated scheduler (ARC-69, AC-4, CW10 seam b, in-process half).
//!
//! AC-4 has two clauses and both are about *when* a job may run:
//!
//! 1. two instances may not both run jobs — exactly one holds the lock;
//! 2. an instance that has lost the lock stops before its next cycle.
//!
//! The real advisory lock is exercised in `leader_pg.rs` against a live
//! PostgreSQL. Here the lock is a stub with a switch, so a test can move it
//! between cycles without waiting for a failover, and can count job calls to
//! prove a non-leader runs nothing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use chrono::{DateTime, TimeZone, Utc};
use poolrouter_controller::jadwal::{
    HasilSiklus, INTERVAL_SIKLUS, KotakFutur, Pekerjaan, SumberLeader, jalankan_loop, satu_siklus,
};
use poolrouter_controller::leader::Leader;
use tokio_util::sync::CancellationToken;

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 5, 3, 0, 0).unwrap()
}

/// A leader whose answer can be flipped from the test.
struct LeaderSaklar {
    memegang: Arc<AtomicBool>,
}

impl Leader for LeaderSaklar {
    fn masih_leader(&self) -> KotakFutur<'_, bool> {
        let memegang = Arc::clone(&self.memegang);
        Box::pin(async move { memegang.load(Ordering::SeqCst) })
    }
}

/// Counts how often it ran and whether it was asked to fail.
struct PekerjaanHitung {
    nama: &'static str,
    jalan: Arc<AtomicUsize>,
    gagal: bool,
}

impl Pekerjaan for PekerjaanHitung {
    fn nama(&self) -> &'static str {
        self.nama
    }

    fn jalankan(&self, _sekarang: DateTime<Utc>) -> KotakFutur<'_, Result<bool, String>> {
        let jalan = Arc::clone(&self.jalan);
        let gagal = self.gagal;
        let nama = self.nama;
        Box::pin(async move {
            jalan.fetch_add(1, Ordering::SeqCst);
            if gagal {
                Err(format!("{nama} sengaja gagal"))
            } else {
                Ok(true)
            }
        })
    }
}

fn saklar(awal: bool) -> (Arc<AtomicBool>, LeaderSaklar) {
    let memegang = Arc::new(AtomicBool::new(awal));
    (
        Arc::clone(&memegang),
        LeaderSaklar {
            memegang: Arc::clone(&memegang),
        },
    )
}

#[tokio::test]
async fn leader_menjalankan_semua_job() {
    let (_saklar, leader) = saklar(true);
    let jalan = Arc::new(AtomicUsize::new(0));
    let a = PekerjaanHitung {
        nama: "a",
        jalan: Arc::clone(&jalan),
        gagal: false,
    };
    let b = PekerjaanHitung {
        nama: "b",
        jalan: Arc::clone(&jalan),
        gagal: false,
    };

    let (hasil, galat) = satu_siklus(&leader, &[&a, &b], t0()).await;

    assert_eq!(hasil, HasilSiklus::Jalan);
    assert!(galat.is_empty(), "tidak ada job yang gagal: {galat:?}");
    assert_eq!(jalan.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn bukan_leader_tidak_menjalankan_satu_job_pun() {
    let (_saklar, leader) = saklar(false);
    let jalan = Arc::new(AtomicUsize::new(0));
    let a = PekerjaanHitung {
        nama: "a",
        jalan: Arc::clone(&jalan),
        gagal: false,
    };

    let (hasil, galat) = satu_siklus(&leader, &[&a], t0()).await;

    assert_eq!(hasil, HasilSiklus::BukanLeader);
    assert!(galat.is_empty());
    // The point of AC-4's second clause: a deposed leader touches nothing.
    assert_eq!(jalan.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn job_yang_gagal_tidak_menghentikan_job_lain() {
    let (_saklar, leader) = saklar(true);
    let jalan = Arc::new(AtomicUsize::new(0));
    let gagal = PekerjaanHitung {
        nama: "gagal",
        jalan: Arc::clone(&jalan),
        gagal: true,
    };
    let sehat = PekerjaanHitung {
        nama: "sehat",
        jalan: Arc::clone(&jalan),
        gagal: false,
    };

    let (hasil, galat) = satu_siklus(&leader, &[&gagal, &sehat], t0()).await;

    assert_eq!(hasil, HasilSiklus::Jalan);
    assert_eq!(jalan.load(Ordering::SeqCst), 2, "job kedua tetap jalan");
    assert_eq!(galat.len(), 1);
    assert_eq!(galat[0].0, "gagal");
}

/// Hands out one leader, then `None` on the next call, recording the requests.
struct SumberSekali {
    tersedia: tokio::sync::Mutex<Vec<Box<dyn Leader>>>,
    diminta: Arc<AtomicUsize>,
}

impl SumberLeader for SumberSekali {
    fn ambil(&self) -> KotakFutur<'_, Option<Box<dyn Leader>>> {
        self.diminta.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { self.tersedia.lock().await.pop() })
    }
}

#[tokio::test]
async fn kehilangan_lock_menghentikan_job_sebelum_siklus_berikutnya() {
    // Cycle 1 runs as leader. Between cycle 1 and 2 the lock is lost; the loop
    // must then run no job at all, and never fall back to the old leader.
    let (saklar_lock, leader) = saklar(true);
    let jalan = Arc::new(AtomicUsize::new(0));
    let sumber = SumberSekali {
        tersedia: tokio::sync::Mutex::new(vec![Box::new(leader) as Box<dyn Leader>]),
        diminta: Arc::new(AtomicUsize::new(0)),
    };
    let pekerjaan: Vec<Box<dyn Pekerjaan>> = vec![Box::new(PekerjaanHitung {
        nama: "a",
        jalan: Arc::clone(&jalan),
        gagal: false,
    })];

    let berhenti = CancellationToken::new();
    let berhenti_uji = berhenti.clone();
    let jalan_uji = Arc::clone(&jalan);
    let saklar_uji = Arc::clone(&saklar_lock);

    // A very short interval keeps the test fast; the production interval is
    // asserted separately so this one cannot drift silently.
    let tugas = tokio::spawn(async move {
        jalankan_loop(
            sumber,
            pekerjaan,
            std::time::Duration::from_millis(5),
            berhenti,
            t0,
        )
        .await;
    });

    // Let cycle 1 run, then depose the leader.
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let setelah_siklus_1 = jalan_uji.load(Ordering::SeqCst);
    assert!(setelah_siklus_1 >= 1, "siklus pertama harus jalan");

    saklar_uji.store(false, Ordering::SeqCst);
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    let setelah_dicabut = jalan_uji.load(Ordering::SeqCst);

    // Shut down; the loop is waiting for a new leader, which never comes.
    berhenti_uji.cancel();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), tugas).await;

    assert_eq!(
        setelah_dicabut, setelah_siklus_1,
        "setelah lock hilang tidak boleh ada job yang jalan"
    );
}

#[test]
fn interval_siklus_di_bawah_anggaran_ambil_alih() {
    // AC-4 allows 30 s. The loop sleeps one interval between cycles, so the
    // interval alone must leave room for the handover itself.
    assert!(INTERVAL_SIKLUS <= std::time::Duration::from_secs(10));
}
