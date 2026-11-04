//! `getSesi`, `perpanjangSesi` and `logout` over HTTP against a real
//! PostgreSQL (SHAA-196, ADR-0003 rev 5 §3.6f). Run by `make -C backend uji-pg`;
//! `#[ignore]`d because it needs `POOLROUTER_TEST_DATABASE_URL`. The clock is
//! injected, so expiry is tested by moving it.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use chrono::{DateTime, Duration, TimeZone, Utc};
use poolrouter_core::admin::sesi::ABSOLUT_DETIK;
use poolrouter_core::admin::sesi_store::SesiStore;
use poolrouter_core::admin::{AsalUi, TokenSesi};
use poolrouter_webserver::admin::sesi::{AdminState, COOKIE_SESI, router};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const ASAL: &str = "https://ui.example.test";

struct Uji {
    app: Router,
    pool: PgPool,
    jam: Arc<Mutex<DateTime<Utc>>>,
    cookie: String,
}

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 8, 0, 0).unwrap()
}

async fn siapkan() -> Uji {
    let url = std::env::var("POOLROUTER_TEST_DATABASE_URL").expect(
        "POOLROUTER_TEST_DATABASE_URL belum diset; jalankan lewat `make -C backend uji-pg`",
    );
    let pool = PgPool::connect(&url).await.expect("database");
    let uid = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO admin_user (pengguna_id, username, password_hash) VALUES ($1, $2, 'x')",
    )
    .bind(uid)
    .bind(format!("sintetis-{}", uid.simple()))
    .execute(&pool)
    .await
    .unwrap();
    let store = SesiStore::baru(pool.clone());
    let token = TokenSesi::baru();
    store
        .terbitkan(uid, &token.hash(), Some("uji"), t0())
        .await
        .unwrap();
    let jam = Arc::new(Mutex::new(t0()));
    let jam2 = jam.clone();
    let state = AdminState::baru(store, AsalUi::baru(ASAL).unwrap())
        .dengan_jam(Arc::new(move || *jam2.lock().unwrap()));
    Uji {
        app: router(state),
        pool,
        jam,
        cookie: format!("{COOKIE_SESI}={}", token.sebagai_b64()),
    }
}

impl Uji {
    fn maju(&self, d: Duration) {
        *self.jam.lock().unwrap() = t0() + d;
    }

    async fn kirim(
        &self,
        method: Method,
        path: &str,
        cookie: Option<&str>,
        origin: Option<&str>,
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        if let Some(o) = origin {
            req = req.header(header::ORIGIN, o);
        }
        let res = self
            .app
            .clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let (parts, body) = res.into_parts();
        let bytes = to_bytes(body, 64 * 1024).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (parts.status, parts.headers, json)
    }

    async fn terakhir_aktif(&self) -> DateTime<Utc> {
        let tok = self.cookie.split('=').nth(1).unwrap().to_owned();
        let hash = TokenSesi::dari_b64(tok).unwrap().hash();
        sqlx::query_scalar(
            "SELECT terakhir_aktif_at FROM admin_session WHERE token_sha256 = decode($1, 'hex')",
        )
        .bind(hash.as_str())
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }
}

fn waktu(v: &Value, k: &str) -> DateTime<Utc> {
    v[k].as_str().unwrap().parse().unwrap()
}

const SESI: &str = "/admin/v1/auth/sesi";
const PERPANJANG: &str = "/admin/v1/auth/sesi/perpanjang";
const LOGOUT: &str = "/admin/v1/auth/logout";

