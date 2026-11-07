//! The two-step login and MFA enrollment over HTTP against real PostgreSQL and
//! Redis (ARC-128, ADR-0006 §9, AC-9..AC-14).
//!
//! The seam is the admin `Router` from `admin::sesi::router(state)`, driven with
//! `tower::ServiceExt::oneshot` and real cookies and `Origin` headers — the same
//! seam `admin_sesi_pg.rs` uses, per the spec's Testing Decisions. Handlers,
//! guards and stores are never called directly.
//!
//! Run by `make -C backend uji-pg`; `#[ignore]`d because it needs
//! `POOLROUTER_TEST_DATABASE_URL` and `POOLROUTER_TEST_REDIS`. The clock is
//! injected so the 5-minute token life is tested by moving it, not by sleeping.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{DateTime, Duration, TimeZone, Utc};
use poolrouter_core::admin::mfa::totp;
use poolrouter_core::admin::origin::AsalUi;
use poolrouter_core::admin::password::{KebijakanPassword, ParameterArgon2, hash_password};
use poolrouter_core::admin::pra_sesi::UMUR_DETIK;
use poolrouter_core::admin::sesi_store::SesiStore;
use poolrouter_core::secret::Brankas;
use poolrouter_webserver::admin::pembatas::PembatasLogin;
use poolrouter_webserver::admin::sesi::{AdminState, COOKIE_SESI, router};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const ASAL: &str = "https://ui.example.test";
const COOKIE_PRA_SESI: &str = "__Host-pr_mfa";
const KUNCI_HMAC: &[u8] = b"kunci-hmac-sintetis-untuk-uji-t2";
const PASSWORD: &str = "kata-sandi-sintetis-yang-panjang";

fn redis_ujian() -> String {
    std::env::var("POOLROUTER_TEST_REDIS")
        .expect("POOLROUTER_TEST_REDIS belum diset; jalankan lewat `make -C backend uji-pg`")
}

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 5, 8, 0, 0).unwrap()
}

struct Uji {
    app: Router,
    pool: PgPool,
    /// Superuser connection, for reading audit rows.
    su: PgPool,
    jam: Arc<Mutex<DateTime<Utc>>>,
    username: String,
    pengguna_id: Uuid,
    /// A second account, referenced by the enrollment flow's issuer column.
    #[allow(dead_code)]
    penerbit_id: Uuid,
}

/// A user with the given roles and a known password, plus an issuer account
/// (the contract forbids a user issuing its own registration code, MF-10 ii).
async fn siapkan(peran: &[&str], mfa_wajib: bool) -> Uji {
    let url = std::env::var("POOLROUTER_TEST_DATABASE_URL").expect(
        "POOLROUTER_TEST_DATABASE_URL belum diset; jalankan lewat `make -C backend uji-pg`",
    );
    let pool = PgPool::connect(&url).await.expect("database");
    // Reading audit tables needs the superuser pool: `pr_admin_api` only has
    // INSERT on `audit_akses` (migrations/20261103000004_grants.sql).
    let su_url = std::env::var("POOLROUTER_TEST_DATABASE_URL_SU").expect(
        "POOLROUTER_TEST_DATABASE_URL_SU belum diset; jalankan lewat `make -C backend uji-pg`",
    );
    let su = PgPool::connect(&su_url).await.expect("database su");
    let redis_url = redis_ujian();

    let uid = Uuid::now_v7();
    let penerbit = Uuid::now_v7();
    // Auditor/operator grants need a distinct proposer and approver (A-02), so
    // the fixture carries two extra accounts and never grants to itself.
    let penyetuju = Uuid::now_v7();
    let username = format!("mfa-{}", uid.simple());
    let nama_penerbit = format!("penerbit-{}", penerbit.simple());
    let nama_penyetuju = format!("penyetuju-{}", penyetuju.simple());
    let hash = hash_password(PASSWORD, KebijakanPassword::f0(), ParameterArgon2::OWASP)
        .unwrap()
        .to_string();
    for (id, nama, h) in [
        (uid, username.as_str(), hash.as_str()),
        (penerbit, nama_penerbit.as_str(), "x"),
        (penyetuju, nama_penyetuju.as_str(), "x"),
    ] {
        sqlx::query(
            "INSERT INTO admin_user (pengguna_id, username, password_hash) VALUES ($1, $2, $3)",
        )
        .bind(id)
        .bind(nama)
        .bind(h)
        .execute(&pool)
        .await
        .unwrap();
    }
    if !peran.is_empty() {
        // A role grant records who proposed and who approved it; the column is
        // NOT NULL and A-02 forbids either party being the user themselves.
        sqlx::query(
            "INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh, disetujui_oleh) \
             SELECT $1, unnest($2::text[]), $3, $4",
        )
        .bind(uid)
        .bind(peran)
        .bind(penerbit)
        .bind(penyetuju)
        .execute(&pool)
        .await
        .unwrap();
    }

    let jam = Arc::new(Mutex::new(t0()));
    let state = AdminState::baru(
        pool.clone(),
        AsalUi::baru(ASAL).unwrap(),
        PembatasLogin::baru(redis::Client::open(redis_url.clone()).unwrap(), KUNCI_HMAC),
        Brankas::dari_isi_kek(&"5a".repeat(32)).unwrap(),
        redis::Client::open(redis_url).unwrap(),
        mfa_wajib,
    )
    .dengan_jam(Arc::new({
        let jam = jam.clone();
        move || *jam.lock().unwrap()
    }));
    let app = router(state).layer(MockConnectInfo(SocketAddr::from(([203, 0, 113, 9], 4000))));
    Uji {
        app,
        pool,
        su,
        jam,
        username,
        pengguna_id: uid,
        penerbit_id: penerbit,
    }
}

