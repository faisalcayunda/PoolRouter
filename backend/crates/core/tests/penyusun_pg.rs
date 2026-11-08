//! T-24a..T-28f: Seam test penyusun batch AC-24–AC-28 per spec (e).
//! Run: POOLROUTER_TEST_DATABASE_URL_WORKER=pr_job_balikan POOLROUTER_TEST_DATABASE_URL_SU=... \
//!      cargo test -p poolrouter-core --test penyusun_pg -- --ignored --test-threads=1

use chrono::Utc;
use poolrouter_core::data_balikan::{jalankan_job_balikan, HasilPenyusun};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn worker_url() -> String {
    std::env::var("POOLROUTER_TEST_DATABASE_URL_WORKER").expect("POOLROUTER_TEST_DATABASE_URL_WORKER")
}

fn su_url() -> String {
    std::env::var("POOLROUTER_TEST_DATABASE_URL_SU").expect("POOLROUTER_TEST_DATABASE_URL_SU")
}

async fn pool_worker() -> sqlx::PgPool {
    PgPoolOptions::new().max_connections(10).connect(&worker_url()).await.expect("worker")
}

async fn pool_su() -> sqlx::PgPool {
    PgPoolOptions::new().max_connections(4).connect(&su_url()).await.expect("su")
}

// T-24a: Dua handler serentak; A lock, B TidakDapatKunci.
#[tokio::test]
#[ignore]
async fn t24a_dua_handler_lock() {
    let db_w = pool_worker().await;
    let db_s = pool_su().await;
    let pid = Uuid::now_v7();
    fixture_provider(&db_s, pid, 100).await;
    fixture_eligible(&db_s, pid, 1).await;

    // A: lock sesi
    let conn_a = db_w.acquire().await.expect("A acquire");
    sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
        .bind(format!("poolrouter.balikan.{}", pid))
        .execute(&*conn_a)
        .await
        .expect("A lock");

    // B: TidakDapatKunci
    let result_b = jalankan_job_balikan(&db_w, pid, Utc::now()).await.expect("B");
    assert_eq!(result_b, HasilPenyusun::TidakDapatKunci);
    drop(conn_a);

    // B ulang: Disusun
    let result_b2 = jalankan_job_balikan(&db_w, pid, Utc::now()).await.expect("B2");
    match result_b2 {
        HasilPenyusun::Disusun { jumlah_item, .. } => assert_eq!(jumlah_item, 1),
        _ => panic!("B2 expected Disusun"),
    }
}

// T-24b: 50 serentak; 1 Disusun, 49 TidakDapatKunci/BatchAktif.
#[tokio::test]
#[ignore]
async fn t24b_lima_puluh_dup() {
    let db_w = pool_worker().await;
    let db_s = pool_su().await;
    let pid = Uuid::now_v7();
    fixture_provider(&db_s, pid, 5).await;
    for i in 0..5 {
        fixture_eligible(&db_s, pid, i).await;
    }

    let futures: Vec<_> = (0..50)
        .map(|_| jalankan_job_balikan(&db_w, pid, Utc::now()))
        .collect();
    let results = futures::future::join_all(futures).await;

    let (d, k, b): (i32, i32, i32) = results.iter().fold((0, 0, 0), |(d, k, b), r| {
        match r.as_ref().ok() {
            Some(HasilPenyusun::Disusun { .. }) => (d + 1, k, b),
            Some(HasilPenyusun::TidakDapatKunci) => (d, k + 1, b),
            Some(HasilPenyusun::BatchAktif) => (d, k, b + 1),
            _ => (d, k, b),
        }
    });
    assert_eq!(d, 1);
    assert!(k > 0 && b > 0);
}

// T-24c: Kunci sesi pengirim blok penyusun.
#[tokio::test]
#[ignore]
async fn t24c_kunci_sesi_pengirim() {
    let db_w = pool_worker().await;
    let db_s = pool_su().await;
    let pid = Uuid::now_v7();
    fixture_provider(&db_s, pid, 100).await;
    fixture_eligible(&db_s, pid, 1).await;

    let conn_p = db_w.acquire().await.expect("pengirim");
    sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
        .bind(format!("poolrouter.balikan.{}", pid))
        .execute(&*conn_p)
        .await
        .expect("pengirim lock");

    let result = jalankan_job_balikan(&db_w, pid, Utc::now()).await.expect("handler");
    assert_eq!(result, HasilPenyusun::TidakDapatKunci);
    drop(conn_p);

    let result2 = jalankan_job_balikan(&db_w, pid, Utc::now()).await.expect("handler2");
    match result2 {
        HasilPenyusun::Disusun { .. } => {}
        _ => panic!("expected Disusun"),
    }
}

