//! `SumberRebuild` over PostgreSQL (ARC-69, AC-5, CW10 seam a).
//!
//! The seam is fixed by CW10: the controller's PostgreSQL source is tested
//! **through** [`bangun_ulang`], not by asserting on its query results. That way
//! the test proves the whole chain — fixture rows in PostgreSQL, counters in
//! Redis — and would fail if either side drifted.
//!
//! Run by `make -C backend uji-pg`; `#[ignore]`d because the tests need
//! `POOLROUTER_TEST_DATABASE_URL_CONTROLLER`. Fixtures are inserted as the superuser
//! (`POOLROUTER_TEST_DATABASE_URL_SU`), because `pr_controller` deliberately has
//! no write grant on `client`/`request` (grant matrix, ADR-0001 section 7.2) and
//! the code under test must run with the production role.
//!
//! Every value here is synthetic: no NIK, no credential, no provider secret.

use chrono::{DateTime, Duration, TimeZone, Utc};
use poolrouter_core::kuota::pemulihan::{
    HasilRebuild, KUNCI_SIAP, SumberRebuild, baca_run_id, bangun_ulang, kunci_attempts,
    nilai_counter,
};
use poolrouter_core::kuota::sumber_pg::SumberPostgres;
use sqlx::PgPool;
use uuid::Uuid;

fn url_su() -> Option<String> {
    std::env::var("POOLROUTER_TEST_DATABASE_URL_SU").ok()
}

fn url_controller() -> String {
    std::env::var("POOLROUTER_TEST_DATABASE_URL_CONTROLLER")
        .expect("POOLROUTER_TEST_DATABASE_URL_CONTROLLER")
}

async fn pool_su() -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url_su().expect("POOLROUTER_TEST_DATABASE_URL_SU"))
        .await
        .expect("koneksi superuser")
}

async fn pool_controller() -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&url_controller())
        .await
        .expect("koneksi pr_controller")
}

/// One fixture graph: everything a pool needs plus a few log rows.
struct Fixture {
    pool_id: Uuid,
    published_at: DateTime<Utc>,
}