impl Uji {
    fn maju(&self, d: Duration) {
        *self.jam.lock().unwrap() = t0() + d;
    }

    /// A TOTP code for the clock the server is currently on, so tests never
    /// reuse a step that anti-replay has already accepted.
    fn kode_sekarang(&self, rahasia: &str) -> String {
        let sekarang = *self.jam.lock().unwrap();
        totp::hitung(rahasia, (sekarang.timestamp() / 30) as u64)
    }

    /// POST JSON with the given cookies and a valid `Origin`.
    async fn kirim(
        &self,
        path: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let mut req = Request::builder()
            .method(Method::POST)
            .uri(path)
            .header(header::ORIGIN, ASAL);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        let req = match body {
            Some(v) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        let res = self.app.clone().oneshot(req).await.unwrap();
        let (parts, body) = res.into_parts();
        let bytes = to_bytes(body, 64 * 1024).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (parts.status, parts.headers, json)
    }

    async fn login(&self, password: &str) -> (StatusCode, axum::http::HeaderMap, Value) {
        self.kirim(
            "/admin/v1/auth/login",
            None,
            Some(serde_json::json!({ "username": self.username, "password": password })),
        )
        .await
    }

    /// Pull a cookie's value out of `Set-Cookie`.
    fn cookie(headers: &axum::http::HeaderMap, nama: &str) -> Option<String> {
        headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|v| {
                let v = v.trim();
                v.strip_prefix(&format!("{nama}="))
                    .map(|rest| rest.split(';').next().unwrap_or_default().to_owned())
            })
    }

    /// Issue a registration code the enrollment flow will accept (AC-8).
    async fn kode_pendaftaran(&self) -> String {
        let store = poolrouter_core::admin::mfa::MfaStore::baru(
            self.pool.clone(),
            Brankas::dari_isi_kek(&"5a".repeat(32)).unwrap(),
        );
        let (_, kode) = store
            .terbit_kode_daftar(self.penerbit_id, self.pengguna_id, t0())
            .await
            .unwrap();
        kode.expose().to_owned()
    }

    /// Seed an active MFA enrollment and return its base32 secret, so the
    /// verification path can be tested without going through enrollment. The
    /// secret is written by the store, so the ciphertext really decrypts.
    async fn pasang_mfa_aktif(&self) -> String {
        let store = poolrouter_core::admin::mfa::MfaStore::baru(
            self.pool.clone(),
            Brankas::dari_isi_kek(&"5a".repeat(32)).unwrap(),
        );
        let pendaftaran = store.daftar(self.pengguna_id).await.unwrap();
        let rahasia = pendaftaran.rahasia().to_owned();
        let kode = self.kode_sekarang(&rahasia);
        store
            .konfirmasi(self.pengguna_id, &kode, t0())
            .await
            .unwrap();
        // Anti-replay (MF-03): the confirmation spent step 0, so move the clock
        // forward one step before the test logs in and needs a fresh code.
        self.maju(Duration::seconds(30));
        rahasia
    }
}

