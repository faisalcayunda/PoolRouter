//! `listProviders`, `getProvider`, `createProvider`, `updateProvider` over HTTP
//! against a real PostgreSQL (ARC-49; AC-1..AC-7, AC-12..AC-15). Run by
//! `make -C backend uji-pg`; `#[ignore]`d because it needs
//! `POOLROUTER_TEST_DATABASE_URL`.
//!
//! The point of this file is versioning and validation: an edit must leave the
//! old version byte-identical (V-03), and every 422 must roll the whole
//! transaction back rather than leave a provider row behind. Reads of the
//! version table use the superuser URL, because `pr_admin_api` can insert and
//! select but the test needs to compare bytes across versions. Every value here
//! is synthetic.

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
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const ASAL: &str = "https://ui.example.test";
const PROVIDERS: &str = "/admin/v1/providers";
/// The same synthetic KEK as the secret test.
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
    /// Secret names this test created, dropped again by [`Uji::bersihkan_secret`].
    dibuat: std::sync::Mutex<Vec<String>>,
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

    /// A secret that exists, so `{{secret.<nama>}}` validates.
    ///
    /// Secrets are global (AD1/AD4, ARC-48): `provider_secret` has no
    /// `provider_id`. The envelope columns carry the lengths the table's CHECK
    /// constraints require (12-byte nonce, 60-byte wrapped DEK), so the row goes
    /// in through the superuser connection without tripping them.
    ///
    /// Every name is recorded so [`Uji`] can drop it again: `admin_secret_pg`
    /// asserts the table is globally empty after a rejected body, and this file
    /// runs before it in the same database.
    async fn secret(&self, nama: &str, pembuat: Uuid) {
        sqlx::query(
            "INSERT INTO provider_secret (nama_secret, secret_ciphertext, \
             secret_dek_terbungkus, secret_nonce, kek_id, diubah_oleh) \
             VALUES ($1, '\\x01'::bytea, decode(repeat('00', 60), 'hex'), \
             decode(repeat('00', 12), 'hex'), 'k', $2)",
        )
        .bind(nama)
        .bind(pembuat)
        .execute(&self.su)
        .await
        .unwrap();
        self.dibuat.lock().unwrap().push(nama.to_owned());
    }

    /// Drop the secrets this test created, so the next binary starts from the
    /// state it expects. Failure to clean up is not the test's subject.
    async fn bersihkan_secret(&self) {
        let nama: Vec<String> = self.dibuat.lock().unwrap().drain(..).collect();
        for n in nama {
            let _ = sqlx::query("DELETE FROM provider_secret WHERE nama_secret = $1")
                .bind(n)
                .execute(&self.su)
                .await;
        }
    }

    /// The stored `dokumen` of one version, as text, for byte comparison.
    async fn dokumen(&self, provider_id: Uuid, versi: i32) -> Option<String> {
        sqlx::query_scalar(
            "SELECT dokumen::text FROM provider_config_versi \
             WHERE provider_id = $1 AND versi = $2",
        )
        .bind(provider_id)
        .bind(versi)
        .fetch_optional(&self.su)
        .await
        .unwrap()
    }

    async fn jumlah_provider(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM provider")
            .fetch_one(&self.su)
            .await
            .unwrap()
    }

    /// Audit rows for a provider. `nilai_lama` is NULL on the first insert and
    /// `nilai_baru` is NULL on a delete, so both are read as `Option`.
    async fn audit(
        &self,
        provider_id: Uuid,
    ) -> Vec<(String, String, Option<Value>, Option<Value>)> {
        sqlx::query_as(
            "SELECT aksi, entitas_jenis, nilai_lama, nilai_baru FROM audit_konfigurasi \
             WHERE entitas_jenis = 'provider' AND entitas_id = $1 ORDER BY terjadi_at",
        )
        .bind(provider_id)
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
    )
    .dengan_jam(Arc::new(t0));
    let app = router(state).layer(MockConnectInfo(SocketAddr::from(([203, 0, 113, 9], 4000))));
    Uji {
        app,
        pool,
        su,
        dibuat: std::sync::Mutex::new(Vec::new()),
    }
}

fn nama_unik(dasar: &str) -> String {
    // Secret names are held to `^[A-Za-z][A-Za-z0-9_]{0,63}$`, which rules out
    // the hyphen, so the separator is `_` and both kinds of fixture stay valid.
    // A process counter keeps names apart under `--test-threads=1`, where two
    // `now_v7()` calls in the same millisecond would otherwise collide.
    static URUTAN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = URUTAN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!(
        "{dasar}_{n:04}_{}",
        &Uuid::now_v7().simple().to_string()[..8]
    )
}

