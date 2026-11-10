//! The `upstream_attempt_daily` snapshot job (ARC-69, AC-6, CW10 seam c).
//!
//! CW10 fixes the seam as "the snapshot job with the clock as a parameter", and
//! the two properties AC-6 asks for are exactly the ones an increment-based
//! implementation would break:
//!
//! - **Idempotent:** running the job twice at the same instant leaves one row
//!   with the same `jumlah`.
//! - **Final value = `COUNT` of the log:** once the period closed, `jumlah`
//!   equals the number of `transaction_log` rows in that period.
//!
//! Run by `make -C backend uji-pg`; fixtures go in as the superuser because
//! `pr_controller` cannot write `request` (grant matrix), while the job itself
//! runs on the `pr_controller` pool it uses in production.

use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};
use poolrouter_controller::snapshot::jalankan;
use poolrouter_core::kuota::periode::{AturanReset, Periode, ZonaTetap, periode_untuk};
use sqlx::PgPool;
use uuid::Uuid;

fn url_su() -> Option<String> {
    std::env::var("POOLROUTER_TEST_DATABASE_URL_SU").ok()
}

async fn pool(url: &str) -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(url)
        .await
        .expect("koneksi PostgreSQL uji")
}

/// The period the job must write, computed exactly as the controller does: a
/// midnight Asia/Jakarta reset and no scheduled change.
fn periode_untuk_fixture(sekarang: DateTime<Utc>) -> Periode {
    let jakarta = ZonaTetap::dari_detik(7 * 3600).expect("offset Jakarta");
    let riwayat = vec![AturanReset::baru(
        sekarang - Duration::days(1),
        NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        jakarta,
    )];
    periode_untuk(sekarang, &riwayat).expect("periode fixture")
}

/// Jam uji yang tetap, di tengah periode: 12:00 Asia/Jakarta, tujuh jam dari
/// reset tengah malam (17:00 UTC) di kedua sisi. Baris log fixture berada pada
/// `sekarang - 60s ..= sekarang` dan snapshot ketiga jalan pada `sekarang + 30s`;
/// dengan jam dinding, uji yang jalan dekat 17:00 UTC melihat sebagian baris
/// jatuh di periode sebelumnya (`jumlah` 1 bukan 4 pada 17:00:10, ARC-253).
/// Tanggalnya di masa lalu supaya `received_at` tidak mendahului `now()` pada
/// riwayat status. `ttl_expires_at` tetap dari `now()`: `bersihkan` tidak bisa
/// menghapus `request` yang punya `transaction_log`, dan request antre yang TTL-nya
/// sudah lewat akan disapu `penyapu` milik uji lain pada basis data yang sama.
fn sekarang_tetap() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 9, 5, 0, 0).unwrap()
}