/// Creates a client, credential, provider, endpoint, downstream pool and one
/// active upstream pool, then `jumlah_log` transaction rows in the period.
///
/// All names carry `ARC-69` so a failed run leaves recognisable rows and the
/// cleanup below can find them.
async fn buat_fixture(su: &PgPool, jumlah_log: usize, snapshot: Option<i64>) -> Fixture {
    let pool_id = Uuid::now_v7();
    let client_id = Uuid::now_v7();
    let admin_id = Uuid::now_v7();
    let provider_id = Uuid::now_v7();
    let downstream_pool_id = Uuid::now_v7();
    let endpoint_id = Uuid::now_v7();
    let kredensial_id = Uuid::now_v7();

    let sekarang = selang_sekarang();

    sqlx::query(
        "INSERT INTO admin_user (pengguna_id, username, password_hash, bahasa_ui)
         VALUES ($1, $2, 'x', 'id')",
    )
    .bind(admin_id)
    .bind(format!("arc69-{}", admin_id.simple()))
    .execute(su)
    .await
    .expect("admin_user");

    sqlx::query("INSERT INTO client (client_id, nama) VALUES ($1, $2)")
        .bind(client_id)
        .bind(format!("arc69-{}", client_id.simple()))
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
        .bind(format!("arc69-{}", provider_id.simple()))
        .execute(su)
        .await
        .expect("provider");

    sqlx::query(
        "INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas)
         VALUES ($1, $2, 1)",
    )
    .bind(downstream_pool_id)
    .bind(format!("arc69-{}", downstream_pool_id.simple()))
    .execute(su)
    .await
    .expect("downstream_pool");

    sqlx::query(
        "INSERT INTO downstream_endpoint
                (downstream_endpoint_id, nama, provider_id, downstream_pool_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(endpoint_id)
    .bind(format!("arc69-{}", endpoint_id.simple()))
    .bind(provider_id)
    .bind(downstream_pool_id)
    .execute(su)
    .await
    .expect("downstream_endpoint");

    // One rate window, so the rebuild writes a GCRA key the scripts can use.
    let parameter = serde_json::json!({
        "rate": { "tps": { "t_mikro": 1_000_000, "burst": 5 } },
        "timeout_total_upstream_detik": 30
    });
    sqlx::query(
        "INSERT INTO upstream_pool
                (upstream_pool_id, provider_id, nama, kuota_harian,
                 parameter_dispatch, reset_kuota_jam, reset_kuota_zona)
         VALUES ($1, $2, $3, 1000, $4, '00:00', 'Asia/Jakarta')",
    )
    .bind(pool_id)
    .bind(provider_id)
    .bind(format!("arc69-{}", pool_id.simple()))
    .bind(parameter)
    .execute(su)
    .await
    .expect("upstream_pool");

    // One transaction for all log rows: the T-04 check is a deferred constraint
    // trigger, so a `request` row and its history must commit together.
    let mut tx = su.begin().await.expect("mulai transaksi fixture");
    // Log rows spread over the current 10-minute bucket's recent past.
    for i in 0..jumlah_log {
        let request_id = Uuid::now_v7();
        let requested_at = sekarang - Duration::seconds(i64::try_from(i).unwrap_or(0) * 30);
        sqlx::query(
            "INSERT INTO request
                    (request_id, client_id, credential_id, downstream_endpoint_id,
                     downstream_pool_id, provider_id, upstream_pool_id,
                     provider_config_version, endpoint_config_version, mode,
                     prioritas, prioritas_efektif, status, metadata, received_at,
                     attempt_count, ttl_expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 1, 'async', 1, 1, 'queued',
                     '{}'::jsonb, $8, 1, $8 + interval '1 hour')",
        )
        .bind(request_id)
        .bind(client_id)
        .bind(kredensial_id)
        .bind(endpoint_id)
        .bind(downstream_pool_id)
        .bind(provider_id)
        .bind(pool_id)
        .bind(requested_at)
        .execute(&mut *tx)
        .await
        .expect("request");
        // A request is created `queued` (T-05) and moves to `dispatched`; the
        // rebuild counts `queued` as `antre` and `dispatched` as holding a slot,
        // so the fixture goes through both states explicitly.
        catat_status(&mut tx, request_id, "queued").await;
        sqlx::query(
            "UPDATE request SET status = 'dispatched', dispatched_at = $2 WHERE request_id = $1",
        )
        .bind(request_id)
        .bind(requested_at)
        .execute(&mut *tx)
        .await
        .expect("dispatch request");
        catat_status(&mut tx, request_id, "dispatched").await;

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

    if let Some(jumlah) = snapshot {
        // Same arithmetic the job uses, so the row is the one the rebuild reads.
        let periode = periode_untuk_fixture(sekarang);
        sqlx::query(
            "INSERT INTO upstream_attempt_daily
                    (upstream_pool_id, periode_mulai, periode_selesai, tanggal, jumlah, snapshot_at)
             VALUES ($1, $2, $3, $4, $5, now())",
        )
        .bind(pool_id)
        .bind(periode.mulai)
        .bind(periode.selesai)
        .bind(periode.tanggal_lokal)
        .bind(jumlah)
        .execute(su)
        .await
        .expect("upstream_attempt_daily");
    }

    Fixture {
        pool_id,
        published_at: sekarang,
    }
}

/// Now, truncated to the second, inside the fixture's period.
fn selang_sekarang() -> DateTime<Utc> {
    let now = Utc::now();
    Utc.with_ymd_and_hms(
        now.year(),
        now.month(),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
    )
    .unwrap()
}

use chrono::{Datelike, NaiveTime, Timelike};
use poolrouter_core::kuota::periode::{AturanReset, Periode, ZonaTetap, periode_untuk};

/// The period the fixture's reset rule produces for `sekarang`, exactly as the
/// controller computes it (Asia/Jakarta, midnight reset, no scheduled change).
fn periode_untuk_fixture(sekarang: DateTime<Utc>) -> Periode {
    let jakarta = ZonaTetap::dari_detik(7 * 3600).expect("offset Jakarta");
    let riwayat = vec![AturanReset::baru(
        sekarang - Duration::days(1),
        NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        jakarta,
    )];
    periode_untuk(sekarang, &riwayat).expect("periode fixture")
}

#[tokio::test]
#[ignore]
async fn rebuild_memakai_angka_terbesar_antara_snapshot_dan_log() {
    let Some(_) = url_su() else {
        eprintln!("POOLROUTER_TEST_DATABASE_URL_SU tidak diset; uji dilewati");
        return;
    };
    let su = pool_su().await;
    let controller = pool_controller().await;

    // Snapshot says 3, the log proves 7: the rebuild must take 7 (M-02: the
    // conservative number), which is what `nilai_counter` documents.
    let fx = buat_fixture(&su, 7, Some(3)).await;

    let redis = redis_uji().await;
    let mut koneksi = redis
        .get_multiplexed_async_connection()
        .await
        .expect("redis");

    let run_id = baca_run_id(&mut koneksi).await.expect("run_id");
    // Force a rebuild: clear the readiness marker.
    redis::cmd("DEL")
        .arg(KUNCI_SIAP)
        .query_async::<()>(&mut koneksi)
        .await
        .expect("hapus pr:siap");

    let sumber = SumberPostgres::baru(controller.clone());
    let hasil = bangun_ulang(&mut koneksi, &run_id, &sumber, fx.published_at)
        .await
        .expect("rebuild");
    let HasilRebuild::Dibangun(ringkasan) = hasil else {
        panic!("rebuild harus dikerjakan, dapat {hasil:?}");
    };

    let nilai = ringkasan
        .attempts
        .iter()
        .find(|(p, _)| *p == fx.pool_id.to_string())
        .map(|(_, v)| *v)
        .expect("pool fixture ada di ringkasan");
    assert_eq!(
        nilai,
        nilai_counter(3, 7),
        "rebuild memakai max(snapshot, log)"
    );

    // The counter landed under the key the scripts will read.
    let keadaan = sumber
        .keadaan(fx.published_at)
        .await
        .expect("keadaan dari PostgreSQL");
    let pool = keadaan
        .iter()
        .find(|k| k.pool_id == fx.pool_id.to_string())
        .expect("pool fixture");
    let di_redis: Option<i64> = redis::cmd("GET")
        .arg(kunci_attempts(&fx.pool_id.to_string(), pool.periode_mulai))
        .query_async(&mut koneksi)
        .await
        .expect("GET attempts");
    assert_eq!(di_redis, Some(nilai), "counter Redis = hasil rebuild");

    bersihkan(&su, fx.pool_id).await;
}

#[tokio::test]
#[ignore]
async fn keadaan_dari_postgresql_cocok_dengan_fixture() {
    let Some(_) = url_su() else {
        eprintln!("POOLROUTER_TEST_DATABASE_URL_SU tidak diset; uji dilewati");
        return;
    };
    let su = pool_su().await;
    let controller = pool_controller().await;
    let fx = buat_fixture(&su, 5, Some(2)).await;

    let sumber = SumberPostgres::baru(controller);
    let keadaan = sumber.keadaan(fx.published_at).await.expect("keadaan");
    let pool = keadaan
        .iter()
        .find(|k| k.pool_id == fx.pool_id.to_string())
        .expect("pool fixture");

    assert_eq!(
        pool.snapshot_attempts, 2,
        "snapshot dari upstream_attempt_daily"
    );
    assert_eq!(pool.log_periode, 5, "COUNT log periode berjalan");
    assert_eq!(pool.antre, 5, "lima request dispatched masuk antre");
    assert_eq!(pool.slot.len(), 5, "lima request dispatched memegang slot");
    assert_eq!(pool.jendela_rate.len(), 1, "satu jendela rate fixture");
    assert_eq!(pool.jendela_rate[0].jenis, "tps");
    assert!(pool.ttl_slot_mikro > 0);

    bersihkan(&su, fx.pool_id).await;
}

/// The status row the T-04 trigger demands for a non-null `request.status`.
async fn catat_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    request_id: Uuid,
    status: &str,
) {
    // `seq` 1 is the creation row (`status_dari` NULL); later rows chain from
    // the previous status, matching the T-04 rule the trigger enforces.
    let seq: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM request_status_history WHERE request_id = $1",
    )
    .bind(request_id)
    .fetch_one(&mut **tx)
    .await
    .expect("seq berikutnya");
    let dari: Option<String> = if seq == 1 {
        None
    } else {
        Some("queued".to_owned())
    };
    sqlx::query(
        "INSERT INTO request_status_history
                (request_id, seq, status_dari, status_ke, aktor, terjadi_at)
         VALUES ($1, $2, $3, $4, 'controller', now())",
    )
    .bind(request_id)
    .bind(seq)
    .bind(dari)
    .bind(status)
    .execute(&mut **tx)
    .await
    .expect("request_status_history");
}

async fn redis_uji() -> redis::Client {
    let url = std::env::var("POOLROUTER_TEST_REDIS").expect("POOLROUTER_TEST_REDIS belum diset");
    redis::Client::open(url).expect("URL Redis tidak sah")
}

/// Removes the fixture graph. `transaction_log` is append-only by trigger, so
/// the rows go as the superuser with the trigger bypassed is not possible; the
/// helper truncates the pool instead, which the trigger does not guard.
async fn bersihkan(su: &PgPool, pool_id: Uuid) {
    let nama = format!("arc69-{}", pool_id.simple());
    for sql in [
        "DELETE FROM request WHERE upstream_pool_id = $1",
        "DELETE FROM upstream_attempt_daily WHERE upstream_pool_id = $1",
        "DELETE FROM upstream_pool WHERE upstream_pool_id = $1",
    ] {
        let _ = sqlx::query(sql).bind(pool_id).execute(su).await;
    }
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