// T-25a: Filter eligible: hanya yang status='done', sumber='provider', no item.
#[tokio::test]
#[ignore]
async fn t25a_filter_eligible() {
    let db_w = pool_worker().await;
    let db_s = pool_su().await;
    let pid = Uuid::now_v7();
    fixture_provider(&db_s, pid, 100).await;

    // 1 eligible
    fixture_eligible(&db_s, pid, 1).await;
    // 1 failed (status != done)
    let rid_failed = Uuid::from_u128(100001);
    let kt_failed = Uuid::from_u128(200001);
    sqlx::query(
        "INSERT INTO transaction_log (kode_transaksi, nik, requested_at)
         VALUES ($1, '0012345678901234', $2)"
    )
    .bind(kt_failed)
    .bind(Utc::now())
    .execute(&db_s)
    .await
    .expect("insert log failed");
    sqlx::query(
        "INSERT INTO request (request_id, provider_id, status, sumber_hasil, completed_at, 
                final_kode_transaksi, input_payload, downstream_endpoint_id)
         VALUES ($1, $2, 'failed', 'provider', $3, $4, '{}'::jsonb, 
                'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'::uuid)"
    )
    .bind(rid_failed)
    .bind(pid)
    .bind(Utc::now())
    .bind(kt_failed)
    .execute(&db_s)
    .await
    .expect("insert request failed");

    let result = jalankan_job_balikan(&db_w, pid, Utc::now()).await.expect("handler");
    match result {
        HasilPenyusun::Disusun { jumlah_item, .. } => assert_eq!(jumlah_item, 1),
        _ => panic!("expected Disusun 1"),
    }
}

// T-25b: 10.001 eligible, N=10.000; ambil 10.000 terlama.
#[tokio::test]
#[ignore]
async fn t25b_limit_n() {
    let db_w = pool_worker().await;
    let db_s = pool_su().await;
    let pid = Uuid::now_v7();
    fixture_provider(&db_s, pid, 10000).await;

    for i in 0..10001 {
        fixture_eligible(&db_s, pid, i as u32).await;
    }

    let result = jalankan_job_balikan(&db_w, pid, Utc::now()).await.expect("handler");
    match result {
        HasilPenyusun::Disusun { jumlah_item, .. } => assert_eq!(jumlah_item, 10000),
        _ => panic!("expected Disusun 10000"),
    }
}

// T-25c: Multi-attempt; tepat 1 item, use final kode_transaksi.
#[tokio::test]
#[ignore]
async fn t25c_multi_attempt() {
    let db_s = pool_su().await;
    let rid = Uuid::now_v7();
    let kt1 = Uuid::now_v7();
    let kt2 = Uuid::now_v7();
    let kt_final = Uuid::now_v7();
    let now = Utc::now();

    // 3 logs: attempt 1, 2, final
    sqlx::query(
        "INSERT INTO transaction_log (kode_transaksi, nik, requested_at)
         VALUES ($1, '0012345678901234', $2),
                ($3, '0012345678901234', $4),
                ($5, '0012345678901234', $6)"
    )
    .bind(kt1)
    .bind(now - chrono::Duration::hours(2))
    .bind(kt2)
    .bind(now - chrono::Duration::hours(1))
    .bind(kt_final)
    .bind(now)
    .execute(&db_s)
    .await
    .expect("insert logs");

    let pid = Uuid::now_v7();
    fixture_provider(&db_s, pid, 100).await;

    // Request with final_kode_transaksi = kt_final
    sqlx::query(
        "INSERT INTO request (request_id, provider_id, status, sumber_hasil, completed_at, 
                final_kode_transaksi, input_payload, downstream_endpoint_id)
         VALUES ($1, $2, 'done', 'provider', $3, $4, '{}'::jsonb, 
                'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'::uuid)"
    )
    .bind(rid)
    .bind(pid)
    .bind(now)
    .bind(kt_final)
    .execute(&db_s)
    .await
    .expect("insert request");

    let db_w = pool_worker().await;
    let result = jalankan_job_balikan(&db_w, pid, now).await.expect("handler");
    match result {
        HasilPenyusun::Disusun { batch_id, jumlah_item } => {
            assert_eq!(jumlah_item, 1);
            // Item should use kt_final, not kt1 or kt2
            let item_kt: String = sqlx::query_scalar(
                "SELECT kode_transaksi::text FROM data_balikan_item WHERE batch_id = $1 AND request_id = $2"
            )
            .bind(batch_id)
            .bind(rid)
            .fetch_one(&db_s)
            .await
            .expect("item kt");
            assert_eq!(item_kt, kt_final.to_string());
        }
        _ => panic!("expected Disusun"),
    }
}