/// Minimal fixture graph; returns the pool id.
async fn fixture(su: &PgPool, jumlah_log: usize, sekarang: DateTime<Utc>) -> Uuid {
    let pool_id = Uuid::now_v7();
    let client_id = Uuid::now_v7();
    let admin_id = Uuid::now_v7();
    let provider_id = Uuid::now_v7();
    let downstream_pool_id = Uuid::now_v7();
    let endpoint_id = Uuid::now_v7();
    let kredensial_id = Uuid::now_v7();

    sqlx::query(
        "INSERT INTO admin_user (pengguna_id, username, password_hash, bahasa_ui)
         VALUES ($1, $2, 'x', 'id')",
    )
    .bind(admin_id)
    .bind(format!("arc69s-{}", admin_id.simple()))
    .execute(su)
    .await
    .expect("admin_user");
    sqlx::query("INSERT INTO client (client_id, nama) VALUES ($1, $2)")
        .bind(client_id)
        .bind(format!("arc69s-{}", client_id.simple()))
        .execute(su)
        .await
        .expect("client");
    sqlx::query(
        "INSERT INTO client_kredensial
                (credential_id, client_id, jenis, kunci_hash, dibuat_oleh)
         VALUES ($1, $2, 'api_key', $3, $4)",
    )
    .bind(kredensial_id)
    .bind(client_id)
    .bind(vec![0u8; 32])
    .bind(admin_id)
    .execute(su)
    .await
    .expect("client_kredensial");
    // `wajib_log_nik = false`: the fixtures must not carry a synthetic NIK, and
    // C-01 only demands one for providers that log it.
    sqlx::query("INSERT INTO provider (provider_id, nama, wajib_log_nik) VALUES ($1, $2, false)")
        .bind(provider_id)
        .bind(format!("arc69s-{}", provider_id.simple()))
        .execute(su)
        .await
        .expect("provider");
    sqlx::query(
        "INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas)
         VALUES ($1, $2, 1)",
    )
    .bind(downstream_pool_id)
    .bind(format!("arc69s-{}", downstream_pool_id.simple()))
    .execute(su)
    .await
    .expect("downstream_pool");
    sqlx::query(
        "INSERT INTO downstream_endpoint
                (downstream_endpoint_id, nama, provider_id, downstream_pool_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(endpoint_id)
    .bind(format!("arc69s-{}", endpoint_id.simple()))
    .bind(provider_id)
    .bind(downstream_pool_id)
    .execute(su)
    .await
    .expect("downstream_endpoint");
    sqlx::query(
        "INSERT INTO upstream_pool
                (upstream_pool_id, provider_id, nama, kuota_harian,
                 reset_kuota_jam, reset_kuota_zona)
         VALUES ($1, $2, $3, 1000, '00:00', 'Asia/Jakarta')",
    )
    .bind(pool_id)
    .bind(provider_id)
    .bind(format!("arc69s-{}", pool_id.simple()))
    .execute(su)
    .await
    .expect("upstream_pool");

    // One transaction for all log rows: the T-04 check is a deferred constraint
    // trigger, so a `request` row and its history must commit together.
    let mut tx = su.begin().await.expect("mulai transaksi fixture");
    for i in 0..jumlah_log {
        let request_id = Uuid::now_v7();
        let requested_at = sekarang - Duration::seconds(i64::try_from(i).unwrap_or(0) * 20);
        sqlx::query(
            "INSERT INTO request
                    (request_id, client_id, credential_id, downstream_endpoint_id,
                     downstream_pool_id, provider_id, upstream_pool_id,
                     provider_config_version, endpoint_config_version, mode,
                     prioritas, prioritas_efektif, status, metadata, received_at,
                     ttl_expires_at, attempt_count)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 1, 'async', 1, 1, 'queued', '{}'::jsonb, $8,
                     now() + interval '1 hour', 0)",
        )
        .bind(request_id)
        .bind(client_id)
        .bind(kredensial_id)
        .bind(endpoint_id)
        .bind(downstream_pool_id)
        .bind(provider_id)
        .bind(pool_id)
        .bind(requested_at)
        // INSERT and its history row share one transaction: the T-04 check is a
        // deferred constraint trigger and only passes at commit.
        .execute(&mut *tx)
        .await
        .expect("request");
        catat_status(&mut tx, request_id, "queued").await;
        sqlx::query(
            "INSERT INTO transaction_log
                    (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id,
                     client_id, downstream_endpoint_id, provider_config_version,
                     endpoint_config_version, requested_at, log_mode, worker_id)
             VALUES ($1, $2, 1, $3, $4, $5, $6, 1, 1, $7, 'metadata_only', 'arc69-worker')",
        )
        .bind(Uuid::now_v7())
        .bind(request_id)
        .bind(provider_id)
        .bind(pool_id)
        .bind(client_id)
        .bind(endpoint_id)
        .bind(requested_at)
        .execute(&mut *tx)
        .await
        .expect("transaction_log");
    }
    tx.commit().await.expect("commit fixture");
    pool_id
}

/// The status row the T-04 trigger demands for a non-null `request.status`.
async fn catat_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    request_id: Uuid,
    status: &str,
) {
    sqlx::query(
        "INSERT INTO request_status_history
                (request_id, seq, status_dari, status_ke, aktor, terjadi_at)
         VALUES ($1, 1, NULL, $2, 'controller', now())",
    )
    .bind(request_id)
    .bind(status)
    .execute(&mut **tx)
    .await
    .expect("request_status_history");
}

async fn jumlah_snapshot(su: &PgPool, pool_id: Uuid) -> Option<i64> {
    sqlx::query_scalar("SELECT jumlah FROM upstream_attempt_daily WHERE upstream_pool_id = $1")
        .bind(pool_id)
        .fetch_optional(su)
        .await
        .expect("baca snapshot")
}

