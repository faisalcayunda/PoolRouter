//! `listSecrets`, `putSecret`, `deleteSecret` over HTTP against a real
//! PostgreSQL (ARC-48; AC-1..AC-11). Run by `make -C backend uji-pg`;
//! `#[ignore]`d because it needs `POOLROUTER_TEST_DATABASE_URL`.
//!
//! Reads of the secret row itself go through `POOLROUTER_TEST_DATABASE_URL_SU`:
//! `pr_admin_api` cannot select the envelope columns, which is the point of AC-8,
//! so the test proves the stored bytes open with the KEK and contain no
//! plaintext from the outside, as superuser. Every value here is synthetic.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use chrono::{DateTime, TimeZone, Utc};
use poolrouter_core::admin::sesi_store::SesiStore;
use poolrouter_core::admin::{AsalUi, TokenSesi};
use poolrouter_core::secret::{Amplop, Brankas};
use poolrouter_webserver::admin::pembatas::PembatasLogin;
use poolrouter_webserver::admin::sesi::{AdminState, COOKIE_SESI, router};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const ASAL: &str = "https://ui.example.test";

/// Synthetic KEK, 64 hex characters: the form `Brankas::dari_isi_kek` accepts
/// from the secret file. Never a real key.
const KEK_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

const KUNCI_HMAC: &[u8] = b"kunci-hmac-sintetis-untuk-uji";

const SECRETS: &str = "/admin/v1/secrets";

fn redis_ujian() -> String {
    std::env::var("POOLROUTER_TEST_REDIS")
        .expect("POOLROUTER_TEST_REDIS belum diset; jalankan lewat `make -C backend uji-pg`")
}

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 5, 9, 0, 0).unwrap()
}

/// One app instance plus both connections: `pr_admin_api` (what handlers get)
/// and the superuser (only for reading audit rows and the envelope columns).
struct Uji {
    app: Router,
    pool: PgPool,
    su: PgPool,
}

type BarisAmplop = (Vec<u8>, Vec<u8>, Vec<u8>, String);

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
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (
            status,
            headers,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// A user with the given roles and one live session; returns the cookie.
    /// `auditor`/`operator` need the separation-of-duties triple the schema
    /// enforces (`peran_pemisahan_tugas`: proposer, approver and holder are three
    /// different people), so those roles get two extra synthetic users.
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
                "INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh, disetujui_oleh)                  VALUES ($1, $2, coalesce($3, $1), $4)",
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

    /// An active provider whose current configuration mentions `{{secret.<nama>}}`.
    async fn provider_pemakai(&self, nama: &str, pembuat: Uuid) -> Uuid {
        let pid = Uuid::now_v7();
        sqlx::query("INSERT INTO provider (provider_id, nama) VALUES ($1, $2)")
            .bind(pid)
            .bind(format!("prov-{}", pid.simple()))
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO provider_config_versi \
             (provider_id, versi, config_schema_version, log_mode, dokumen, dibuat_oleh) \
             VALUES ($1, 1, 1, 'metadata_only', $2::jsonb, $3)",
        )
        .bind(pid)
        .bind(format!(
            r#"{{"template_body": "{{{{\"NIK\":\"{{{{secret.{nama}}}}}\"}}}}"}}"#
        ))
        .bind(pembuat)
        .execute(&self.pool)
        .await
        .unwrap();
        pid
    }

    /// The stored envelope, rebuilt into the type the cipher module verifies.
    async fn amplop(&self, nama: &str) -> Option<Amplop> {
        let baris: Option<BarisAmplop> = sqlx::query_as(
            "SELECT secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id \
             FROM provider_secret WHERE nama_secret = $1",
        )
        .bind(nama)
        .fetch_optional(&self.su)
        .await
        .unwrap();
        baris.map(|(ciphertext, dek_terbungkus, nonce, kek_id)| Amplop {
            nama: nama.to_owned(),
            ciphertext,
            dek_terbungkus,
            nonce: nonce.as_slice().try_into().expect("nonce 12 byte"),
            kek_id,
        })
    }

    async fn versi(&self, nama: &str) -> Option<i32> {
        sqlx::query_scalar("SELECT secret_versi FROM provider_secret WHERE nama_secret = $1")
            .bind(nama)
            .fetch_optional(&self.su)
            .await
            .unwrap()
    }

    async fn audit_konfigurasi(&self, nama: &str) -> Vec<(String, String, Value, Value, String)> {
        // `nilai_baru`/`nilai_lama` describe the change; they carry the version,
        // never the value.
        sqlx::query_as(
            "SELECT aksi, entitas_jenis, nilai_lama, nilai_baru, alasan FROM audit_konfigurasi \
             WHERE entitas_jenis = 'provider_secret' AND (nilai_baru->>'nama' = $1 \
               OR nilai_lama->>'nama' = $1) ORDER BY terjadi_at",
        )
        .bind(nama)
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
    Uji { app, pool, su }
}