/// A valid provider body. `secret` names one stored secret and `parameter`
/// lists the names the template may reference.
fn body(nama: &str, secret: &str, parameter: &[&str]) -> Value {
    let params: Vec<Value> = parameter
        .iter()
        .map(|p| serde_json::json!({ "nama": p }))
        .collect();
    serde_json::json!({
        "nama": nama,
        "metode": "GET",
        "base_url": "https://dukcapil.example.go.id/api",
        "body_template": format!(
            r#"{{"NIK":"{{{{input.NAMA_LGKP}}}}","UID":"{{{{secret.{secret}}}}}"}}"#
        ),
        "parameters": params,
        "respons_skema": { "status_skema": "status_skor" },
        "tls_trust": "ca_kustom",
        "log_mode": "metadata_only",
        "config_schema_version": 1
    })
}

fn no_store(h: &HeaderMap) {
    assert_eq!(h[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
#[ignore]
async fn create_lalu_get_dan_list() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let s = nama_unik("SECRET");
    u.secret(&s, uid).await;
    let nama = nama_unik("dukcapil");

    // AC-1: mounted, answers 201, not 501.
    let (st, h, b) = u
        .kirim(
            Method::POST,
            PROVIDERS,
            Some(&pa),
            Some(body(&nama, &s, &["NAMA_LGKP"])),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    no_store(&h); // AC-2
    let id = b["provider_id"].as_str().unwrap().to_owned();
    assert_eq!(b["provider_config_version"], 1);
    assert_eq!(b["nama"], nama);
    assert_eq!(b["metode"], "GET", "D1: GET carries the body");
    assert_eq!(b["tls_trust"], "ca_kustom");
    assert_eq!(b["log_mode"], "metadata_only");
    assert_eq!(b["config_schema_version"], 1);

    // AC-13: the template is stored verbatim, names included.
    let tpl = b["body_template"].as_str().unwrap();
    assert!(tpl.contains("{{input.NAMA_LGKP}}"), "{tpl}");
    assert!(tpl.contains(&format!("{{{{secret.{s}}}}}")), "{tpl}");

    let (st, h, b) = u
        .kirim(Method::GET, &format!("{PROVIDERS}/{id}"), Some(&pa), None)
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert_eq!(b["provider_id"].as_str().unwrap(), id);

    let (st, _, b) = u.kirim(Method::GET, PROVIDERS, Some(&pa), None).await;
    assert_eq!(st, StatusCode::OK);
    let daftar = b.as_array().expect("array");
    assert!(
        daftar
            .iter()
            .any(|p| p["provider_id"].as_str() == Some(&id))
    );
    u.bersihkan_secret().await;
}

#[tokio::test]
#[ignore]
async fn create_menulis_baris_dan_versi_1_dalam_satu_transaksi() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let s = nama_unik("SECRET");
    u.secret(&s, uid).await;
    let nama = nama_unik("dukcapil");

    let (st, _, b) = u
        .kirim(
            Method::POST,
            PROVIDERS,
            Some(&pa),
            Some(body(&nama, &s, &["NAMA_LGKP"])),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    let id = Uuid::parse_str(b["provider_id"].as_str().unwrap()).unwrap();

    // AC-12: the pointer and the version row exist together.
    let (versi_aktif, dibuat_oleh): (i32, Uuid) = sqlx::query_as(
        "SELECT p.config_versi_aktif, v.dibuat_oleh FROM provider p \
         JOIN provider_config_versi v \
           ON v.provider_id = p.provider_id AND v.versi = p.config_versi_aktif \
         WHERE p.provider_id = $1",
    )
    .bind(id)
    .fetch_one(&u.su)
    .await
    .unwrap();
    assert_eq!(versi_aktif, 1);
    assert_eq!(dibuat_oleh, uid);

    // AC-5 shape: one audit row, entity `provider`, action `buat`.
    let audit = u.audit(id).await;
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].0, "buat");
    assert_eq!(audit[0].1, "provider");
    assert!(audit[0].2.is_none(), "create has no old value");

    // The log_mode column is the AD5 spelling.
    let log_mode: String =
        sqlx::query_scalar("SELECT log_mode FROM provider_config_versi WHERE provider_id = $1")
            .bind(id)
            .fetch_one(&u.su)
            .await
            .unwrap();
    assert_eq!(log_mode, "metadata_only");
    u.bersihkan_secret().await;
}

#[tokio::test]
#[ignore]
async fn update_menambah_versi_dan_versi_lama_utuh() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let s = nama_unik("SECRET");
    u.secret(&s, uid).await;
    let nama = nama_unik("dukcapil");

    let (st, _, b) = u
        .kirim(
            Method::POST,
            PROVIDERS,
            Some(&pa),
            Some(body(&nama, &s, &["NAMA_LGKP"])),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    let id = Uuid::parse_str(b["provider_id"].as_str().unwrap()).unwrap();
    let sebelum = u.dokumen(id, 1).await.expect("versi 1 ada");

    // Change the base URL; `alasan` is required on update (DESIGN.md §3).
    let mut ubah = body(&nama, &s, &["NAMA_LGKP", "TRESHOLD"]);
    ubah["base_url"] = Value::String("https://dukcapil2.example.go.id/api".into());
    ubah["body_template"] =
        Value::String(r#"{"NIK":"{{input.NAMA_LGKP}}","T":"{{endpoint.TRESHOLD}}"}"#.into());
    ubah["alasan"] = Value::String("menambah parameter TRESHOLD sintetis".into());

    let (st, h, b) = u
        .kirim(
            Method::PUT,
            &format!("{PROVIDERS}/{id}"),
            Some(&pa),
            Some(ubah),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    no_store(&h);
    assert_eq!(b["provider_config_version"], 2);
    assert_eq!(b["base_url"], "https://dukcapil2.example.go.id/api");

    // V-03: version 1 is byte-identical before and after.
    let sesudah = u.dokumen(id, 1).await.expect("versi 1 masih ada");
    assert_eq!(sebelum, sesudah, "versi lama tidak boleh berubah");
    assert!(u.dokumen(id, 2).await.is_some(), "versi 2 tersimpan");

    // The pointer moved to 2.
    let aktif: i32 =
        sqlx::query_scalar("SELECT config_versi_aktif FROM provider WHERE provider_id = $1")
            .bind(id)
            .fetch_one(&u.su)
            .await
            .unwrap();
    assert_eq!(aktif, 2);

    // Two audit rows: `buat` then `ubah`, and the second carries old and new.
    let audit = u.audit(id).await;
    assert_eq!(audit.len(), 2);
    assert_eq!(audit[1].0, "ubah");
    assert_eq!(audit[1].2.as_ref().unwrap()["versi"], 1);
    assert_eq!(audit[1].3.as_ref().unwrap()["versi"], 2);
    u.bersihkan_secret().await;
}

#[tokio::test]
#[ignore]
async fn kurung_tidak_seimbang_422_dengan_posisi_dan_tanpa_baris() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let s = nama_unik("SECRET");
    u.secret(&s, uid).await;
    let nama = nama_unik("dukcapil");
    let awal = u.jumlah_provider().await;

    let mut b = body(&nama, &s, &["NAMA_LGKP"]);
    // AC-13: one closing brace.
    b["body_template"] = Value::String(r#"{"NIK":"{{tanggal_lahir}"}"#.into());

    let (st, h, prob) = u.kirim(Method::POST, PROVIDERS, Some(&pa), Some(b)).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{prob}");
    no_store(&h);
    assert_eq!(prob["kode"], "validasi_gagal");
    let errors = prob["errors"].as_array().expect("errors");
    let e = errors
        .iter()
        .find(|e| e["kode"] == "template_tidak_seimbang")
        .expect("template_tidak_seimbang");
    assert_eq!(e["field"], "/body_template");
    assert!(e["posisi"]["baris"].as_i64().unwrap() >= 1);
    assert!(e["posisi"]["kolom"].as_i64().unwrap() >= 1);

    // Nothing was written.
    assert_eq!(u.jumlah_provider().await, awal);
    u.bersihkan_secret().await;
}

#[tokio::test]
#[ignore]
async fn secret_yang_tidak_ada_422_dengan_namanya() {
    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"]).await;
    let nama = nama_unik("dukcapil");
    let hilang = nama_unik("TIDAK_ADA");

    let (st, _, prob) = u
        .kirim(
            Method::POST,
            PROVIDERS,
            Some(&pa),
            Some(body(&nama, &hilang, &["NAMA_LGKP"])),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{prob}");
    let errors = prob["errors"].as_array().expect("errors");
    let e = errors
        .iter()
        .find(|e| e["kode"] == "template_secret_tidak_ada")
        .expect("template_secret_tidak_ada");
    // The name is echoed so the UI can offer to create it — never a value.
    assert_eq!(e["field"], "/body_template");
    assert!(
        prob.to_string().contains(&hilang),
        "nama secret harus muncul: {prob}"
    );
}

#[tokio::test]
#[ignore]
async fn parameter_tidak_ada_di_template_422() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let s = nama_unik("SECRET");
    u.secret(&s, uid).await;
    let nama = nama_unik("dukcapil");

    let mut b = body(&nama, &s, &["NAMA_LGKP"]);
    // TRESHOLD is referenced but not declared.
    b["body_template"] = Value::String(r#"{"T":"{{endpoint.TRESHOLD}}"}"#.into());

    let (st, _, prob) = u.kirim(Method::POST, PROVIDERS, Some(&pa), Some(b)).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{prob}");
    let errors = prob["errors"].as_array().expect("errors");
    assert!(
        errors
            .iter()
            .any(|e| e["kode"] == "template_parameter_tidak_ada"),
        "{prob}"
    );
    u.bersihkan_secret().await;
}

#[tokio::test]
#[ignore]
async fn config_schema_version_selain_satu_422() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let s = nama_unik("SECRET");
    u.secret(&s, uid).await;
    let nama = nama_unik("dukcapil");

    let mut b = body(&nama, &s, &["NAMA_LGKP"]);
    b["config_schema_version"] = Value::from(2);

    let (st, _, prob) = u.kirim(Method::POST, PROVIDERS, Some(&pa), Some(b)).await;
    // `maximum: 1` in the contract catches it before the handler; either way it
    // is a 422 and nothing is stored.
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{prob}");
    u.bersihkan_secret().await;
}

#[tokio::test]
#[ignore]
async fn field_tak_dikenal_422() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let s = nama_unik("SECRET");
    u.secret(&s, uid).await;
    let nama = nama_unik("dukcapil");

    let mut b = body(&nama, &s, &["NAMA_LGKP"]);
    b["field_karangan"] = Value::from(1);

    let (st, _, prob) = u.kirim(Method::POST, PROVIDERS, Some(&pa), Some(b)).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{prob}");
    u.bersihkan_secret().await;
}

#[tokio::test]
#[ignore]
async fn regex_tidak_bisa_dikompilasi_422() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let s = nama_unik("SECRET");
    u.secret(&s, uid).await;
    let nama = nama_unik("dukcapil");

    let mut b = body(&nama, &s, &["NAMA_LGKP"]);
    b["parameters"] = serde_json::json!([
        { "nama": "NAMA_LGKP", "rule": { "jenis": "regex", "pola": "([a-z" } }
    ]);

    let (st, _, prob) = u.kirim(Method::POST, PROVIDERS, Some(&pa), Some(b)).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{prob}");
    assert!(
        prob["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kode"] == "format"),
        "{prob}"
    );
    u.bersihkan_secret().await;
}