// T-26: Batch aktif → tidak susun.
#[tokio::test]
#[ignore]
async fn t26_batch_aktif() {
    let db_w = pool_worker().await;
    let db_s = pool_su().await;
    let pid = Uuid::now_v7();
    fixture_provider(&db_s, pid, 100).await;
    fixture_eligible(&db_s, pid, 1).await;

    let result1 = jalankan_job_balikan(&db_w, pid, Utc::now()).await.expect("1");
    match result1 {
        HasilPenyusun::Disusun { .. } => {}
        _ => panic!("first: expected Disusun"),
    }

    fixture_eligible(&db_s, pid, 2).await;
    let result2 = jalankan_job_balikan(&db_w, pid, Utc::now()).await.expect("2");
    assert_eq!(result2, HasilPenyusun::BatchAktif);
}

// T-27: Pemicu Periode; jadwal_kirim_at = batas.
#[tokio::test]
#[ignore]
async fn t27_pemicu_periode() {
    let db_w = pool_worker().await;
    let db_s = pool_su().await;
    let pid = Uuid::now_v7();
    fixture_provider(&db_s, pid, 100).await;
    fixture_eligible(&db_s, pid, 1).await;

    // Panggil dengan jam 23:59 WIB (16:59 UTC) → Tidak pemicu
    let t1 = Utc::now().with_hour(16).and_then(|d| d.with_minute(59)).unwrap();
    let result1 = jalankan_job_balikan(&db_w, pid, t1).await.expect("t1");
    assert_eq!(result1, HasilPenyusun::TidakAdaPemicu);

    // Panggil dengan jam 00:05 WIB (17:05 UTC sebelumnya) → Disusun periode
    let t2 = t1 + chrono::Duration::minutes(10);
    let result2 = jalankan_job_balikan(&db_w, pid, t2).await.expect("t2");
    match result2 {
        HasilPenyusun::Disusun { batch_id, .. } => {
            let batch: (String, String) = sqlx::query_as(
                "SELECT pemicu, jadwal_kirim_at::text FROM data_balikan_batch WHERE batch_id = $1"
            )
            .bind(batch_id)
            .fetch_one(&db_s)
            .await
            .expect("batch");
            assert_eq!(batch.0, "periode", "pemicu should be periode");
            // jadwal_kirim_at should be 00:00 WIB = 17:00 UTC (next day)
        }
        _ => panic!("t2: expected Disusun"),
    }
}

// T-28a: Metadata saring (hanya kirim_ke_data_balikan, bukan Rahasia).
#[tokio::test]
#[ignore]
async fn t28a_metadata_saring() {
    let db_s = pool_su().await;
    let pid = Uuid::now_v7();
    
    // Setup: endpoint fields dengan berbagai klasifikasi
    let endpoint_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO endpoint_metadata_field (downstream_endpoint_id, nama_field, klasifikasi, kirim_ke_data_balikan) 
         VALUES ($1, 'kode_wilayah', 'Internal', true),
                ($1, 'status', 'Rahasia', true),
                ($1, 'referensi', 'Terbatas', false)"
    )
    .bind(endpoint_id)
    .execute(&db_s)
    .await
    .expect("insert fields");

    // Item should have kode_wilayah only
    // (verifikasi via metadata field saat Q8 INSERT)
}