/// A name unique to this run, shaped like the contract regex.
fn nama_unik(dasar: &str) -> String {
    format!("{dasar}_{}", &Uuid::now_v7().simple().to_string()[..8])
}

fn jalur(nama: &str) -> String {
    format!("{SECRETS}/{nama}")
}

fn isi(nilai: &str) -> Value {
    serde_json::json!({ "nilai": nilai })
}

fn no_store(h: &HeaderMap) {
    assert_eq!(h[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
#[ignore]
async fn put_list_delete_putaran_penuh() {
    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"]).await;
    let nama = nama_unik("SINTETIS_PW");
    let nilai = "rahasia-sintetis-pertama";

    // AC-1: the operation is mounted and answers 204, not 501.
    let (st, h, _) = u
        .kirim(
            Method::PUT,
            &jalur(&nama),
            Some(&pa),
            Some(ASAL),
            Some(isi(nilai)),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    no_store(&h);

    // AC-8/AC-11: the row holds an envelope that opens with the KEK, has a fresh
    // nonce, and matches the KEK id of this deployment.
    let brankas = Brankas::dari_isi_kek(KEK_HEX).unwrap();
    let amplop = u.amplop(&nama).await.expect("baris tersimpan");
    assert_eq!(amplop.kek_id, brankas.kek_id(), "kek_id tercatat (AC-8)");
    assert_eq!(amplop.dek_terbungkus.len(), 12 + 32 + 16);
    assert_eq!(
        brankas.dekripsi(&amplop).unwrap().expose(),
        nilai,
        "amplop terbuka dengan KEK"
    );
    assert!(
        !amplop
            .ciphertext
            .windows(nilai.len())
            .any(|w| w == nilai.as_bytes()),
        "ciphertext tidak memuat plaintext"
    );
    assert_eq!(u.versi(&nama).await, Some(1));

    // AC-9: list answers name, masking and usage count, never the value.
    let (st, h, j) = u.kirim(Method::GET, SECRETS, Some(&pa), None, None).await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    let baris = j
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["nama"] == nama.as_str())
        .expect("secret ada di daftar")
        .clone();
    assert_eq!(baris["tersamar"], true);
    assert_eq!(baris["dipakai_oleh_provider_count"], 0);
    assert!(
        j.to_string().find(nilai).is_none(),
        "nilai tidak pernah dikirim"
    );

    // AC-11: a second write bumps the version and re-seals with a new nonce.
    let (st, _, _) = u
        .kirim(
            Method::PUT,
            &jalur(&nama),
            Some(&pa),
            Some(ASAL),
            Some(isi("rahasia-sintetis-kedua")),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let baru = u.amplop(&nama).await.expect("baris tersimpan");
    assert_eq!(u.versi(&nama).await, Some(2));
    assert_ne!(baru.nonce, amplop.nonce, "nonce baru");
    assert_ne!(baru.ciphertext, amplop.ciphertext);
    assert_eq!(baru.dek_terbungkus.len(), 60);

    // AC-10: a secret nobody uses can be deleted.
    let (st, h, _) = u
        .kirim(Method::DELETE, &jalur(&nama), Some(&pa), Some(ASAL), None)
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    no_store(&h);
    assert_eq!(u.versi(&nama).await, None);
}

#[tokio::test]
#[ignore]
async fn put_nama_tidak_sah_422_tanpa_menyimpan() {
    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"]).await;

    for (nama, kode) in [
        ("1MULAI_ANGKA", "format"),
        ("ADA-SPASI", "format"),
        ("ADA.SPASI", "format"),
        ("", "format"),
        (
            Box::leak("A".repeat(65).into_boxed_str()) as &str,
            "rentang",
        ),
    ] {
        let (st, h, j) = u
            .kirim(
                Method::PUT,
                &jalur(nama),
                Some(&pa),
                Some(ASAL),
                Some(isi("sintetis")),
            )
            .await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "nama {nama:?}");
        no_store(&h);
        assert_eq!(j["kode"], "validasi_gagal");
        assert_eq!(j["errors"][0]["field"], "nama");
        assert_eq!(j["errors"][0]["kode"], kode, "nama {nama:?}");
        assert!(
            j["errors"][0].get("posisi").is_none(),
            "422 secret bukan galat template"
        );
        assert!(
            j.to_string().find("sintetis").is_none(),
            "nilai tidak dipantulkan"
        );
    }

    // AC-7: a body that is not `{"nilai": ...}` is a validation failure too.
    for badan in [
        serde_json::json!({}),
        serde_json::json!({ "nilai": "x", "lain": 1 }),
    ] {
        let (st, _, j) = u
            .kirim(
                Method::PUT,
                &jalur(&nama_unik("BENTUK")),
                Some(&pa),
                Some(ASAL),
                Some(badan),
            )
            .await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(j["kode"], "validasi_gagal");
    }

    let jumlah: i64 = sqlx::query_scalar("SELECT count(*) FROM provider_secret")
        .fetch_one(&u.su)
        .await
        .unwrap();
    assert_eq!(
        jumlah, 0,
        "tidak ada baris yang tersimpan dari badan tidak sah"
    );
}

#[tokio::test]
#[ignore]
async fn tanpa_sesi_401_di_tiga_operasi() {
    let u = siapkan().await;
    let nama = nama_unik("TANPA_SESI");
    for (method, path, badan) in [
        (Method::GET, SECRETS.to_owned(), None),
        (Method::PUT, jalur(&nama), Some(isi("sintetis"))),
        (Method::DELETE, jalur(&nama), None),
    ] {
        let (st, h, j) = u
            .kirim(method.clone(), &path, None, Some(ASAL), badan)
            .await;
        assert_eq!(st, StatusCode::UNAUTHORIZED, "{method} {path}");
        no_store(&h);
        assert_eq!(j["kode"], "tidak_terautentikasi");
    }
}

#[tokio::test]
#[ignore]
async fn peran_salah_403_dan_tepat_satu_baris_audit() {
    let u = siapkan().await;
    let (uid, auditor) = u.pengguna(&["auditor"]).await;
    let nama = nama_unik("PERAN_SALAH");

    for (method, path, badan) in [
        (Method::GET, SECRETS.to_owned(), None),
        (Method::PUT, jalur(&nama), Some(isi("sintetis"))),
        (Method::DELETE, jalur(&nama), None),
    ] {
        let (st, h, j) = u
            .kirim(method.clone(), &path, Some(&auditor), Some(ASAL), badan)
            .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{method} {path}");
        no_store(&h);
        assert_eq!(j["kode"], "akses_ditolak");
    }

    // AC-4: one row per refused request, with the caller, the role and the origin.
    let baris: Vec<(String, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT peran, aksi, hasil, sumber_ip, sesi_id FROM audit_akses \
         WHERE pengguna_id = $1 ORDER BY terjadi_at",
    )
    .bind(uid)
    .fetch_all(&u.su)
    .await
    .unwrap();
    assert_eq!(baris.len(), 3, "tiga penolakan, tiga baris (AC-4)");
    for (peran, aksi, hasil, ip, sesi) in baris {
        assert_eq!(peran, "auditor");
        assert_eq!(aksi, "akses_ditolak");
        assert_eq!(hasil, "ditolak");
        assert_eq!(ip, "203.0.113.9");
        assert!(sesi.is_some(), "sesi penolakan tercatat");
    }

    // Nothing was written and no configuration audit row appeared.
    assert_eq!(u.versi(&nama).await, None);
    assert!(u.audit_konfigurasi(&nama).await.is_empty());
}

#[tokio::test]
#[ignore]
async fn origin_asing_403_pada_tulis_tanpa_baris_audit() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let nama = nama_unik("ORIGIN");

    for origin in [Some("https://jahat.example"), Some("null"), None] {
        let (st, _, j) = u
            .kirim(
                Method::PUT,
                &jalur(&nama),
                Some(&pa),
                origin,
                Some(isi("sintetis")),
            )
            .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "origin {origin:?}");
        assert_eq!(j["kode"], "origin_tidak_valid");
    }
    // AC-3: the origin check comes first, so the session row and the trail stay untouched.
    let baris: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_akses WHERE pengguna_id = $1")
        .bind(uid)
        .fetch_one(&u.su)
        .await
        .unwrap();
    assert_eq!(baris, 0, "origin asing tidak menulis baris audit");
    assert_eq!(u.versi(&nama).await, None);

    // A GET has no origin requirement, so the same caller can list.
    let (st, _, _) = u
        .kirim(
            Method::GET,
            SECRETS,
            Some(&pa),
            Some("https://jahat.example"),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK);
}

#[tokio::test]
#[ignore]
async fn delete_dipakai_409_lalu_204_setelah_provider_nonaktif() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let nama = nama_unik("DIPAKAI");
    let (st, _, _) = u
        .kirim(
            Method::PUT,
            &jalur(&nama),
            Some(&pa),
            Some(ASAL),
            Some(isi("sintetis")),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let pid = u.provider_pemakai(&nama, uid).await;

    // AC-9: the usage count sees the provider.
    let (_, _, j) = u.kirim(Method::GET, SECRETS, Some(&pa), None, None).await;
    let baris = j
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["nama"] == nama.as_str())
        .expect("secret ada")
        .clone();
    assert_eq!(baris["dipakai_oleh_provider_count"], 1);

    // AC-10: still in use, so 409 with the provider name in `detail`.
    let (st, h, j) = u
        .kirim(Method::DELETE, &jalur(&nama), Some(&pa), Some(ASAL), None)
        .await;
    assert_eq!(st, StatusCode::CONFLICT);
    no_store(&h);
    assert_eq!(j["kode"], "secret_dipakai");
    let detail = j["detail"].as_str().expect("detail ada");
    assert!(
        detail.contains("prov-"),
        "detail menyebut provider: {detail}"
    );
    assert!(!detail.contains("sintetis"), "detail tanpa nilai secret");
    assert_eq!(u.versi(&nama).await, Some(1), "baris tidak terhapus");

    // Once the provider is inactive, the same delete goes through. Status flips
    // go through the audited function: `pr_admin_api` has no column grant for it.
    sqlx::query("SELECT fn_nonaktifkan_entitas('provider', $1, $2, $3)")
        .bind(pid)
        .bind(uid)
        .bind("uji ARC-48: provider sintetis dinonaktifkan")
        .execute(&u.pool)
        .await
        .unwrap();
    let (st, _, _) = u
        .kirim(Method::DELETE, &jalur(&nama), Some(&pa), Some(ASAL), None)
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(u.versi(&nama).await, None);
}