#[tokio::test]
#[ignore]
async fn rule_validasi_kembali_sama_persis() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let s = nama_unik("SECRET");
    u.secret(&s, uid).await;
    let nama = nama_unik("dukcapil");

    let mut b = body(&nama, &s, &["NAMA_LGKP", "TRESHOLD", "JENIS"]);
    b["parameters"] = serde_json::json!([
        { "nama": "NAMA_LGKP", "rule": { "jenis": "regex", "pola": "^[A-Z ]{3,100}$" } },
        { "nama": "TRESHOLD", "nilai": "95", "rule": { "jenis": "panjang", "panjang_min": 1, "panjang_maks": 3 } },
        { "nama": "JENIS", "rule": { "jenis": "enum", "nilai_diizinkan": ["L", "P"] } }
    ]);

    let (st, _, dibuat) = u
        .kirim(Method::POST, PROVIDERS, Some(&pa), Some(b.clone()))
        .await;
    assert_eq!(st, StatusCode::CREATED, "{dibuat}");
    let id = dibuat["provider_id"].as_str().unwrap().to_owned();

    // AC-15: read back identical, including the regex and the bounds.
    let (st, _, dibaca) = u
        .kirim(Method::GET, &format!("{PROVIDERS}/{id}"), Some(&pa), None)
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        dibaca["parameters"], b["parameters"],
        "rule validasi harus bolak-balik sama persis"
    );
    u.bersihkan_secret().await;
}

