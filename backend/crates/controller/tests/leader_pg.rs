//! Leader election against a real PostgreSQL (ARC-69, AC-4, CW10 seam b).
//!
//! Run by `make -C backend uji-pg`; `#[ignore]`d because they need
//! `POOLROUTER_TEST_DATABASE_URL_CONTROLLER`. The lock is a mechanism of PostgreSQL itself,
//! so a stub or an in-memory database cannot prove any of it: the test needs
//! real backends whose lifetimes it can end.
//!
//! The scenario is AC-4's own: two instances race for the lock, one wins, and
//! the loser takes over within the 30 s budget once the winner's backend is
//! gone.

use std::time::{Duration, Instant};

use poolrouter_controller::leader::{INTERVAL_COBA, Leader, PemegangLeader};

/// The connection string of the test database (`pr_controller` in `uji-pg`).
fn url() -> Option<String> {
    std::env::var("POOLROUTER_TEST_DATABASE_URL_CONTROLLER").ok()
}

async fn pool(maks: u32) -> sqlx::PgPool {
    let url = url().expect("POOLROUTER_TEST_DATABASE_URL_CONTROLLER");
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(maks)
        .connect(&url)
        .await
        .expect("koneksi PostgreSQL uji")
}

#[tokio::test]
#[ignore]
async fn hanya_satu_instance_memegang_lock() {
    let pool = pool(4).await;
    let pertama = PemegangLeader::coba_ambil(&pool).await.expect("ambil lock");
    assert!(pertama.is_some(), "instance pertama harus jadi leader");

    // A second instance, on its own connection, must be refused while the
    // first still holds the lock.
    let kedua = PemegangLeader::coba_ambil(&pool).await.expect("coba lock");
    assert!(kedua.is_none(), "instance kedua tidak boleh jadi leader");

    assert!(pertama.unwrap().masih_leader().await);
}

#[tokio::test]
#[ignore]
async fn standby_mengambil_alih_setelah_leader_mati() {
    let pool = pool(4).await;
    let leader = PemegangLeader::coba_ambil(&pool)
        .await
        .expect("ambil lock")
        .expect("leader pertama");

    let standby_pool = pool.clone();
    let mulai = Instant::now();
    // The standby waits on its own pool; the leader is dropped "hard" below.
    let standby = tokio::spawn(async move {
        let berhenti = tokio_util::sync::CancellationToken::new();
        PemegangLeader::tunggu_jadi_leader(&standby_pool, &berhenti).await
    });

    // Give the standby one chance to see the lock taken, then release it the
    // way `kill -9` would: the holding connection dies without an unlock.
    tokio::time::sleep(INTERVAL_COBA + Duration::from_millis(100)).await;
    drop(leader);

    let pengganti = tokio::time::timeout(Duration::from_secs(30), standby)
        .await
        .expect("ambil alih dalam 30 s (AC-4)")
        .expect("tugas standby tidak panic")
        .expect("standby menjadi leader");

    assert!(
        mulai.elapsed() <= Duration::from_secs(30),
        "ambil alih {} s, melewati anggaran AC-4",
        mulai.elapsed().as_secs()
    );
    assert!(pengganti.masih_leader().await);
}

#[tokio::test]
#[ignore]
async fn instance_yang_kehilangan_koneksi_tidak_lagi_mengaku_leader() {
    let pool = pool(4).await;
    let leader = PemegangLeader::coba_ambil(&pool)
        .await
        .expect("ambil lock")
        .expect("leader");
    assert!(leader.masih_leader().await);

    // Kill the backend that holds the lock, the way a network partition or a
    // `pg_terminate_backend` would. The lock dies with it.
    sqlx::query(
        "SELECT pg_terminate_backend(pid)
           FROM pg_locks
          WHERE locktype = 'advisory'
            AND objsubid = 1
            AND granted",
    )
    .execute(&pool)
    .await
    .expect("terminate backend pemegang lock");

    // AC-4, second clause: the next check must answer "no", which makes the
    // scheduler stop before running another job.
    assert!(
        !leader.masih_leader().await,
        "instance yang kehilangan lock harus berhenti mengaku leader"
    );
}