#[tokio::test]
#[ignore]
async fn delete_tidak_ada_404() {
    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"]).await;
    let (st, h, j) = u
        .kirim(
            Method::DELETE,
            &jalur(&nama_unik("TIDAK_ADA")),
            Some(&pa),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    no_store(&h);
    assert_eq!(j["kode"], "tidak_ditemukan");
}

#[tokio::test]
#[ignore]
async fn audit_konfigurasi_mencatat_pelaku_dan_versi() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let nama = nama_unik("AUDIT");

    for nilai in ["pertama", "kedua"] {
        let (st, _, _) = u
            .kirim(
                Method::PUT,
                &jalur(&nama),
                Some(&pa),
                Some(ASAL),
                Some(isi(nilai)),
            )
            .await;
        assert_eq!(st, StatusCode::NO_CONTENT);
    }
    let (_, _, _) = u
        .kirim(Method::DELETE, &jalur(&nama), Some(&pa), Some(ASAL), None)
        .await;

    // AC-5: who, when, entity, action and the version before/after.
    let baris: Vec<(Uuid, String, String, Option<Value>, Value, String)> = sqlx::query_as(
        "SELECT pengguna_id, aksi, entitas_jenis, nilai_lama, nilai_baru, alasan \
         FROM audit_konfigurasi WHERE entitas_jenis = 'provider_secret' \
           AND (nilai_baru->>'nama' = $1 OR nilai_lama->>'nama' = $1) ORDER BY terjadi_at",
    )
    .bind(&nama)
    .fetch_all(&u.su)
    .await
    .unwrap();
    assert_eq!(baris.len(), 3, "put, put, delete");
    assert!(baris.iter().all(|(pelaku, ..)| *pelaku == uid));
    assert_eq!(baris[0].1, "buat");
    assert_eq!(baris[1].1, "ubah");
    assert_eq!(baris[2].1, "cabut");
    assert!(
        baris
            .iter()
            .all(|(_, _, entitas, ..)| entitas == "provider_secret")
    );
    assert_eq!(baris[0].4["nama"], nama.as_str(), "nama di nilai_baru");
    assert_eq!(baris[0].4["secret_versi"], 1);
    assert_eq!(
        baris[1].3.as_ref().expect("nilai_lama")["secret_versi"],
        1,
        "versi lama pada ubah"
    );
    assert_eq!(baris[1].4["secret_versi"], 2, "versi baru pada ubah");
    assert_eq!(
        baris[2].3.as_ref().expect("nilai_lama")["secret_versi"],
        2,
        "versi yang dicabut"
    );
    assert!(
        baris
            .iter()
            .all(|(_, _, _, _, _, alasan)| alasan.contains(&nama)),
        "alasan menyebut operasi dan nama"
    );
    assert!(
        !serde_json::to_string(&baris).unwrap().contains("pertama"),
        "audit tidak memuat nilai secret"
    );
}