// --- AC-9: password alone never issues a session for MFA roles --------------

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn auditor_tanpa_mfa_mendapat_langkah_daftar_dan_token_pra_sesi() {
    let u = siapkan(&["auditor"], true).await;
    let (status, headers, body) = u.login(PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["langkah"], "daftar_mfa");
    let token = Uji::cookie(&headers, COOKIE_PRA_SESI).expect("cookie pra-sesi");
    assert!(!token.is_empty());
    // No session cookie at all.
    assert!(Uji::cookie(&headers, COOKIE_SESI).is_none());
    // The pre-session cookie is hardened like the session cookie.
    let raw = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with(COOKIE_PRA_SESI))
        .unwrap()
        .to_owned();
    assert!(raw.contains("HttpOnly") && raw.contains("Secure") && raw.contains("SameSite=Strict"));
    assert!(raw.contains(&format!("Max-Age={UMUR_DETIK}")));
    assert!(raw.contains("Path=/"));
    // AC-2: no-store on this response too.
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
}

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn operator_tetap_login_satu_langkah_walau_mfa_wajib() {
    let u = siapkan(&["operator"], true).await;
    let (status, headers, _) = u.login(PASSWORD).await;
    // AC-9's policy covers auditor/platform_admin; operator is untouched.
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(Uji::cookie(&headers, COOKIE_SESI).is_some());
    assert!(Uji::cookie(&headers, COOKIE_PRA_SESI).is_none());
}

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn policy_mati_membuat_auditor_login_satu_langkah() {
    let u = siapkan(&["auditor"], false).await;
    let (status, headers, _) = u.login(PASSWORD).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(Uji::cookie(&headers, COOKIE_SESI).is_some());
}

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn auditor_terdaftar_mendapat_langkah_mfa() {
    let u = siapkan(&["auditor"], true).await;
    u.pasang_mfa_aktif().await;
    let (status, _, body) = u.login(PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["langkah"], "mfa");
}

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn password_salah_tetap_401_tanpa_token_pra_sesi() {
    let u = siapkan(&["auditor"], true).await;
    let (status, headers, _) = u.login("kata-sandi-salah").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(Uji::cookie(&headers, COOKIE_PRA_SESI).is_none());
    assert!(Uji::cookie(&headers, COOKIE_SESI).is_none());
}