#[tokio::test]
#[ignore]
async fn tanpa_sesi_401_dan_peran_salah_403() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let (_, auditor) = u.pengguna(&["auditor"]).await;
    let s = nama_unik("SECRET");
    u.secret(&s, uid).await;
    let nama = nama_unik("dukcapil");

    // AC-3: no session.
    let (st, _, prob) = u
        .kirim(
            Method::POST,
            PROVIDERS,
            None,
            Some(body(&nama, &s, &["NAMA_LGKP"])),
        )
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_eq!(prob["kode"], "tidak_terautentikasi");

    // AC-4: a role outside the PA column is refused.
    let (st, _, prob) = u
        .kirim(
            Method::POST,
            PROVIDERS,
            Some(&auditor),
            Some(body(&nama, &s, &["NAMA_LGKP"])),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert_eq!(prob["kode"], "akses_ditolak");

    // The PA call still works, and only one row exists for this name.
    let (st, _, b) = u
        .kirim(
            Method::POST,
            PROVIDERS,
            Some(&pa),
            Some(body(&nama, &s, &["NAMA_LGKP"])),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    u.bersihkan_secret().await;
}

#[tokio::test]
#[ignore]
async fn nama_duplikat_422_bukan_500() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let s = nama_unik("SECRET");
    u.secret(&s, uid).await;
    let nama = nama_unik("dukcapil");

    let (st, _, _) = u
        .kirim(
            Method::POST,
            PROVIDERS,
            Some(&pa),
            Some(body(&nama, &s, &["NAMA_LGKP"])),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED);

    let (st, _, prob) = u
        .kirim(
            Method::POST,
            PROVIDERS,
            Some(&pa),
            Some(body(&nama, &s, &["NAMA_LGKP"])),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{prob}");
    let errors = prob["errors"].as_array().expect("errors");
    assert!(errors.iter().any(|e| e["field"] == "/nama"), "{prob}");
    u.bersihkan_secret().await;
}

#[tokio::test]
#[ignore]
async fn provider_tidak_ada_404() {
    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"]).await;
    let id = Uuid::now_v7();

    let (st, h, prob) = u
        .kirim(Method::GET, &format!("{PROVIDERS}/{id}"), Some(&pa), None)
        .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    no_store(&h);
    assert_eq!(prob["kode"], "tidak_ditemukan");

    let mut b = body(&nama_unik("x"), "S", &["NAMA_LGKP"]);
    b["alasan"] = Value::String("percobaan ubah provider sintetis".into());
    let (st, _, _) = u
        .kirim(
            Method::PUT,
            &format!("{PROVIDERS}/{id}"),
            Some(&pa),
            Some(b),
        )
        .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore]
async fn update_tanpa_alasan_422() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let s = nama_unik("SECRET");
    u.secret(&s, uid).await;
    let nama = nama_unik("dukcapil");

    let (st, _, b) = u
        .kirim(
            Method::POST,
            PROVIDERS,
            Some(&pa),
            Some(body(&nama, &s, &["NAMA_LGKP"])),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    let id = b["provider_id"].as_str().unwrap().to_owned();

    // No `alasan` on update (DESIGN.md §3 requires it).
    let (st, _, prob) = u
        .kirim(
            Method::PUT,
            &format!("{PROVIDERS}/{id}"),
            Some(&pa),
            Some(body(&nama, &s, &["NAMA_LGKP"])),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{prob}");
    assert!(
        prob["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["field"] == "/alasan"),
        "{prob}"
    );

    // Still version 1.
    let versi: i32 =
        sqlx::query_scalar("SELECT config_versi_aktif FROM provider WHERE provider_id = $1")
            .bind(Uuid::parse_str(&id).unwrap())
            .fetch_one(&u.su)
            .await
            .unwrap();
    assert_eq!(versi, 1);
    u.bersihkan_secret().await;
}

#[tokio::test]
#[ignore]
async fn base_url_bukan_https_422() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let s = nama_unik("SECRET");
    u.secret(&s, uid).await;
    let nama = nama_unik("dukcapil");

    let mut b = body(&nama, &s, &["NAMA_LGKP"]);
    b["base_url"] = Value::String("http://dukcapil.example.go.id/api".into());

    let (st, _, prob) = u.kirim(Method::POST, PROVIDERS, Some(&pa), Some(b)).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{prob}");
    assert!(
        prob["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["field"] == "/base_url"),
        "{prob}"
    );
    u.bersihkan_secret().await;
}
