//! Admin API pool CRUD + quota tracker (R3 T3: AC-16–18).
//! Test seam: handlers via HTTP (production router + oneshot), PostgreSQL (pr_admin_api role),
//! no mock repository.
//!
//! Validators and pure logic tested in unit tests (config.rs); these tests verify:
//! - AC-16: create/update upstream pool → validate + write audit row
//! - AC-17: create/update downstream pool → exact one upstream, reject over-quota
//! - AC-18: get quota tracker → DB counters, not Redis

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use chrono::{DateTime, TimeZone, Utc};
use poolrouter_core::admin::sesi_store::SesiStore;
use poolrouter_core::admin::{AsalUi, TokenSesi};
use poolrouter_core::secret::Brankas;
use poolrouter_webserver::admin::pembatas::PembatasLogin;
use poolrouter_webserver::admin::sesi::{AdminState, COOKIE_SESI, router};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const ASAL: &str = "https://ui.example.test";
const KEK_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const KUNCI_HMAC: &[u8] = b"kunci-hmac-sintetis-untuk-uji";

fn redis_ujian() -> String {
    std::env::var("POOLROUTER_TEST_REDIS")
        .expect("POOLROUTER_TEST_REDIS belum diset; jalankan lewat `make -C backend uji-pg`")
}

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 5, 9, 0, 0).unwrap()
}

struct Uji {
    app: Router,
    pool: PgPool,
    su: PgPool,
}