async fn bersihkan(su: &PgPool, pool_id: Uuid) {
    let nama = format!("arc69s-{}", pool_id.simple());
    let _ = sqlx::query("DELETE FROM request WHERE upstream_pool_id = $1")
        .bind(pool_id)
        .execute(su)
        .await;
    let _ = sqlx::query("DELETE FROM upstream_attempt_daily WHERE upstream_pool_id = $1")
        .bind(pool_id)
        .execute(su)
        .await;
    let _ = sqlx::query("DELETE FROM upstream_pool WHERE upstream_pool_id = $1")
        .bind(pool_id)
        .execute(su)
        .await;
    let _ = sqlx::query("DELETE FROM downstream_endpoint WHERE nama = $1")
        .bind(&nama)
        .execute(su)
        .await;
    let _ = sqlx::query("DELETE FROM downstream_pool WHERE nama = $1")
        .bind(&nama)
        .execute(su)
        .await;
    let _ = sqlx::query("DELETE FROM client WHERE nama = $1")
        .bind(&nama)
        .execute(su)
        .await;
}

#[tokio::test]
#[ignore]
async fn snapshot_idempoten_dan_final_sama_dengan_count_log() {
    let Some(_) = url_su() else {
        eprintln!("POOLROUTER_TEST_DATABASE_URL_SU tidak diset; uji dilewati");
        return;
    };
    let su = pool(&url_su().expect("su")).await;
    let controller =
        pool(&std::env::var("POOLROUTER_TEST_DATABASE_URL_CONTROLLER").expect("url")).await;

    let sekarang = sekarang_tetap();
    let fx = fixture(&su, 4, sekarang).await;

    let pertama = jalankan(&controller, sekarang).await.expect("snapshot 1");
    assert!(pertama.pool >= 1);

    let sesudah_1 = jumlah_snapshot(&su, fx).await.expect("baris snapshot");
    assert_eq!(sesudah_1, 4, "jumlah = COUNT log periode");

    // Second run at the very same instant: an absolute upsert, so the row must
    // not change. An increment would make this 8 (AC-6, idempotent).
    jalankan(&controller, sekarang).await.expect("snapshot 2");
    let sesudah_2 = jumlah_snapshot(&su, fx).await.expect("baris snapshot");
    assert_eq!(sesudah_2, 4, "snapshot kedua tidak menggandakan jumlah");

    // A later run in the same period picks up the rows added meanwhile.
    let mut tx = su.begin().await.expect("mulai transaksi tambahan");
    for _ in 0..2 {
        let request_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO request
                    (request_id, client_id, credential_id, downstream_endpoint_id,
                     downstream_pool_id, provider_id, upstream_pool_id,
                     provider_config_version, endpoint_config_version, mode,
                     prioritas, prioritas_efektif, status, metadata, received_at,
                     ttl_expires_at, attempt_count)
             SELECT $1, client_id, credential_id, downstream_endpoint_id,
                    downstream_pool_id, provider_id, upstream_pool_id,
                    1, 1, 'async', 1, 1, 'queued', '{}'::jsonb, $2,
                    now() + interval '1 hour', 0
               FROM request WHERE upstream_pool_id = $3 LIMIT 1",
        )
        .bind(request_id)
        .bind(sekarang - Duration::seconds(5))
        .bind(fx)
        .execute(&mut *tx)
        .await
        .expect("request tambahan");
        catat_status(&mut tx, request_id, "queued").await;
        sqlx::query(
            "INSERT INTO transaction_log
                    (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id,
                     client_id, downstream_endpoint_id, provider_config_version,
                     endpoint_config_version, requested_at, log_mode, worker_id)
             SELECT $1, r.request_id, 1, r.provider_id, r.upstream_pool_id,
                    r.client_id, r.downstream_endpoint_id, 1, 1, $2,
                    'metadata_only', 'arc69-worker'
               FROM request r WHERE r.request_id = $3",
        )
        .bind(Uuid::now_v7())
        .bind(sekarang - Duration::seconds(5))
        .bind(request_id)
        .execute(&mut *tx)
        .await
        .expect("transaction_log tambahan");
    }
    tx.commit().await.expect("commit tambahan");

    jalankan(&controller, sekarang + Duration::seconds(30))
        .await
        .expect("snapshot 3");
    let sesudah_3 = jumlah_snapshot(&su, fx).await.expect("baris snapshot");
    assert_eq!(sesudah_3, 6, "snapshot berikutnya mengejar COUNT log");

    // And the row keys on the period the rebuild will look up.
    let periode = periode_untuk_fixture(sekarang);
    let kunci: DateTime<Utc> = sqlx::query_scalar(
        "SELECT periode_mulai FROM upstream_attempt_daily WHERE upstream_pool_id = $1",
    )
    .bind(fx)
    .fetch_one(&su)
    .await
    .expect("periode_mulai");
    assert_eq!(kunci, periode.mulai, "periode_mulai = periode rebuild");

    bersihkan(&su, fx).await;
}
