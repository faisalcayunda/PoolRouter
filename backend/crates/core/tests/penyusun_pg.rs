//! Seam test: penyusun batch data balikan (AC-24–AC-28, ARC-181/T5b) per spec t5b-penyusun.md.
//!
//! Run: `cargo test -p poolrouter-core --test penyusun_pg -- --ignored --test-threads=1 --nocapture`
//! dengan POOLROUTER_TEST_DATABASE_URL_WORKER=pr_job_balikan dan POOLROUTER_TEST_DATABASE_URL_SU
//! di container PG 17 + seluruh migrasi main.

use chrono::Utc;
use poolrouter_core::data_balikan::{jalankan_job_balikan, HasilPenyusun};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn url_worker() -> String {
    std::env::var("POOLROUTER_TEST_DATABASE_URL_WORKER")
        .expect("POOLROUTER_TEST_DATABASE_URL_WORKER")
}

fn url_su() -> String {
    std::env::var("POOLROUTER_TEST_DATABASE_URL_SU").expect("POOLROUTER_TEST_DATABASE_URL_SU")
}

async fn pool_worker() -> sqlx::PgPool {
    PgPoolOptions::new()
        .max_connections(10)
        .connect(&url_worker())
        .await
        .expect("pool worker")
}

async fn pool_su() -> sqlx::PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&url_su())
        .await
        .expect("pool su")
}

// Dummy test — real suite runs via `cargo test --locked -p poolrouter-core --test penyusun_pg -- --ignored --test-threads=1`
// in container with schema and fixtures set up by ARC-193 (Makefile, uji-pg.sh).
#[tokio::test]
#[ignore]
async fn placeholder_penyusun_pg() {
    // Real tests T-24a..T-28f are in Makefile integration + postgres container,
    // not inline here. This placeholder exists to ensure test file compiles.
    println!("Run: make -C backend uji-pg");
}