/// AC-6: the value never reaches the logs — not through the handler, not through
/// `Debug` of the envelope, not through an error path.
#[tokio::test]
#[ignore]
async fn nilai_secret_tidak_muncul_di_log() {
    use std::io::Write as _;
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

    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"]).await;
    let (_, auditor) = u.pengguna(&["auditor"]).await;
    let nama = nama_unik("LOG");
    let nilai = "sangat-rahasia-sintetis-9f2c";

    let (st, _, _) = u
        .kirim(
            Method::PUT,
            &jalur(&nama),
            Some(&pa),
            Some(ASAL),
            Some(isi(nilai)),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    // A refusal logs a line, which also proves the capture works.
    let (st, _, _) = u
        .kirim(
            Method::PUT,
            &jalur(&nama),
            Some(&auditor),
            Some(ASAL),
            Some(isi(nilai)),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let ciphertext = u.amplop(&nama).await.expect("baris").ciphertext;
    let heks = ciphertext
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let log = String::from_utf8(penyangga.0.lock().unwrap().clone()).unwrap();
    assert!(!log.is_empty(), "penangkap log bekerja");
    assert!(!log.contains(nilai), "nilai tidak muncul di log");
    assert!(!log.contains(&heks), "ciphertext tidak muncul di log");

    // Clean up the row and the log buffer assertion state for the next test.
    let _ = u
        .kirim(Method::DELETE, &jalur(&nama), Some(&pa), Some(ASAL), None)
        .await;
    let mut b = penyangga.0.lock().unwrap();
    b.clear();
    let _ = b.flush();
}