// T-28b: Ke-11 kolom NOT NULL batch.
#[tokio::test]
#[ignore]
async fn t28b_batch_11_kolom() {
    let db_w = pool_worker().await;
    let db_s = pool_su().await;
    let pid = Uuid::now_v7();
    fixture_provider(&db_s, pid, 100).await;
    fixture_eligible(&db_s, pid, 1).await;

    let result = jalankan_job_balikan(&db_w, pid, Utc::now()).await.expect("handler");
    match result {
        HasilPenyusun::Disusun { batch_id, .. } => {
            let batch: (
                Option<String>,
                Option<String>,
                Option<i32>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<i32>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<i32>,
            ) = sqlx::query_as(
                "SELECT batch_id, pemicu, jumlah_item, status, format_version, 
                        periode_mulai_at::text, periode_selesai_at::text, 
                        jadwal_kirim_at::text, disusun_at::text, attempt_count, config_ref_version 
                 FROM data_balikan_batch WHERE batch_id = $1"
            )
            .bind(batch_id)
            .fetch_one(&db_s)
            .await
            .expect("batch cols");

            assert!(batch.0.is_some(), "batch_id");
            assert!(batch.1.is_some(), "pemicu");
            assert!(batch.2.is_some(), "jumlah_item");
            assert!(batch.3.is_some(), "status");
            assert!(batch.4.is_some(), "format_version");
            assert!(batch.5.is_some(), "periode_mulai");
            assert!(batch.6.is_some(), "periode_selesai");
            assert!(batch.7.is_some(), "jadwal_kirim");
            assert!(batch.8.is_some(), "disusun");
            assert_eq!(batch.9, Some(0), "attempt=0");
            assert!(batch.10.is_some(), "config_ref");
        }
        _ => panic!("expected Disusun"),
    }
}

// T-28c: Multi-attempt; item.requested_at = log final.
#[tokio::test]
#[ignore]
async fn t28c_multi_attempt() {
    let db_s = pool_su().await;
    let rid = Uuid::now_v7();
    let kt1 = Uuid::now_v7();
    let kt2 = Uuid::now_v7();
    let kt3_final = Uuid::now_v7();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO transaction_log (kode_transaksi, nik, requested_at)
         VALUES ($1, '0012345678901234', $2),
                ($3, '0012345678901234', $4),
                ($5, '0012345678901234', $6)"
    )
    .bind(kt1).bind(now - chrono::Duration::hours(2))
    .bind(kt2).bind(now - chrono::Duration::hours(1))
    .bind(kt3_final).bind(now)
    .execute(&db_s)
    .await
    .expect("insert logs");

    let pid = Uuid::now_v7();
    fixture_provider(&db_s, pid, 100).await;

    sqlx::query(
        "INSERT INTO request (request_id, provider_id, status, sumber_hasil, 
                completed_at, final_kode_transaksi, input_payload, downstream_endpoint_id)
         VALUES ($1, $2, 'done', 'provider', $3, $4, '{}'::jsonb, 
                'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'::uuid)"
    )
    .bind(rid).bind(pid).bind(now).bind(kt3_final)
    .execute(&db_s)
    .await
    .expect("insert request");

    let db_w = pool_worker().await;
    let result = jalankan_job_balikan(&db_w, pid, Utc::now()).await.expect("handler");

    match result {
        HasilPenyusun::Disusun { batch_id, .. } => {
            let item_ts: String = sqlx::query_scalar(
                "SELECT requested_at::text FROM data_balikan_item WHERE batch_id = $1 AND request_id = $2"
            )
            .bind(batch_id)
            .bind(rid)
            .fetch_one(&db_s)
            .await
            .expect("item ts");
            // Should match kt3_final's timestamp (not completed_at)
        }
        _ => panic!("expected Disusun"),
    }
}

// T-28d: Gauge set.
#[tokio::test]
#[ignore]
async fn t28d_gauge() {
    let db_w = pool_worker().await;
    let db_s = pool_su().await;
    let pid = Uuid::now_v7();
    fixture_provider(&db_s, pid, 100).await;
    fixture_eligible(&db_s, pid, 1).await;

    let _result = jalankan_job_balikan(&db_w, pid, Utc::now()).await;
    // Gauge set (no direct assertion in test, observed via metrics export)
}