#[tokio::test]
#[ignore]
async fn get_sesi_berulang_tidak_menunda_dan_respons_hanya_tiga_field() {
    let u = siapkan().await;
    for m in [5, 10, 20] {
        u.maju(Duration::minutes(m));
        let (st, h, j) = u.kirim(Method::GET, SESI, Some(&u.cookie), None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(h[header::CACHE_CONTROL], "no-store");
        assert!(h.get(header::SET_COOKIE).is_none());
        let kunci: Vec<_> = j.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            {
                let mut k = kunci;
                k.sort();
                k
            },
            [
                "absolut_berakhir_pada",
                "idle_berakhir_pada",
                "waktu_server"
            ]
        );
        assert_eq!(
            waktu(&j, "idle_berakhir_pada"),
            t0() + Duration::minutes(30)
        );
    }
    assert_eq!(u.terakhir_aktif().await, t0());
    u.maju(Duration::minutes(30));
    let (st, _, j) = u.kirim(Method::GET, SESI, Some(&u.cookie), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_eq!(j["kode"], "tidak_terautentikasi");
}

#[tokio::test]
#[ignore]
async fn perpanjang_menggeser_idle_tanpa_set_cookie_dan_absolut_tetap() {
    let u = siapkan().await;
    u.maju(Duration::minutes(20));
    let (st, h, j) = u
        .kirim(Method::POST, PERPANJANG, Some(&u.cookie), Some(ASAL))
        .await;
    assert_eq!(st, StatusCode::OK);
    assert!(h.get(header::SET_COOKIE).is_none());
    assert_eq!(h[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        waktu(&j, "idle_berakhir_pada"),
        t0() + Duration::minutes(50)
    );
    assert_eq!(
        waktu(&j, "absolut_berakhir_pada"),
        t0() + Duration::seconds(ABSOLUT_DETIK)
    );
}

#[tokio::test]
#[ignore]
async fn perpanjang_origin_asing_403_dan_terakhir_aktif_tidak_berubah() {
    let u = siapkan().await;
    u.maju(Duration::minutes(20));
    for origin in [Some("https://jahat.example"), Some("null"), None] {
        let (st, _, j) = u
            .kirim(Method::POST, PERPANJANG, Some(&u.cookie), origin)
            .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "origin {origin:?}");
        assert_eq!(j["kode"], "origin_tidak_valid");
    }
    assert_eq!(u.terakhir_aktif().await, t0());
}

#[tokio::test]
#[ignore]
async fn mendekati_batas_absolut_idle_sama_dengan_absolut_dan_sesudahnya_401() {
    let u = siapkan().await;
    let mut m = 25;
    while m < ABSOLUT_DETIK / 60 - 10 {
        u.maju(Duration::minutes(m));
        let (st, ..) = u
            .kirim(Method::POST, PERPANJANG, Some(&u.cookie), Some(ASAL))
            .await;
        assert_eq!(st, StatusCode::OK);
        m += 25;
    }
    u.maju(Duration::seconds(ABSOLUT_DETIK) - Duration::minutes(10));
    let (_, _, j) = u
        .kirim(Method::POST, PERPANJANG, Some(&u.cookie), Some(ASAL))
        .await;
    assert_eq!(j["idle_berakhir_pada"], j["absolut_berakhir_pada"]);

    u.maju(Duration::seconds(ABSOLUT_DETIK));
    let (st, ..) = u
        .kirim(Method::POST, PERPANJANG, Some(&u.cookie), Some(ASAL))
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore]
async fn tanpa_cookie_atau_cookie_salah_401() {
    let u = siapkan().await;
    let (st, ..) = u.kirim(Method::GET, SESI, None, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let asing = format!("{COOKIE_SESI}={}", TokenSesi::baru().sebagai_b64());
    let (st, ..) = u.kirim(Method::GET, SESI, Some(&asing), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, ..) = u
        .kirim(
            Method::GET,
            SESI,
            Some(&format!("{COOKIE_SESI}=pendek")),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore]
async fn logout_menghapus_sesi_dan_mengakhiri_cookie() {
    let u = siapkan().await;
    let (st, ..) = u
        .kirim(
            Method::POST,
            LOGOUT,
            Some(&u.cookie),
            Some("https://jahat.example"),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, h, _) = u
        .kirim(Method::POST, LOGOUT, Some(&u.cookie), Some(ASAL))
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let sc = h[header::SET_COOKIE].to_str().unwrap();
    assert!(sc.starts_with("__Host-pr_session=;") && sc.contains("Max-Age=0"));
    assert!(sc.contains("HttpOnly") && sc.contains("Secure") && sc.contains("SameSite=Strict"));

    let (st, ..) = u.kirim(Method::GET, SESI, Some(&u.cookie), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}