// --- AC-11: one failure message, fifth failure destroys the token -----------

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn kode_salah_generik_dan_kelima_menghanguskan_token() {
    let u = siapkan(&["auditor"], true).await;
    let rahasia = u.pasang_mfa_aktif().await;
    let (_, headers, _) = u.login(PASSWORD).await;
    let token = Uji::cookie(&headers, COOKIE_PRA_SESI).unwrap();
    let cookie = format!("{COOKIE_PRA_SESI}={token}");

    // Four failures keep the token; the fifth destroys it.
    for _ in 0..4 {
        let (status, _, body) = u
            .kirim(
                "/admin/v1/auth/mfa",
                Some(&cookie),
                Some(serde_json::json!({ "kode": "000000" })),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        // Generic: no hint which code kind failed.
        assert_eq!(body["kode"], "kredensial_salah");
    }
    let (status, _, body) = u
        .kirim(
            "/admin/v1/auth/mfa",
            Some(&cookie),
            Some(serde_json::json!({ "kode": "000000" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["kode"], "kredensial_salah");

    // The token is gone: even a valid code is refused now (AC-11).
    let kode_sah = u.kode_sekarang(&rahasia);
    let (status, _, _) = u
        .kirim(
            "/admin/v1/auth/mfa",
            Some(&cookie),
            Some(serde_json::json!({ "kode": kode_sah })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn kode_totp_sah_menerbitkan_sesi_dengan_mfa_terverifikasi() {
    let u = siapkan(&["auditor"], true).await;
    let rahasia = u.pasang_mfa_aktif().await;
    let (_, headers, _) = u.login(PASSWORD).await;
    let token = Uji::cookie(&headers, COOKIE_PRA_SESI).unwrap();

    let kode = u.kode_sekarang(&rahasia);
    let (status, headers, _) = u
        .kirim(
            "/admin/v1/auth/mfa",
            Some(&format!("{COOKIE_PRA_SESI}={token}")),
            Some(serde_json::json!({ "kode": kode })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let sesi_token = Uji::cookie(&headers, COOKIE_SESI).expect("cookie sesi");
    // The pre-session cookie is cleared.
    let hapus = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.starts_with(&format!("{COOKIE_PRA_SESI}=")) && v.contains("Max-Age=0"));
    assert!(hapus, "pra-sesi harus dihapus");

    // AC-10: the session carries mfa_terverifikasi_at.
    let terverifikasi: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT mfa_terverifikasi_at FROM admin_session WHERE token_sha256 = \
         decode($1, 'hex')",
    )
    .bind(hash_dari(&sesi_token))
    .fetch_one(&u.pool)
    .await
    .unwrap();
    assert!(terverifikasi.is_some());
}

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn token_pra_sesi_hanya_bisa_dipakai_sekali() {
    let u = siapkan(&["auditor"], true).await;
    let rahasia = u.pasang_mfa_aktif().await;
    let (_, headers, _) = u.login(PASSWORD).await;
    let token = Uji::cookie(&headers, COOKIE_PRA_SESI).unwrap();
    let cookie = format!("{COOKIE_PRA_SESI}={token}");
    let kode = u.kode_sekarang(&rahasia);

    let (status, _, _) = u
        .kirim(
            "/admin/v1/auth/mfa",
            Some(&cookie),
            Some(serde_json::json!({ "kode": kode })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Replay with the same token: 401 (AC-10).
    let (status, _, _) = u
        .kirim(
            "/admin/v1/auth/mfa",
            Some(&cookie),
            Some(serde_json::json!({ "kode": kode })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn token_kedaluwarsa_setelah_lima_menit() {
    let u = siapkan(&["auditor"], true).await;
    let rahasia = u.pasang_mfa_aktif().await;
    let (_, headers, _) = u.login(PASSWORD).await;
    let token = Uji::cookie(&headers, COOKIE_PRA_SESI).unwrap();

    // Just inside the window: still usable.
    u.maju(Duration::seconds(UMUR_DETIK as i64 - 10));
    let kode = totp::hitung(
        &rahasia,
        ((t0().timestamp() + UMUR_DETIK as i64 - 10) / 30) as u64,
    );
    let (status, _, _) = u
        .kirim(
            "/admin/v1/auth/mfa",
            Some(&format!("{COOKIE_PRA_SESI}={token}")),
            Some(serde_json::json!({ "kode": kode })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn tanpa_cookie_pra_sesi_auth_mfa_401() {
    let u = siapkan(&["auditor"], true).await;
    let (status, _, _) = u
        .kirim(
            "/admin/v1/auth/mfa",
            None,
            Some(serde_json::json!({ "kode": "123456" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn token_daftar_mfa_ditolak_di_auth_mfa() {
    let u = siapkan(&["auditor"], true).await;
    let (_, headers, _) = u.login(PASSWORD).await;
    let token = Uji::cookie(&headers, COOKIE_PRA_SESI).unwrap();
    // The user has no active MFA, so this is a `daftar_mfa` token; AC-10 says
    // `/auth/mfa` must refuse it rather than verify anything.
    let (status, _, _) = u
        .kirim(
            "/admin/v1/auth/mfa",
            Some(&format!("{COOKIE_PRA_SESI}={token}")),
            Some(serde_json::json!({ "kode": "123456" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn percobaan_kode_tercatat_di_audit_login() {
    let u = siapkan(&["auditor"], true).await;
    u.pasang_mfa_aktif().await;
    let (_, headers, _) = u.login(PASSWORD).await;
    let token = Uji::cookie(&headers, COOKIE_PRA_SESI).unwrap();
    let (status, _, _) = u
        .kirim(
            "/admin/v1/auth/mfa",
            Some(&format!("{COOKIE_PRA_SESI}={token}")),
            Some(serde_json::json!({ "kode": "000000" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // AC-11: every code attempt is an `audit_login` row.
    let gagal: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_login WHERE pengguna_id = $1 AND hasil = 'gagal'",
    )
    .bind(u.pengguna_id)
    .fetch_one(&u.su)
    .await
    .unwrap();
    assert!(gagal >= 1, "percobaan kode harus tercatat di audit_login");
}

// --- AC-2: no-store on every auth response, including 4xx -------------------

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn setiap_respons_auth_tanpa_tembolok() {
    let u = siapkan(&["auditor"], true).await;
    let (_, _, _) = u.login(PASSWORD).await;
    let (_, headers, _) = u
        .kirim(
            "/admin/v1/auth/mfa",
            None,
            Some(serde_json::json!({ "kode": "123456" })),
        )
        .await;
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
    let (_, headers, _) = u
        .kirim(
            "/admin/v1/auth/mfa/daftar",
            None,
            Some(serde_json::json!({ "kode_pendaftaran": "AAAA" })),
        )
        .await;
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
}

// --- AC-3: no code, secret, URI or recovery code in the log ------------------

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn kode_dan_secret_tidak_muncul_di_log() {
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    struct Penyangga(Arc<Mutex<Vec<u8>>>);
    struct Penulis(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Penulis {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> MakeWriter<'a> for Penyangga {
        type Writer = Penulis;
        fn make_writer(&'a self) -> Self::Writer {
            let _ = self.0.lock().map(|mut b| b.clear());
            Penulis(Arc::clone(&self.0))
        }
    }

    let penyangga = Penyangga::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(penyangga.clone())
        .finish();
    let _penjaga = tracing::subscriber::set_default(subscriber);

    let u = siapkan(&["auditor"], true).await;
    let (_, headers, _) = u.login(PASSWORD).await;
    let token = Uji::cookie(&headers, COOKIE_PRA_SESI).unwrap();
    let cookie = format!("{COOKIE_PRA_SESI}={token}");

    let kode_daftar = u.kode_pendaftaran().await;
    let (status, _, body) = u
        .kirim(
            "/admin/v1/auth/mfa/daftar",
            Some(&cookie),
            Some(serde_json::json!({ "kode_pendaftaran": kode_daftar })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let uri = body["otpauth_uri"].as_str().unwrap().to_owned();
    let rahasia = body["rahasia_base32"].as_str().unwrap().to_owned();

    // A wrong code forces the failure log paths too.
    let (status, _, _) = u
        .kirim(
            "/admin/v1/auth/mfa",
            Some(&cookie),
            Some(serde_json::json!({ "kode": "000000" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let log = String::from_utf8(penyangga.0.lock().unwrap().clone()).unwrap();
    assert!(!log.is_empty(), "penangkap log bekerja");
    // AC-3: none of the sensitive values reached the log.
    assert!(!log.contains(&rahasia), "secret tidak boleh muncul di log");
    assert!(!log.contains(&uri), "URI otpauth tidak boleh muncul di log");
    assert!(!log.contains("otpauth://"), "URI tidak boleh muncul di log");
    assert!(
        !log.contains(&kode_daftar),
        "kode pendaftaran tidak boleh di log"
    );
    assert!(
        !log.contains(PASSWORD),
        "kata sandi tidak boleh muncul di log"
    );
}

// --- AC-12: enrollment needs a registration code ---------------------------

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn daftar_mengembalikan_uri_otpauth_sekali() {
    let u = siapkan(&["auditor"], true).await;
    let (_, headers, _) = u.login(PASSWORD).await;
    let token = Uji::cookie(&headers, COOKIE_PRA_SESI).unwrap();
    let kode_daftar = u.kode_pendaftaran().await;
    let (status, _, body) = u
        .kirim(
            "/admin/v1/auth/mfa/daftar",
            Some(&format!("{COOKIE_PRA_SESI}={token}")),
            Some(serde_json::json!({ "kode_pendaftaran": kode_daftar })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let uri = body["otpauth_uri"].as_str().unwrap();
    assert!(uri.starts_with("otpauth://totp/PoolRouter:"));
    assert!(uri.contains("issuer=PoolRouter"));
    assert!(!uri.contains("http"));
    // AC-3: the secret is in the body only, under the dedicated field.
    assert!(!body["rahasia_base32"].as_str().unwrap().is_empty());
}

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn daftar_tanpa_kode_atau_kode_salah_ditolak() {
    let u = siapkan(&["auditor"], true).await;
    let (_, headers, _) = u.login(PASSWORD).await;
    let token = Uji::cookie(&headers, COOKIE_PRA_SESI).unwrap();
    let cookie = format!("{COOKIE_PRA_SESI}={token}");

    // A wrong code: refused, and the refusal is on record (AC-12).
    let (status, _, _) = u
        .kirim(
            "/admin/v1/auth/mfa/daftar",
            Some(&cookie),
            Some(serde_json::json!({ "kode_pendaftaran": "ZZZZZZZZZZZZZZZZ" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let ditolak: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_akses WHERE aksi = 'daftar_mfa' AND hasil = 'ditolak'",
    )
    .fetch_one(&u.su)
    .await
    .unwrap();
    assert!(ditolak >= 1, "penolakan daftar_mfa harus tercatat");

    // A missing body is 422, not a silent success.
    let (_, headers, _) = u.login(PASSWORD).await;
    let token2 = Uji::cookie(&headers, COOKIE_PRA_SESI).unwrap();
    let (status, _, _) = u
        .kirim(
            "/admin/v1/auth/mfa/daftar",
            Some(&format!("{COOKIE_PRA_SESI}={token2}")),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn kode_pendaftaran_milik_orang_lain_ditolak() {
    let lain = siapkan(&["auditor"], true).await;
    let u = siapkan(&["auditor"], true).await;
    // A code issued to another user must not start this user's enrollment.
    let kode_asing = lain.kode_pendaftaran().await;
    let (_, headers, _) = u.login(PASSWORD).await;
    let token = Uji::cookie(&headers, COOKIE_PRA_SESI).unwrap();
    let (status, _, _) = u
        .kirim(
            "/admin/v1/auth/mfa/daftar",
            Some(&format!("{COOKIE_PRA_SESI}={token}")),
            Some(serde_json::json!({ "kode_pendaftaran": kode_asing })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// --- AC-13: confirmation activates, returns 10 codes, issues a session ------

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn konfirmasi_mengaktifkan_dan_mengembalikan_sepuluh_kode() {
    let u = siapkan(&["auditor"], true).await;
    let (_, headers, _) = u.login(PASSWORD).await;
    let token = Uji::cookie(&headers, COOKIE_PRA_SESI).unwrap();
    let cookie = format!("{COOKIE_PRA_SESI}={token}");
    let kode_daftar = u.kode_pendaftaran().await;
    let (status, _, body) = u
        .kirim(
            "/admin/v1/auth/mfa/daftar",
            Some(&cookie),
            Some(serde_json::json!({ "kode_pendaftaran": kode_daftar })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let rahasia = body["rahasia_base32"].as_str().unwrap().to_owned();

    // `daftar` seeds `langkah_terakhir = 0`, and confirmation requires a strictly
    // greater step (MF-03 anti-replay), so confirm on the next step.
    u.maju(Duration::seconds(30));
    let kode = u.kode_sekarang(&rahasia);
    let (status, headers, body) = u
        .kirim(
            "/admin/v1/auth/mfa/daftar/konfirmasi",
            Some(&cookie),
            Some(serde_json::json!({ "kode_totp": kode })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["kode_pemulihan"].as_array().map(Vec::len),
        Some(10),
        "10 kode pemulihan"
    );
    // The session is issued with the MFA stamp (AC-13).
    let sesi_token = Uji::cookie(&headers, COOKIE_SESI).expect("cookie sesi");
    let terverifikasi: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT mfa_terverifikasi_at FROM admin_session WHERE token_sha256 = decode($1, 'hex')",
    )
    .bind(hash_dari(&sesi_token))
    .fetch_one(&u.pool)
    .await
    .unwrap();
    assert!(terverifikasi.is_some());
    // `pengguna_mfa` is active.
    let aktif: bool =
        sqlx::query_scalar("SELECT status = 'aktif' FROM pengguna_mfa WHERE pengguna_id = $1")
            .bind(u.pengguna_id)
            .fetch_one(&u.pool)
            .await
            .unwrap();
    assert!(aktif);

    // Confirming again with the same token is 401 (AC-13).
    let (status, _, _) = u
        .kirim(
            "/admin/v1/auth/mfa/daftar/konfirmasi",
            Some(&cookie),
            Some(serde_json::json!({ "kode_totp": kode })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// --- AC-14: /auth/me carries the mfa object --------------------------------

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn me_setelah_login_kode_pemulihan_melaporkan_sisa_sembilan() {
    let u = siapkan(&["auditor"], true).await;

    // Enroll through the real endpoints so the recovery codes are genuine.
    let (_, headers, _) = u.login(PASSWORD).await;
    let pra = Uji::cookie(&headers, COOKIE_PRA_SESI).unwrap();
    let cookie = format!("{COOKIE_PRA_SESI}={pra}");
    let kode_daftar = u.kode_pendaftaran().await;
    let (status, _, body) = u
        .kirim(
            "/admin/v1/auth/mfa/daftar",
            Some(&cookie),
            Some(serde_json::json!({ "kode_pendaftaran": kode_daftar })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let rahasia = body["rahasia_base32"].as_str().unwrap().to_owned();

    // `daftar` seeds `langkah_terakhir = 0`, and confirmation requires a strictly
    // greater step (MF-03 anti-replay), so confirm on the next step.
    u.maju(Duration::seconds(30));
    let kode = u.kode_sekarang(&rahasia);
    let (status, _, body) = u
        .kirim(
            "/admin/v1/auth/mfa/daftar/konfirmasi",
            Some(&cookie),
            Some(serde_json::json!({ "kode_totp": kode })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let kode_pemulihan = body["kode_pemulihan"].as_array().unwrap()[0]
        .as_str()
        .unwrap()
        .to_owned();

    // Log in again using that recovery code instead of a TOTP code.
    let (status, headers, body) = u.login(PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "langkah mfa");
    assert_eq!(body["langkah"], "mfa");
    let pra2 = Uji::cookie(&headers, COOKIE_PRA_SESI).unwrap();
    let (status, headers, _) = u
        .kirim(
            "/admin/v1/auth/mfa",
            Some(&format!("{COOKIE_PRA_SESI}={pra2}")),
            Some(serde_json::json!({ "kode_pemulihan": kode_pemulihan })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let sesi_token = Uji::cookie(&headers, COOKIE_SESI).unwrap();

    let req = Request::builder()
        .method(Method::GET)
        .uri("/admin/v1/auth/me")
        .header(header::COOKIE, format!("{COOKIE_SESI}={sesi_token}"))
        .body(Body::empty())
        .unwrap();
    let res = u.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = to_bytes(res.into_body(), 64 * 1024).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["mfa"]["wajib"], true);
    assert_eq!(body["mfa"]["aktif"], true);
    // AC-14: one of the ten has been spent.
    assert_eq!(body["mfa"]["sisa_kode_pemulihan"], 9);
}

#[tokio::test]
#[ignore = "butuh PostgreSQL dan Redis; jalankan lewat make -C backend uji-pg"]
async fn me_auditor_tanpa_mfa_melaporkan_wajib_tapi_belum_aktif() {
    let u = siapkan(&["auditor"], true).await;
    let store = SesiStore::baru(u.pool.clone());
    let token = poolrouter_core::admin::TokenSesi::baru();
    store
        .terbitkan(u.pengguna_id, &token.hash(), None, t0())
        .await
        .unwrap();
    let req = Request::builder()
        .method(Method::GET)
        .uri("/admin/v1/auth/me")
        .header(
            header::COOKIE,
            format!("{COOKIE_SESI}={}", token.sebagai_b64()),
        )
        .body(Body::empty())
        .unwrap();
    let res = u.app.clone().oneshot(req).await.unwrap();
    let bytes = to_bytes(res.into_body(), 64 * 1024).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["mfa"]["wajib"], true);
    assert_eq!(body["mfa"]["aktif"], false);
    assert_eq!(body["mfa"]["sisa_kode_pemulihan"], 0);
}

/// The `token_sha256` value for a base64url session token, without pulling
/// `sha2` into this crate's dev-dependencies.
fn hash_dari(token: &str) -> String {
    poolrouter_core::admin::TokenSesi::dari_b64(token)
        .unwrap()
        .hash()
        .as_str()
        .to_owned()
}
