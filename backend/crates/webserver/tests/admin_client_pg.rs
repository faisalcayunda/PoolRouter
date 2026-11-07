//! Client, credential, webhook and allowlist over HTTP against a real
//! PostgreSQL (f0-admin-api AC-19–AC-24). Run by `make -C backend uji-pg`;
//! `#[ignore]`d because it needs `POOLROUTER_TEST_DATABASE_URL`.
//!
//! Reading the envelope columns goes through `POOLROUTER_TEST_DATABASE_URL_SU`
//! because `pr_admin_api` is denied SELECT on them (AC-22). Every value here is
//! synthetic.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use chrono::{DateTime, Duration, TimeZone, Utc};
use poolrouter_core::admin::sesi_store::SesiStore;
use poolrouter_core::admin::{AsalUi, TokenSesi};
use poolrouter_core::kredensial::{cocok_secret, cocok_secret_klien, urai_api_key};
use poolrouter_core::secret::{Amplop, Brankas};
use poolrouter_webserver::admin::pembatas::PembatasLogin;
use poolrouter_webserver::admin::sesi::{AdminState, COOKIE_SESI, router};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
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
        origin: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        if let Some(o) = origin {
            req = req.header(header::ORIGIN, o);
        }
        let req = match body {
            Some(b) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(b.to_string()))
                .unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        let res = self.app.clone().oneshot(req).await.unwrap();
        let st = res.status();
        let h = res.headers().clone();
        let b = to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        let j: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
        (st, h, j)
    }

    /// A user with `peran` and a fresh session cookie. `client_owner` needs the
    /// `client_id` column set before its role row exists (trigger), so pass it.
    async fn pengguna(&self, peran: &[&str], client_id: Option<Uuid>) -> (Uuid, String) {
        let uid = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO admin_user (pengguna_id, username, password_hash, client_id) \
             VALUES ($1, $2, 'x', $3)",
        )
        .bind(uid)
        .bind(format!("sintetis-{}", uid.simple()))
        .bind(client_id)
        .execute(&self.pool)
        .await
        .unwrap();
        for p in peran {
            sqlx::query(
                "INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh, disetujui_oleh) \
                 VALUES ($1, $2, $1, $1)",
            )
            .bind(uid)
            .bind(p)
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

    async fn klien(&self, nama: &str) -> Uuid {
        let cid = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO client (client_id, nama, limiter_penolakan_burst, \
                    limiter_penolakan_isi_ulang_per_detik) VALUES ($1, $2, 20, 1.0)",
        )
        .bind(cid)
        .bind(nama)
        .execute(&self.pool)
        .await
        .unwrap();
        cid
    }

    async fn allowlist(&self, domain: &str, oleh: Uuid) {
        sqlx::query(
            "INSERT INTO webhook_domain_allowlist (domain, aktif, diubah_oleh) VALUES ($1, true, $2)",
        )
        .bind(domain)
        .bind(oleh)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn audit_terakhir(&self, entitas: &str) -> Option<Value> {
        sqlx::query_scalar(
            "SELECT jsonb_build_object('aksi', aksi, 'nilai_baru', nilai_baru, 'alasan', alasan) \
               FROM audit_konfigurasi WHERE entitas_jenis = $1 ORDER BY terjadi_at DESC LIMIT 1",
        )
        .bind(entitas)
        .fetch_optional(&self.pool)
        .await
        .unwrap()
    }
}

async fn siapkan() -> Uji {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::ERROR)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .try_init();
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

fn unik(dasar: &str) -> String {
    format!("{dasar}-{}", &Uuid::now_v7().simple().to_string()[..10])
}

fn no_store(h: &HeaderMap) {
    assert_eq!(
        h[header::CACHE_CONTROL],
        "no-store",
        "Cache-Control no-store"
    );
}

