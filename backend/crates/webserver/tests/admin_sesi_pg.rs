//! `getSesi`, `perpanjangSesi` and `logout` over HTTP against a real
//! PostgreSQL (SHAA-196, ADR-0003 rev 5 §3.6f). Run by `make -C backend uji-pg`;
//! `#[ignore]`d because it needs `POOLROUTER_TEST_DATABASE_URL`. The clock is
//! injected, so expiry is tested by moving it.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::connect_info::MockConnectInfo;
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
    let state = AdminState::baru(pool.clone(), AsalUi::baru(ASAL).unwrap())
        .dengan_jam(Arc::new(move || *jam2.lock().unwrap()));
    Uji {
        app: router(state).layer(MockConnectInfo(SocketAddr::from(([203, 0, 113, 9], 4000)))),
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

// ---------------------------------------------------------------- login

use poolrouter_core::admin::password::{KebijakanPassword, ParameterArgon2, hash_password};

const PASSWORD: &str = "kata-sandi-sintetis-15+";
const LOGIN: &str = "/admin/v1/auth/login";

async fn pool_super() -> PgPool {
    let url = std::env::var("POOLROUTER_TEST_DATABASE_URL_SU").expect(
        "POOLROUTER_TEST_DATABASE_URL_SU belum diset; jalankan lewat `make -C backend uji-pg`",
    );
    PgPool::connect(&url).await.expect("database (superuser)")
}

async fn pengguna_login(pool: &PgPool, aktif: bool) -> (Uuid, String) {
    let uid = Uuid::now_v7();
    let nama = format!("sintetis-{}", uid.simple());
    let hash = hash_password(PASSWORD, KebijakanPassword::f0(), ParameterArgon2::OWASP).unwrap();
    sqlx::query("INSERT INTO admin_user (pengguna_id, username, password_hash, aktif) VALUES ($1, $2, $3, $4)")
        .bind(uid)
        .bind(&nama)
        .bind(hash.as_str())
        .bind(aktif)
        .execute(pool)
        .await
        .unwrap();
    (uid, nama)
}

impl Uji {
    async fn login(
        &self,
        username: &str,
        password: &str,
        origin: Option<&str>,
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let body = serde_json::json!({"username": username, "password": password}).to_string();
        self.login_mentah(body, origin).await
    }

    async fn login_mentah(
        &self,
        body: String,
        origin: Option<&str>,
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let mut req = Request::builder()
            .method(Method::POST)
            .uri(LOGIN)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(o) = origin {
            req = req.header(header::ORIGIN, o);
        }
        let res = self
            .app
            .clone()
            .oneshot(req.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let (parts, body) = res.into_parts();
        let bytes = to_bytes(body, 64 * 1024).await.unwrap();
        (
            parts.status,
            parts.headers,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

async fn jumlah_audit(su: &PgPool, uid: Option<Uuid>, hasil: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_login WHERE pengguna_id IS NOT DISTINCT FROM $1 AND hasil = $2",
    )
    .bind(uid)
    .bind(hasil)
    .fetch_one(su)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore]
async fn login_berhasil_menerbitkan_cookie_yang_berlaku_dan_tercatat() {
    let u = siapkan().await;
    let su = pool_super().await;
    let (uid, nama) = pengguna_login(&u.pool, true).await;

    let (st, h, _) = u.login(&nama, PASSWORD, Some(ASAL)).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(h[header::CACHE_CONTROL], "no-store");
    let sc = h[header::SET_COOKIE].to_str().unwrap().to_owned();
    assert!(sc.starts_with("__Host-pr_session="));
    for bagian in ["Path=/", "HttpOnly", "Secure", "SameSite=Strict"] {
        assert!(sc.contains(bagian), "{sc}");
    }
    assert!(!sc.contains("Domain"));
    let cookie = sc.split(';').next().unwrap().to_owned();
    let (st, ..) = u.kirim(Method::GET, SESI, Some(&cookie), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(jumlah_audit(&su, Some(uid), "berhasil").await, 1);
}

#[tokio::test]
#[ignore]
async fn login_gagal_tidak_membedakan_pengguna_salah_dan_password_salah() {
    let u = siapkan().await;
    let su = pool_super().await;
    let (uid, nama) = pengguna_login(&u.pool, true).await;
    let (inaktif, nama_inaktif) = pengguna_login(&u.pool, false).await;

    let (s1, h1, j1) = u.login(&nama, "password-salah-sekali-ya", Some(ASAL)).await;
    let (s2, _, j2) = u.login("tidak-ada-orang-ini", PASSWORD, Some(ASAL)).await;
    let (s3, _, j3) = u.login(&nama_inaktif, PASSWORD, Some(ASAL)).await;
    for (st, j) in [(s1, &j1), (s2, &j2), (s3, &j3)] {
        assert_eq!(st, StatusCode::UNAUTHORIZED);
        assert_eq!(j["kode"], "kredensial_salah");
    }
    assert_eq!(j1, j2);
    assert!(h1.get(header::SET_COOKIE).is_none());
    assert_eq!(jumlah_audit(&su, Some(uid), "gagal").await, 1);
    assert_eq!(jumlah_audit(&su, Some(inaktif), "gagal").await, 1);
}

#[tokio::test]
#[ignore]
async fn lima_kegagalan_mengunci_username_lima_belas_menit() {
    let u = siapkan().await;
    let (_, nama) = pengguna_login(&u.pool, true).await;
    for i in 0..5 {
        u.maju(Duration::minutes(i));
        let (st, ..) = u.login(&nama, "password-salah-sekali-ya", Some(ASAL)).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED, "percobaan {i}");
    }
    // Locked: even the right password is refused, with Retry-After.
    u.maju(Duration::minutes(5));
    let (st, h, j) = u.login(&nama, PASSWORD, Some(ASAL)).await;
    assert_eq!(st, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(j["kode"], "login_dibatasi");
    assert!(
        h[header::RETRY_AFTER]
            .to_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            >= 1
    );
    // After the lock the right password works again.
    u.maju(Duration::minutes(4 + 15));
    let (st, ..) = u.login(&nama, PASSWORD, Some(ASAL)).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
}

#[tokio::test]
#[ignore]
async fn login_origin_asing_403_dan_body_salah_422_tanpa_sesi() {
    let u = siapkan().await;
    let (uid, nama) = pengguna_login(&u.pool, true).await;
    for origin in [Some("https://jahat.example"), None] {
        let (st, _, j) = u.login(&nama, PASSWORD, origin).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
        assert_eq!(j["kode"], "origin_tidak_valid");
    }
    let (st, _, j) = u.login_mentah("{\"username\": 5}".into(), Some(ASAL)).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(j["kode"], "validasi_gagal");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM admin_session WHERE pengguna_id = $1")
        .bind(uid)
        .fetch_one(&u.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

// ---------------------------------------------------------------- me / preferensi

const ME: &str = "/admin/v1/auth/me";
const PREFERENSI: &str = "/admin/v1/auth/me/preferensi";

impl Uji {
    async fn pengguna_id(&self) -> Uuid {
        let tok = self.cookie.split('=').nth(1).unwrap().to_owned();
        let hash = TokenSesi::dari_b64(tok).unwrap().hash();
        sqlx::query_scalar(
            "SELECT pengguna_id FROM admin_session WHERE token_sha256 = decode($1, 'hex')",
        )
        .bind(hash.as_str())
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn patch(
        &self,
        cookie: Option<&str>,
        origin: Option<&str>,
        body: &str,
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let mut req = Request::builder()
            .method(Method::PATCH)
            .uri(PREFERENSI)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        if let Some(o) = origin {
            req = req.header(header::ORIGIN, o);
        }
        let res = self
            .app
            .clone()
            .oneshot(req.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap();
        let (parts, body) = res.into_parts();
        let bytes = to_bytes(body, 64 * 1024).await.unwrap();
        (
            parts.status,
            parts.headers,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn bahasa(&self, uid: Uuid) -> Option<String> {
        sqlx::query_scalar("SELECT bahasa_ui FROM admin_user WHERE pengguna_id = $1")
            .bind(uid)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

#[tokio::test]
#[ignore]
async fn get_me_memuat_identitas_peran_dan_bahasa_ui() {
    let u = siapkan().await;
    let uid = u.pengguna_id().await;
    let (st, _, j) = u.kirim(Method::GET, ME, Some(&u.cookie), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(j["pengguna_id"], uid.to_string());
    assert_eq!(j["peran"], serde_json::json!([]));
    assert!(
        j.get("bahasa_ui").is_none(),
        "belum diatur: field tidak ada"
    );
    assert!(j.get("client_id").is_none());

    sqlx::query(
        "INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh) VALUES ($1, 'platform_admin', $1)",
    )
    .bind(uid)
    .execute(&u.pool)
    .await
    .unwrap();
    u.patch(Some(&u.cookie), Some(ASAL), r#"{"bahasa_ui":"en"}"#)
        .await;
    let (_, _, j) = u.kirim(Method::GET, ME, Some(&u.cookie), None).await;
    assert_eq!(j["peran"], serde_json::json!(["platform_admin"]));
    assert_eq!(j["bahasa_ui"], "en");

    let (st, ..) = u.kirim(Method::GET, ME, None, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore]
async fn patch_preferensi_sukses_tanpa_menyentuh_sesi_dan_tanpa_audit_akses() {
    let u = siapkan().await;
    let su = pool_super().await;
    let uid = u.pengguna_id().await;
    u.maju(Duration::minutes(10));

    let (st, h, j) = u
        .patch(Some(&u.cookie), Some(ASAL), r#"{"bahasa_ui":"en"}"#)
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(h[header::CACHE_CONTROL], "no-store");
    assert!(h.get(header::SET_COOKIE).is_none());
    assert_eq!(j, serde_json::json!({"bahasa_ui": "en"}));
    assert_eq!(u.bahasa(uid).await.as_deref(), Some("en"));
    // The session was not touched: idle still counts from login.
    assert_eq!(u.terakhir_aktif().await, t0());
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_akses WHERE pengguna_id = $1")
        .bind(uid)
        .fetch_one(&su)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
#[ignore]
async fn patch_preferensi_ditolak_tanpa_sesi_origin_asing_dan_nilai_salah() {
    let u = siapkan().await;
    let uid = u.pengguna_id().await;

    let (st, ..) = u.patch(None, Some(ASAL), r#"{"bahasa_ui":"en"}"#).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    for origin in [Some("https://jahat.example"), None] {
        let (st, _, j) = u
            .patch(Some(&u.cookie), origin, r#"{"bahasa_ui":"en"}"#)
            .await;
        assert_eq!(st, StatusCode::FORBIDDEN);
        assert_eq!(j["kode"], "origin_tidak_valid");
    }
    for badan in [r#"{"bahasa_ui":"fr"}"#, r#"{}"#, "bukan json"] {
        let (st, _, j) = u.patch(Some(&u.cookie), Some(ASAL), badan).await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{badan}");
        assert_eq!(j["kode"], "validasi_gagal");
    }
    assert_eq!(u.bahasa(uid).await, None);
}

#[tokio::test]
#[ignore]
async fn patch_preferensi_tidak_bisa_mengubah_pengguna_lain() {
    let u = siapkan().await;
    let (lain, _) = pengguna_login(&u.pool, true).await;
    // Any attempt to name another user is an unknown field, hence 422.
    let badan = format!(r#"{{"bahasa_ui":"en","pengguna_id":"{lain}"}}"#);
    let (st, ..) = u.patch(Some(&u.cookie), Some(ASAL), &badan).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(u.bahasa(lain).await, None);
    // And a valid call changes only the caller.
    u.patch(Some(&u.cookie), Some(ASAL), r#"{"bahasa_ui":"en"}"#)
        .await;
    assert_eq!(u.bahasa(lain).await, None);
}