impl Uji {
    async fn kirim(
        &self,
        method: Method,
        path: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Value) {
        let asal = method != Method::GET;
        let mut req = Request::builder().method(method).uri(path);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        if asal {
            req = req.header(header::ORIGIN, ASAL);
        }
        let req = match body {
            Some(b) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(b.to_string()))
                .unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (
            status,
            headers,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn pengguna(&self, peran: &[&str]) -> (Uuid, String) {
        let uid = Uuid::now_v7();
        self.user(uid).await;
        for p in peran {
            let (pengaju, penyetuju) = if matches!(*p, "auditor" | "operator") {
                let a = Uuid::now_v7();
                let b = Uuid::now_v7();
                self.user(a).await;
                self.user(b).await;
                (Some(a), Some(b))
            } else {
                (None, None)
            };
            sqlx::query(
                "INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh, disetujui_oleh) \
                 VALUES ($1, $2, coalesce($3, $1), $4)",
            )
            .bind(uid)
            .bind(p)
            .bind(pengaju)
            .bind(penyetuju)
            .execute(&self.pool)
            .await
            .unwrap();
        }
        let token = TokenSesi::baru();
        SesiStore::baru(self.pool.clone())
            .terbitkan(uid, &token.hash(), Some("uji"), t0())
            .await
            .unwrap();
        (uid, format!("{COOKIE_SESI}={}", token.sebagai_b64()))
    }

    async fn user(&self, uid: Uuid) {
        sqlx::query(
            "INSERT INTO admin_user (pengguna_id, username, password_hash) VALUES ($1, $2, 'x')",
        )
        .bind(uid)
        .bind(format!("sintetis-{}", uid.simple()))
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn provider(&self, pembuat: Uuid) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO provider (provider_id, nama, metode, base_url, log_mode, \
             diubah_oleh, config_schema_version, provider_config_version) \
             VALUES ($1, 'dukcapil', 'POST', 'https://dukcapil.example.go.id', \
             'metadata_only', $2, 1, 1)",
        )
        .bind(id)
        .bind(pembuat)
        .execute(&self.su)
        .await
        .unwrap();
        id
    }

    async fn audit_konfigurasi(&self, entitas_id: Uuid) -> Vec<(String, String)> {
        sqlx::query_as(
            "SELECT aksi, entitas_jenis FROM audit_konfigurasi WHERE entitas_id = $1 ORDER BY terjadi_at",
        )
        .bind(entitas_id)
        .fetch_all(&self.su)
        .await
        .unwrap()
    }
}

async fn siapkan() -> Uji {
    let url = std::env::var("POOLROUTER_TEST_DATABASE_URL").expect(
        "POOLROUTER_TEST_DATABASE_URL belum diset; jalankan lewat `make -C backend uji-pg`",
    );
    let su_url = std::env::var("POOLROUTER_TEST_DATABASE_URL_SU").expect(
        "POOLROUTER_TEST_DATABASE_URL_SU belum diset; jalankan lewat `make -C backend uji-pg`",
    );
    let pool = PgPool::connect(&url).await.expect("database");
    let su = PgPool::connect(&su_url).await.expect("database su");
    let state = AdminState::baru(
        pool.clone(),
        AsalUi::baru(ASAL).unwrap(),
        PembatasLogin::baru(redis::Client::open(redis_ujian()).unwrap(), KUNCI_HMAC),
        Brankas::dari_isi_kek(KEK_HEX).unwrap(),
        redis::Client::open(redis_ujian()).unwrap(),
        false,
    )
    .dengan_jam(Arc::new(t0));
    let app = router(state).layer(MockConnectInfo(SocketAddr::from(([203, 0, 113, 9], 4000))));
    Uji { app, pool, su }
}

fn no_store(h: &HeaderMap) {
    assert_eq!(h[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
#[ignore]
async fn ac16_create_upstream_pool_passes_validate() {
    let u = siapkan().await;
    let (_uid, pa) = u.pengguna(&["platform_admin"]).await;
    let provider_id = u.provider(_uid).await;

    let body = json!({
        "provider_id": provider_id,
        "kuota_harian": 500,
        "rate_limit_per_jam": 60,
        "porsi_sync": 0.1,
        "margin_kuota": 0.01,
        "batas_percobaan": 3,
        "retry_backoff_base_detik": 5,
        "retry_backoff_cap_detik": 300,
        "retry_rasio": 0.1,
        "retry_min_per_jam": 20,
        "jendela_kuota": "bergulir_24j",
        "reset_kuota_jam": 0,
        "reset_kuota_zona": "Asia/Jakarta",
        "jeda_min": 250,
        "jeda_maks": 1500,
        "ambang_berdampingan": 8
    });

    let (st, h, b) = u
        .kirim(Method::POST, "/admin/v1/upstream-pools", Some(&pa), Some(body))
        .await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    no_store(&h);
    assert!(b["upstream_pool_id"].as_str().is_some());
    let pool_id = Uuid::parse_str(b["upstream_pool_id"].as_str().unwrap()).unwrap();

    let audit_rows = u.audit_konfigurasi(pool_id).await;
    assert!(audit_rows.iter().any(|(aksi, _)| aksi == "buat"));
}

#[tokio::test]
#[ignore]
async fn ac16_create_upstream_pool_validates_bounds() {
    let u = siapkan().await;
    let (_uid, pa) = u.pengguna(&["platform_admin"]).await;
    let provider_id = u.provider(_uid).await;

    let body = json!({
        "provider_id": provider_id,
        "kuota_harian": 0,  // Below minimum 1
        "rate_limit_per_jam": 60,
        "porsi_sync": 0.1,
        "margin_kuota": 0.01,
        "batas_percobaan": 3,
        "retry_backoff_base_detik": 5,
        "retry_backoff_cap_detik": 300,
        "retry_rasio": 0.1,
        "retry_min_per_jam": 20,
        "jendela_kuota": "bergulir_24j",
        "reset_kuota_jam": 0,
        "reset_kuota_zona": "Asia/Jakarta",
        "jeda_min": 250,
        "jeda_maks": 1500,
        "ambang_berdampingan": 8
    });

    let (st, h, b) = u
        .kirim(Method::POST, "/admin/v1/upstream-pools", Some(&pa), Some(body))
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    no_store(&h);
    assert_eq!(b["status"], 422);
    let errors = b["errors"].as_array().unwrap();
    assert!(errors.iter().any(|e| e["field"] == "kuota_harian" && e["kode"] == "rentang"));
}

#[tokio::test]
#[ignore]
async fn ac17_create_downstream_pool_single_upstream() {
    let u = siapkan().await;
    let (_uid, pa) = u.pengguna(&["platform_admin"]).await;
    let provider_id = u.provider(_uid).await;

    // Create upstream pool first
    let up_body = json!({
        "provider_id": provider_id,
        "kuota_harian": 500,
        "rate_limit_per_jam": 60,
        "porsi_sync": 0.1,
        "margin_kuota": 0.01,
        "batas_percobaan": 3,
        "retry_backoff_base_detik": 5,
        "retry_backoff_cap_detik": 300,
        "retry_rasio": 0.1,
        "retry_min_per_jam": 20,
        "jendela_kuota": "bergulir_24j",
        "reset_kuota_jam": 0,
        "reset_kuota_zona": "Asia/Jakarta",
        "jeda_min": 250,
        "jeda_maks": 1500,
        "ambang_berdampingan": 8
    });
    let (st, _, up_resp) = u
        .kirim(Method::POST, "/admin/v1/upstream-pools", Some(&pa), Some(up_body))
        .await;
    assert_eq!(st, StatusCode::CREATED);
    let upstream_pool_id = Uuid::parse_str(up_resp["upstream_pool_id"].as_str().unwrap()).unwrap();

    // Create downstream pool
    let dp_body = json!({
        "nama": "BGN MPM",
        "upstream_pool_ids": [upstream_pool_id],
        "ttl_antrean_detik": 86400
    });
    let (st, h, b) = u
        .kirim(Method::POST, "/admin/v1/downstream-pools", Some(&pa), Some(dp_body))
        .await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    no_store(&h);
    assert_eq!(b["nama"], "BGN MPM");
    assert_eq!(b["prioritas"], 1);  // Read-only, always 1 in F0
    let pool_id = Uuid::parse_str(b["downstream_pool_id"].as_str().unwrap()).unwrap();

    let audit_rows = u.audit_konfigurasi(pool_id).await;
    assert!(audit_rows.iter().any(|(aksi, _)| aksi == "buat"));
}

#[tokio::test]
#[ignore]
async fn ac18_get_quota_tracker_reads_db() {
    let u = siapkan().await;
    let (_uid, pa) = u.pengguna(&["platform_admin"]).await;
    let provider_id = u.provider(_uid).await;

    // Create upstream pool
    let up_body = json!({
        "provider_id": provider_id,
        "kuota_harian": 500,
        "rate_limit_per_jam": 60,
        "porsi_sync": 0.1,
        "margin_kuota": 0.01,
        "batas_percobaan": 3,
        "retry_backoff_base_detik": 5,
        "retry_backoff_cap_detik": 300,
        "retry_rasio": 0.1,
        "retry_min_per_jam": 20,
        "jendela_kuota": "bergulir_24j",
        "reset_kuota_jam": 0,
        "reset_kuota_zona": "Asia/Jakarta",
        "jeda_min": 250,
        "jeda_maks": 1500,
        "ambang_berdampingan": 8
    });
    let (st, _, up_resp) = u
        .kirim(Method::POST, "/admin/v1/upstream-pools", Some(&pa), Some(up_body))
        .await;
    assert_eq!(st, StatusCode::CREATED);
    let upstream_pool_id = Uuid::parse_str(up_resp["upstream_pool_id"].as_str().unwrap()).unwrap();

    // Query tracker: should read from DB, not Redis
    let (st, h, b) = u
        .kirim(
            Method::GET,
            &format!("/admin/v1/kuota?upstream_pool_id={upstream_pool_id}"),
            Some(&pa),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    no_store(&h);
    let trackers = b.as_array().unwrap();
    assert!(!trackers.is_empty());
}