/// AC-19 (+AC-2, AC-3): client CRUD, limiter ranges, reason required on limiter
/// change, CO only its own client.
#[tokio::test]
#[ignore]
async fn klien_crud_dan_batas_limiter() {
    let u = siapkan().await;
    let (pa_id, pa) = u.pengguna(&["platform_admin"], None).await;

    // AC-3: no session → 401 and no-store.
    let (st, h, j) = u
        .kirim(Method::GET, "/admin/v1/clients", None, None, None)
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_eq!(j["kode"], "tidak_terautentikasi");
    no_store(&h);

    let nama = unik("Klien");
    let (st, h, j) = u
        .kirim(
            Method::POST,
            "/admin/v1/clients",
            Some(&pa),
            Some(ASAL),
            Some(json!({ "nama": nama, "limiter_penolakan_burst": 0 })),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(j["errors"][0]["field"], "limiter_penolakan_burst");
    no_store(&h);

    let (st, _, j) = u
        .kirim(
            Method::POST,
            "/admin/v1/clients",
            Some(&pa),
            Some(ASAL),
            Some(json!({ "nama": nama, "limiter_penolakan_isi_ulang_per_detik": 0.05 })),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        j["errors"][0]["field"],
        "limiter_penolakan_isi_ulang_per_detik"
    );

    let (st, h, dibuat) = u
        .kirim(
            Method::POST,
            "/admin/v1/clients",
            Some(&pa),
            Some(ASAL),
            Some(json!({
                "nama": nama,
                "limiter_penolakan_burst": 500,
                "limiter_penolakan_isi_ulang_per_detik": 50.0
            })),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED);
    no_store(&h);
    let cid = Uuid::parse_str(dibuat["client_id"].as_str().unwrap()).unwrap();
    assert_eq!(dibuat["limiter_penolakan_burst"], 500);

    // PA sees it.
    let (st, _, daftar) = u
        .kirim(
            Method::GET,
            "/admin/v1/clients",
            Some(&pa),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        daftar
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["client_id"] == *cid.to_string().as_str())
    );

    // A CO with its own client sees only that one, even if others exist.
    let lain = u.klien(&unik("KlienLain")).await;
    let (_, co) = u.pengguna(&["client_owner"], Some(cid)).await;
    let (st, _, daftar) = u
        .kirim(
            Method::GET,
            "/admin/v1/clients",
            Some(&co),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    let arr = daftar.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["client_id"], *cid.to_string().as_str());
    assert!(
        !arr.iter()
            .any(|c| c["client_id"] == *lain.to_string().as_str())
    );
    // getClient on somebody else's client → 404, not 403.
    let (st, _, _) = u
        .kirim(
            Method::GET,
            &format!("/admin/v1/clients/{lain}"),
            Some(&co),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    // CO cannot write.
    let (st, _, _) = u
        .kirim(
            Method::PUT,
            &format!("/admin/v1/clients/{cid}"),
            Some(&co),
            Some(ASAL),
            Some(json!({ "nama": "NamaLain" })),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Changing the limiter without a reason → 422 alasan.
    let (st, _, j) = u
        .kirim(
            Method::PUT,
            &format!("/admin/v1/clients/{cid}"),
            Some(&pa),
            Some(ASAL),
            Some(json!({ "nama": nama, "limiter_penolakan_burst": 100 })),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(j["errors"][0]["field"], "alasan");

    // With a reason → 200 and one audit row carrying the reason.
    let (st, h, diubah) = u
        .kirim(
            Method::PUT,
            &format!("/admin/v1/clients/{cid}"),
            Some(&pa),
            Some(ASAL),
            Some(json!({
                "nama": nama,
                "limiter_penolakan_burst": 100,
                "alasan": "Beban naik setelah uji sintetis."
            })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert_eq!(diubah["limiter_penolakan_burst"], 100);
    let audit = u.audit_terakhir("client").await.expect("audit client");
    assert_eq!(audit["aksi"], "ubah");
    assert_eq!(audit["alasan"], "Beban naik setelah uji sintetis.");
    assert_eq!(audit["nilai_baru"]["limiter_penolakan_burst"], 100);
    let _ = pa_id;
}

/// AC-20: secret returned once, only the hash stored, third active credential
/// → 409, expiry over 365 days → 422, list without hash.
#[tokio::test]
#[ignore]
async fn kredensial_sekali_dan_batas_aktif() {
    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"], None).await;
    let cid = u.klien(&unik("KlienKred")).await;

    let (st, h, pertama) = u
        .kirim(
            Method::POST,
            &format!("/admin/v1/clients/{cid}/credentials"),
            Some(&pa),
            Some(ASAL),
            Some(json!({ "jenis": "api_key" })),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED);
    no_store(&h);
    let kunci = pertama["secret"].as_str().unwrap();
    let terurai = urai_api_key(kunci).expect("format pr_<id>_<secret>");
    // The id in the wire format is the credential id.
    assert_eq!(
        terurai.credential_id.to_string(),
        pertama["credential_id"].as_str().unwrap()
    );
    // Stored hash opens with the secret part only.
    let hash: Vec<u8> =
        sqlx::query_scalar("SELECT kunci_hash FROM client_kredensial WHERE credential_id = $1")
            .bind(Uuid::parse_str(pertama["credential_id"].as_str().unwrap()).unwrap())
            .fetch_one(&u.su)
            .await
            .unwrap();
    assert!(cocok_secret(terurai.secret, &hash), "hash API key cocok");
    assert!(!kunci.contains("kunci_hash"));

    // oauth2 → base64url 256-bit secret, argon2id hash.
    let (st, _, kedua) = u
        .kirim(
            Method::POST,
            &format!("/admin/v1/clients/{cid}/credentials"),
            Some(&pa),
            Some(ASAL),
            Some(json!({ "jenis": "oauth2" })),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED);
    let rahasia = kedua["secret"].as_str().unwrap();
    assert_eq!(rahasia.len(), 43, "base64url 256 bit tanpa padding");
    let phc: Vec<u8> =
        sqlx::query_scalar("SELECT kunci_hash FROM client_kredensial WHERE credential_id = $1")
            .bind(Uuid::parse_str(kedua["credential_id"].as_str().unwrap()).unwrap())
            .fetch_one(&u.su)
            .await
            .unwrap();
    assert!(cocok_secret_klien(rahasia, &phc), "argon2id cocok");
    assert!(String::from_utf8_lossy(&phc).starts_with("$argon2id$"));

    // Third active → 409 batas_kredensial (trigger).
    let (st, _, j) = u
        .kirim(
            Method::POST,
            &format!("/admin/v1/clients/{cid}/credentials"),
            Some(&pa),
            Some(ASAL),
            Some(json!({ "jenis": "api_key" })),
        )
        .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(j["kode"], "batas_kredensial");

    // Expiry beyond 365 days → 422.
    let jauh = (Utc::now() + Duration::days(366)).to_rfc3339();
    let cid2 = u.klien(&unik("KlienKred2")).await;
    let (st, _, j) = u
        .kirim(
            Method::POST,
            &format!("/admin/v1/clients/{cid2}/credentials"),
            Some(&pa),
            Some(ASAL),
            Some(json!({ "jenis": "api_key", "kedaluwarsa_at": jauh })),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(j["errors"][0]["field"], "kedaluwarsa_at");

    // List has no hash field.
    let (st, h, daftar) = u
        .kirim(
            Method::GET,
            &format!("/admin/v1/clients/{cid}/credentials"),
            Some(&pa),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    let arr = daftar.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    for baris in arr {
        assert!(baris.get("kunci_hash").is_none());
        assert!(baris.get("secret").is_none());
        assert_eq!(baris["status"], "aktif");
    }
}

/// Connection pool that runs every statement as `pr_api`, the role the token
/// endpoint authenticates under. `pr_admin_api` (the admin API's own role)
/// deliberately cannot read `client_kredensial.kunci_hash`.
async fn api_pool(su: &PgPool) -> PgPool {
    let opsi = su.connect_options().as_ref().clone();
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("SET ROLE pr_api").execute(conn).await?;
                Ok(())
            })
        })
        .connect_with(opsi)
        .await
        .expect("database pr_api")
}

/// AC-20: the secret `createCredential` hands back once is what the token
/// endpoint actually accepts — the stored hash is the value `client_secret`
/// verifies, not just something that hashes the same way.
#[tokio::test]
#[ignore]
async fn kredensial_dipakai_token_endpoint() {
    use poolrouter_webserver::downstream::auth::Authenticator;
    use poolrouter_webserver::downstream::credential::PgCredentialStore;
    use poolrouter_webserver::downstream::jwt::JwtKeys;

    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"], None).await;
    let cid = u.klien(&unik("KlienToken")).await;

    let (st, _, dibuat) = u
        .kirim(
            Method::POST,
            &format!("/admin/v1/clients/{cid}/credentials"),
            Some(&pa),
            Some(ASAL),
            Some(json!({ "jenis": "oauth2" })),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED);
    let rahasia = dibuat["secret"].as_str().unwrap().to_owned();

    // The real store and verifier the `POST /v1/oauth/token` path uses.
    let kunci = Arc::new({
        use aws_lc_rs::rand::SystemRandom;
        use aws_lc_rs::signature::Ed25519KeyPair;
        let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let b = poolrouter_core::kredensial::base64url_encode(doc.as_ref())
            .replace('-', "+")
            .replace('_', "/");
        let pem = format!("-----BEGIN PRIVATE KEY-----\n{b}\n-----END PRIVATE KEY-----\n");
        JwtKeys::from_pem("https://poolrouter.test", &pem, None).expect("kunci uji")
    });
    let pengesah = Authenticator::new(
        Arc::new(PgCredentialStore::new(api_pool(&u.su).await)),
        kunci,
    );

    let (token_client, _credential) = pengesah
        .client_secret(&cid.to_string(), &rahasia)
        .await
        .expect("secret dari createCredential diterima token endpoint");
    assert_eq!(token_client, cid);

    // A wrong secret for the same client is refused.
    assert!(
        pengesah
            .client_secret(&cid.to_string(), "salah-sekali")
            .await
            .is_err(),
        "secret salah ditolak"
    );
}

/// AC-21 (+AC-22): webhook config only https, host must be allowlisted, 404
/// before it is configured; rotate returns the secret once, versi 1 then 2, and
/// `pr_admin_api` cannot read the envelope.
#[tokio::test]
#[ignore]
async fn webhook_konfigurasi_rotate_dan_amplop() {
    let u = siapkan().await;
    let (pa_id, pa) = u.pengguna(&["platform_admin"], None).await;
    let cid = u.klien(&unik("KlienHook")).await;
    let jalur = format!("/admin/v1/clients/{cid}/webhook");

    // AC-21: not configured yet → 404.
    let (st, h, _) = u
        .kirim(Method::GET, &jalur, Some(&pa), Some(ASAL), None)
        .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    no_store(&h);

    // http → 422 webhook_tidak_https.
    let (st, _, j) = u
        .kirim(
            Method::PUT,
            &jalur,
            Some(&pa),
            Some(ASAL),
            Some(json!({ "webhook_url": "http://contoh.test/hook", "aktif": true })),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(j["kode"], "webhook_tidak_https");

    // Host not allowlisted → 422 webhook_domain_tidak_diizinkan.
    let domain = unik("hook").replace('-', "x");
    let (st, _, j) = u
        .kirim(
            Method::PUT,
            &jalur,
            Some(&pa),
            Some(ASAL),
            Some(json!({ "webhook_url": format!("https://{domain}.test/hook"), "aktif": true })),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(j["kode"], "webhook_domain_tidak_diizinkan");

    u.allowlist(&format!("{domain}.test"), pa_id).await;

    let (st, h, simpan) = u
        .kirim(
            Method::PUT,
            &jalur,
            Some(&pa),
            Some(ASAL),
            Some(json!({ "webhook_url": format!("https://{domain}.test/hook"), "aktif": true })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert_eq!(simpan["webhook_url"], format!("https://{domain}.test/hook"));
    assert_eq!(simpan["secret_versi"], Value::Null, "belum ada secret");

    // AC-22: first rotation → versi 1, whsec_ + base64 32 byte.
    let (st, h, rot1) = u
        .kirim(
            Method::POST,
            &format!("{jalur}/rotasi-secret"),
            Some(&pa),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert_eq!(rot1["secret_versi"], 1);
    let secret1 = rot1["secret"].as_str().unwrap();
    assert!(secret1.starts_with("whsec_"));
    let isi = secret1.trim_start_matches("whsec_");
    let byte = poolrouter_core::kredensial::base64url_decode(isi)
        .or_else(|| base64_standard_decode(isi))
        .expect("base64 sah");
    assert_eq!(byte.len(), 32);

    // `pr_admin_api` cannot read the envelope columns (AC-22).
    let ditolak = sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT secret_ciphertext FROM client_webhook WHERE client_id = $1",
    )
    .bind(cid)
    .fetch_one(&u.pool)
    .await;
    assert!(
        ditolak.is_err(),
        "pr_admin_api tidak boleh SELECT ciphertext"
    );

    // The envelope opens with the shared KEK and equals the response.
    let (ciphertext, dek_terbungkus, nonce, kek_id): (Vec<u8>, Vec<u8>, Vec<u8>, String) =
        sqlx::query_as(
            "SELECT secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id \
               FROM client_webhook WHERE client_id = $1",
        )
        .bind(cid)
        .fetch_one(&u.su)
        .await
        .unwrap();
    let brankas = Brankas::dari_isi_kek(KEK_HEX).unwrap();
    let mut n = [0u8; 12];
    n.copy_from_slice(&nonce);
    let amplop = Amplop {
        nama: format!("client_webhook {cid} v1"),
        ciphertext,
        dek_terbungkus,
        nonce: n,
        kek_id,
    };
    let buka = brankas.dekripsi(&amplop).unwrap();
    assert_eq!(buka.expose().as_str(), secret1);

    // Second rotation → versi 2 and a different secret.
    let (st, _, rot2) = u
        .kirim(
            Method::POST,
            &format!("{jalur}/rotasi-secret"),
            Some(&pa),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(rot2["secret_versi"], 2);
    assert_ne!(rot1["secret"], rot2["secret"]);

    // getWebhookConfig now carries the secret version, never the secret.
    let (st, _, lihat) = u
        .kirim(Method::GET, &jalur, Some(&pa), Some(ASAL), None)
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(lihat["secret_versi"], 2);
    assert!(lihat.get("secret").is_none());
}

fn base64_standard_decode(text: &str) -> Option<Vec<u8>> {
    // The generator uses standard base64 (with `+`, `/`, `=`), so try that
    // alphabet after the base64url helper has failed.
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bit = 0u32;
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        buf = (buf << 6) | v as u32;
        bit += 6;
        if bit >= 8 {
            bit -= 8;
            out.push((buf >> bit) as u8);
        }
    }
    Some(out)
}

/// AC-23: webhook log is empty before any delivery; CO reads only its own
/// client, a foreign client is 404.
#[tokio::test]
#[ignore]
async fn log_webhook_kosong_dan_batas_klien() {
    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"], None).await;
    let cid = u.klien(&unik("KlienLog")).await;
    let (_, co) = u.pengguna(&["client_owner"], Some(cid)).await;
    let jalur = format!("/admin/v1/clients/{cid}/webhook/log");

    let (st, h, log) = u
        .kirim(Method::GET, &jalur, Some(&co), Some(ASAL), None)
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert!(log.as_array().unwrap().is_empty());

    let (st, _, log) = u
        .kirim(Method::GET, &jalur, Some(&pa), Some(ASAL), None)
        .await;
    assert_eq!(st, StatusCode::OK);
    assert!(log.as_array().unwrap().is_empty());

    // CO asking for another client's log → 404.
    let lain = u.klien(&unik("KlienLog2")).await;
    let (st, _, _) = u
        .kirim(
            Method::GET,
            &format!("/admin/v1/clients/{lain}/webhook/log"),
            Some(&co),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

/// AC-24: allowlist list/put, delete idempotent (204 even when gone), PA only.
#[tokio::test]
#[ignore]
async fn allowlist_put_dan_hapus_idempoten() {
    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"], None).await;
    let domain = unik("izin").replace('-', "y");

    // PUT creates, 200 with the entry.
    let (st, h, entri) = u
        .kirim(
            Method::PUT,
            &format!("/admin/v1/webhook-allowlist/{domain}.test"),
            Some(&pa),
            Some(ASAL),
            Some(json!({ "aktif": true })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert_eq!(entri["domain"], format!("{domain}.test"));
    assert_eq!(entri["aktif"], true);

    // List contains it.
    let (st, _, daftar) = u
        .kirim(
            Method::GET,
            "/admin/v1/webhook-allowlist",
            Some(&pa),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        daftar
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["domain"] == *format!("{domain}.test").as_str())
    );

    // DELETE removes, 204; a second DELETE is still 204.
    let (st, h, _) = u
        .kirim(
            Method::DELETE,
            &format!("/admin/v1/webhook-allowlist/{domain}.test"),
            Some(&pa),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    no_store(&h);
    let (st, _, _) = u
        .kirim(
            Method::DELETE,
            &format!("/admin/v1/webhook-allowlist/{domain}.test"),
            Some(&pa),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // CO is not allowed to touch the global list.
    let cid = u.klien(&unik("KlienIzin")).await;
    let (_, co) = u.pengguna(&["client_owner"], Some(cid)).await;
    let (st, _, _) = u
        .kirim(
            Method::GET,
            "/admin/v1/webhook-allowlist",
            Some(&co),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}