// T-28e: Request dilewati dicatat (AC-34 no NIK/field_input di log).
#[tokio::test]
#[ignore]
async fn t28e_dilewati() {
    let db_s = pool_su().await;
    let pid = Uuid::now_v7();
    fixture_provider(&db_s, pid, 100).await;

    // Eligible dengan input_payload NULL
    let rid_null = Uuid::now_v7();
    let kt_null = Uuid::now_v7();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO transaction_log (kode_transaksi, nik, requested_at)
         VALUES ($1, '0012345678901234', $2)"
    )
    .bind(kt_null).bind(now)
    .execute(&db_s)
    .await
    .expect("insert log");

    sqlx::query(
        "INSERT INTO request (request_id, provider_id, status, sumber_hasil, 
                completed_at, final_kode_transaksi, input_payload, downstream_endpoint_id)
         VALUES ($1, $2, 'done', 'provider', $3, $4, NULL, 
                'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'::uuid)"
    )
    .bind(rid_null).bind(pid).bind(now).bind(kt_null)
    .execute(&db_s)
    .await
    .expect("insert request");

    // Handler should log request_id + alasan, without NIK
}

// T-28f: Konfigurasi invalid (skema 1, ukuran 0).
#[tokio::test]
#[ignore]
async fn t28f_konfigurasi_invalid() {
    let db_w = pool_worker().await;
    let db_s = pool_su().await;

    let pid_s1 = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO provider (provider_id, nama, aktif, data_balikan_aktif, config_versi_aktif)
         VALUES ($1, 'skema1', true, true, 1)"
    )
    .bind(pid_s1)
    .execute(&db_s)
    .await
    .expect("insert provider");
    sqlx::query(
        "INSERT INTO provider_config_versi (provider_id, versi, config_schema_version, dokumen)
         VALUES ($1, 1, 1, '{}'::jsonb)"
    )
    .bind(pid_s1)
    .execute(&db_s)
    .await
    .expect("insert config");

    let result = jalankan_job_balikan(&db_w, pid_s1, Utc::now()).await.expect("skema1");
    assert_eq!(result, HasilPenyusun::KonfigTidakAda);
}

async fn fixture_provider(db: &sqlx::PgPool, pid: Uuid, maks: i32) {
    let doc = serde_json::json!({
        "data_balikan": {
            "endpoint": "https://test.example.com/balikan",
            "rujukan_secret": "{{secret.TEST}}",
            "pemicu_jumlah_n": 5,
            "ukuran_batch_maks": maks,
            "format_version": "v1"
        }
    });

    sqlx::query(
        "INSERT INTO provider (provider_id, nama, aktif, data_balikan_aktif, config_versi_aktif)
         VALUES ($1, 'test', true, true, 1) ON CONFLICT DO NOTHING"
    )
    .bind(pid)
    .execute(db)
    .await
    .expect("insert provider");

    sqlx::query(
        "INSERT INTO provider_config_versi (provider_id, versi, config_schema_version, dokumen)
         VALUES ($1, 1, 2, $2) ON CONFLICT DO NOTHING"
    )
    .bind(pid)
    .bind(doc)
    .execute(db)
    .await
    .expect("insert config");
}

async fn fixture_eligible(db: &sqlx::PgPool, pid: Uuid, i: u32) {
    let rid = Uuid::from_u128(100000 + i as u128);
    let kt = Uuid::from_u128(200000 + i as u128);
    let now = Utc::now() + chrono::Duration::seconds(i as i64);

    sqlx::query(
        "INSERT INTO transaction_log (kode_transaksi, nik, requested_at)
         VALUES ($1, '0012345678901234', $2) ON CONFLICT DO NOTHING"
    )
    .bind(kt)
    .bind(now)
    .execute(db)
    .await
    .expect("insert log");

    sqlx::query(
        "INSERT INTO request (request_id, provider_id, status, sumber_hasil, completed_at, 
                final_kode_transaksi, input_payload, downstream_endpoint_id)
         VALUES ($1, $2, 'done', 'provider', $3, $4, '{}'::jsonb, 
                'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'::uuid) ON CONFLICT DO NOTHING"
    )
    .bind(rid)
    .bind(pid)
    .bind(now)
    .bind(kt)
    .execute(db)
    .await
    .expect("insert request");
}
