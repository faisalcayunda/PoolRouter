//! K-20 client and credential handlers over HTTP against a real PostgreSQL
//! (f0-admin-api AC-28..AC-31, ARC-167). Run by `make -C backend uji-pg`;
//! `#[ignore]`d because it needs `POOLROUTER_TEST_DATABASE_URL`. The
//! transition/audit math itself is proven at the service layer in
//! `admin_k20_pg.rs`; this file proves the HTTP wiring: role gates, 404s,
//! `no-store`, and the response shapes `PratinjauK20`/`HasilK20`/`Client`/
//! `Credential` the contract promises. Every NIK, payload and reason below
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
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const ASAL: &str = "https://ui.example.test";
const KEK_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const KUNCI_HMAC: &[u8] = b"kunci-hmac-sintetis-untuk-uji-k20";

fn redis_ujian() -> String {
    std::env::var("POOLROUTER_TEST_REDIS")
        .expect("POOLROUTER_TEST_REDIS belum diset; jalankan lewat `make -C backend uji-pg`")
}

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 7, 9, 0, 0).unwrap()
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

    async fn kredensial(&self, client_id: Uuid, oleh: Uuid) -> Uuid {
        let kid = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, dibuat_oleh) \
             VALUES ($1, $2, 'api_key', '\\x01', $3)",
        )
        .bind(kid)
        .bind(client_id)
        .bind(oleh)
        .execute(&self.su)
        .await
        .unwrap();
        kid
    }

    /// A downstream endpoint routed through its own provider/pool/upstream, so
    /// a queued request can reference it without depending on seed data.
    async fn endpoint(&self, tag: &str) -> Uuid {
        let provider = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO provider (provider_id, nama, data_balikan_aktif) VALUES ($1, $2, false)",
        )
        .bind(provider)
        .bind(format!("prov-k20-http-{tag}"))
        .execute(&self.su)
        .await
        .unwrap();
        let upstream = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, kuota_harian, batas_percobaan, retry_backoff_base_detik) \
             VALUES ($1, $2, $3, 1000, 3, 1)",
        )
        .bind(upstream)
        .bind(provider)
        .bind(format!("up-k20-http-{tag}"))
        .execute(&self.su)
        .await
        .unwrap();
        let dpool = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) VALUES ($1, $2, 1)",
        )
        .bind(dpool)
        .bind(format!("down-k20-http-{tag}"))
        .execute(&self.su)
        .await
        .unwrap();
        sqlx::query("INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id) VALUES ($1, $2)")
            .bind(dpool)
            .bind(upstream)
            .execute(&self.su)
            .await
            .unwrap();
        let endpoint = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, downstream_pool_id) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(endpoint)
        .bind(format!("ep-k20-http-{tag}"))
        .bind(provider)
        .bind(dpool)
        .execute(&self.su)
        .await
        .unwrap();
        endpoint
    }

    async fn audit_terakhir(&self, entitas: &str, id: Uuid) -> Option<Value> {
        sqlx::query_scalar(
            "SELECT jsonb_build_object('aksi', aksi, 'nilai_baru', nilai_baru, 'alasan', alasan, \
                        'jumlah_cancelled', jumlah_cancelled, 'jumlah_failed', jumlah_failed) \
               FROM audit_konfigurasi WHERE entitas_jenis = $1 AND entitas_id = $2 \
               ORDER BY terjadi_at DESC LIMIT 1",
        )
        .bind(entitas)
        .bind(id)
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
    let app = router(state).layer(MockConnectInfo(SocketAddr::from(([203, 0, 113, 10], 4000))));
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

/// One `queued` request, `attempt_count = 0`, against its own endpoint.
async fn request_queued(
    su: &PgPool,
    client_id: Uuid,
    credential_id: Uuid,
    endpoint: Uuid,
    dpool: Uuid,
    nik: &str,
) -> Uuid {
    let rid = Uuid::now_v7();
    let mut tx = su.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, downstream_pool_id, \
            provider_id, provider_config_version, endpoint_config_version, mode, prioritas, prioritas_efektif, \
            status, input_payload, metadata, received_at, ttl_expires_at, attempt_count) \
         SELECT $1, $2, $3, $4, $5, provider_id, 1, 1, 'async', 1, 1, 'queued', $6, '{}'::jsonb, $7, $7 + interval '1 day', 0 \
           FROM downstream_endpoint WHERE downstream_endpoint_id = $4",
    )
    .bind(rid)
    .bind(client_id)
    .bind(credential_id)
    .bind(endpoint)
    .bind(dpool)
    .bind(serde_json::json!({ "NIK": nik }))
    .bind(Utc::now())
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) \
         VALUES ($1, 1, NULL, 'queued', 'webserver')",
    )
    .bind(rid)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    rid
}

/// AC-28/AC-29/AC-30: client preview/deactivate/activate over HTTP, PA only.
#[tokio::test]
#[ignore]
async fn client_pratinjau_nonaktifkan_aktifkan() {
    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"], None).await;
    let cid = u.klien(&unik("KlienK20")).await;

    // AC-3: no session -> 401.
    let (st, h, _) = u
        .kirim(
            Method::GET,
            &format!("/admin/v1/clients/{cid}/pratinjau-nonaktif"),
            None,
            None,
            None,
        )
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    no_store(&h);

    // Unknown id -> 404.
    let hantu = Uuid::now_v7();
    let (st, _, _) = u
        .kirim(
            Method::GET,
            &format!("/admin/v1/clients/{hantu}/pratinjau-nonaktif"),
            Some(&pa),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // CO cannot preview a client's deactivation (fn_nonaktifkan_entitas has no
    // client_owner branch for jenis = 'client').
    let (_, co) = u.pengguna(&["client_owner"], Some(cid)).await;
    let (st, _, _) = u
        .kirim(
            Method::GET,
            &format!("/admin/v1/clients/{cid}/pratinjau-nonaktif"),
            Some(&co),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // No queued requests yet -> preview is all zero, AC-28 shape, no leakage fields.
    let (st, h, pratinjau) = u
        .kirim(
            Method::GET,
            &format!("/admin/v1/clients/{cid}/pratinjau-nonaktif"),
            Some(&pa),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert_eq!(pratinjau["akan_cancelled"], 0);
    assert_eq!(pratinjau["akan_failed"], 0);
    assert!(pratinjau.get("dihitung_pada").is_some());
    assert!(pratinjau.get("request_id").is_none());
    assert!(pratinjau.get("nik").is_none());

    // Deactivate with a too-short reason -> 422 alasan (K20Service::nonaktifkan
    // validates length before calling fn_nonaktifkan_entitas).
    let (st, _, j) = u
        .kirim(
            Method::POST,
            &format!("/admin/v1/clients/{cid}/nonaktifkan"),
            Some(&pa),
            Some(ASAL),
            Some(json!({ "alasan": "pendek" })),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(j["errors"][0]["field"], "alasan");

    // AC-29: deactivate with a reason -> 200 HasilK20, one audit row carrying it.
    let alasan = "uji HTTP K20 client, cakupan ARC-167";
    let (st, h, hasil) = u
        .kirim(
            Method::POST,
            &format!("/admin/v1/clients/{cid}/nonaktifkan"),
            Some(&pa),
            Some(ASAL),
            Some(json!({ "alasan": alasan })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert_eq!(hasil["jumlah_cancelled"], 0);
    assert_eq!(hasil["jumlah_failed"], 0);
    assert!(hasil.get("dilakukan_pada").is_some());
    let audit = u.audit_terakhir("client", cid).await.expect("audit client");
    assert_eq!(audit["aksi"], "nonaktifkan");
    assert_eq!(audit["alasan"], alasan);
    assert_eq!(audit["nilai_baru"]["aktif"], false);

    let aktif: bool = sqlx::query_scalar("SELECT aktif FROM client WHERE client_id = $1")
        .bind(cid)
        .fetch_one(&u.su)
        .await
        .unwrap();
    assert!(!aktif);

    // Deactivating again (already inactive) still succeeds (idempotent flip, no
    // waiting request this time): the SQL function does not require "was active".
    // AC-30: activate -> 200 entity, aktif = true.
    let (st, h, client) = u
        .kirim(
            Method::POST,
            &format!("/admin/v1/clients/{cid}/aktifkan"),
            Some(&pa),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert_eq!(client["aktif"], true);
    assert_eq!(client["client_id"], *cid.to_string().as_str());

    let aktif: bool = sqlx::query_scalar("SELECT aktif FROM client WHERE client_id = $1")
        .bind(cid)
        .fetch_one(&u.su)
        .await
        .unwrap();
    assert!(aktif);
}

/// AC-28/AC-29/AC-30/U-54b: credential preview/revoke over HTTP, CO can act on
/// its own client's credential, cross-client CO is refused with the same 404
/// as an unknown id, and reactivation always 409.
#[tokio::test]
#[ignore]
async fn credential_pratinjau_cabut_dan_aktifkan_selalu_409() {
    let u = siapkan().await;
    let (pa_id, pa) = u.pengguna(&["platform_admin"], None).await;
    let cid = u.klien(&unik("KlienKredK20")).await;
    let kid = u.kredensial(cid, pa_id).await;
    let endpoint = u.endpoint(&unik("kred")).await;
    let dpool: Uuid = sqlx::query_scalar(
        "SELECT downstream_pool_id FROM downstream_endpoint WHERE downstream_endpoint_id = $1",
    )
    .bind(endpoint)
    .fetch_one(&u.su)
    .await
    .unwrap();
    let nik = "0000000000000099";
    request_queued(&u.su, cid, kid, endpoint, dpool, nik).await;

    let (_, co) = u.pengguna(&["client_owner"], Some(cid)).await;
    let lain = u.klien(&unik("KlienLainK20")).await;
    let (_, co_lain) = u.pengguna(&["client_owner"], Some(lain)).await;

    // A different client's owner gets 404, same as an unknown credential.
    let (st, _, _) = u
        .kirim(
            Method::GET,
            &format!("/admin/v1/credentials/{kid}/pratinjau-cabut"),
            Some(&co_lain),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // The owner of this credential's own client previews it: AC-28 shape.
    let (st, h, pratinjau) = u
        .kirim(
            Method::GET,
            &format!("/admin/v1/credentials/{kid}/pratinjau-cabut"),
            Some(&co),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert_eq!(pratinjau["akan_cancelled"], 1);
    assert_eq!(pratinjau["akan_failed"], 0);

    // AC-29: revoke with a reason -> 200 Credential{status: dicabut}, one audit row.
    let alasan = "uji HTTP K20 kredensial, cakupan ARC-167";
    let (st, h, kredensial) = u
        .kirim(
            Method::POST,
            &format!("/admin/v1/credentials/{kid}/cabut"),
            Some(&co),
            Some(ASAL),
            Some(json!({ "alasan": alasan })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert_eq!(kredensial["status"], "dicabut");
    assert!(kredensial.get("dicabut_at").is_some());
    let audit = u
        .audit_terakhir("kredensial", kid)
        .await
        .expect("audit kredensial");
    assert_eq!(audit["aksi"], "cabut");
    assert_eq!(audit["alasan"], alasan);
    assert_eq!(audit["jumlah_cancelled"], 1);
    assert_eq!(audit["jumlah_failed"], 0);

    let status_request: String = sqlx::query_scalar(
        "SELECT status FROM request WHERE client_id = $1 AND credential_id = $2",
    )
    .bind(cid)
    .bind(kid)
    .fetch_one(&u.su)
    .await
    .unwrap();
    assert_eq!(status_request, "cancelled");

    // AC-6: the NIK never surfaces in the response bodies above.
    assert!(!kredensial.to_string().contains(nik));
    assert!(!pratinjau.to_string().contains(nik));

    // U-54b: reactivation always 409 kredensial_dicabut, PA only.
    let (st, h, j) = u
        .kirim(
            Method::POST,
            &format!("/admin/v1/credentials/{kid}/aktifkan"),
            Some(&pa),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::CONFLICT);
    no_store(&h);
    assert_eq!(j["kode"], "kredensial_dicabut");

    // CO cannot even attempt reactivation (fn_aktifkan_entitas is PA-only).
    let (st, _, _) = u
        .kirim(
            Method::POST,
            &format!("/admin/v1/credentials/{kid}/aktifkan"),
            Some(&co),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

/// AC-29 (client): HasilK20 == baris audit_konfigurasi, termasuk angka non-nol.
#[tokio::test]
#[ignore]
async fn client_hasil_k20_sama_dengan_audit() {
    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"], None).await;
    let cid = u.klien(&unik("KlienHasil")).await;
    let uid = u.pengguna(&["platform_admin"], None).await.0;
    let kid = u.kredensial(cid, uid).await;
    let endpoint = u.endpoint(&unik("ep")).await;
    let dpool: Uuid = sqlx::query_scalar(
        "SELECT downstream_pool_id FROM downstream_endpoint WHERE downstream_endpoint_id = $1",
    )
    .bind(endpoint)
    .fetch_one(&u.su)
    .await
    .unwrap();
    request_queued(&u.su, cid, kid, endpoint, dpool, "0000000000000077").await;

    let alasan = "uji kesamaan angka HasilK20 dengan audit";
    let (st, _, hasil) = u
        .kirim(
            Method::POST,
            &format!("/admin/v1/clients/{cid}/nonaktifkan"),
            Some(&pa),
            Some(ASAL),
            Some(json!({ "alasan": alasan })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(hasil["jumlah_cancelled"], 1);
    assert_eq!(hasil["jumlah_failed"], 0);

    let audit = u.audit_terakhir("client", cid).await.expect("audit client");
    assert_eq!(audit["aksi"], "nonaktifkan");
    assert_eq!(audit["jumlah_cancelled"], hasil["jumlah_cancelled"]);
    assert_eq!(audit["jumlah_failed"], hasil["jumlah_failed"]);
    assert_eq!(audit["alasan"], alasan);
}

/// AC-31: dua nonaktifkan bersamaan pada client yang sama -> satu transisi per
/// request, satu baris audit dengan angka bukan nol.
#[tokio::test]
#[ignore]
async fn client_dua_nonaktifkan_bersamaan_satu_transisi() {
    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"], None).await;
    let cid = u.klien(&unik("KlienParalel")).await;
    let uid = u.pengguna(&["platform_admin"], None).await.0;
    let kid = u.kredensial(cid, uid).await;
    let endpoint = u.endpoint(&unik("epp")).await;
    let dpool: Uuid = sqlx::query_scalar(
        "SELECT downstream_pool_id FROM downstream_endpoint WHERE downstream_endpoint_id = $1",
    )
    .bind(endpoint)
    .fetch_one(&u.su)
    .await
    .unwrap();
    request_queued(&u.su, cid, kid, endpoint, dpool, "0000000000000088").await;

    let alasan_a = "uji paralel client pertama sintetis";
    let alasan_b = "uji paralel client kedua sintetis";
    let app = u.app.clone();
    let pa_a = pa.clone();
    let pa_b = pa.clone();
    let path = format!("/admin/v1/clients/{cid}/nonaktifkan");
    let (ra, rb) = tokio::join!(
        async {
            let req = Request::builder()
                .method(Method::POST)
                .uri(path.clone())
                .header(header::COOKIE, pa_a.as_str())
                .header(header::ORIGIN, ASAL)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "alasan": alasan_a }).to_string()))
                .unwrap();
            app.clone().oneshot(req).await.unwrap()
        },
        async {
            let req = Request::builder()
                .method(Method::POST)
                .uri(path.clone())
                .header(header::COOKIE, pa_b.as_str())
                .header(header::ORIGIN, ASAL)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "alasan": alasan_b }).to_string()))
                .unwrap();
            app.clone().oneshot(req).await.unwrap()
        }
    );

    let mut ada_nonzero = false;
    for res in [ra, rb] {
        let b = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let j: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
        if j["jumlah_cancelled"].as_i64().unwrap_or(0) > 0 {
            ada_nonzero = true;
        }
    }
    assert!(
        ada_nonzero,
        "AC-31: minimal satu eksekusi mencatat cancel non-nol"
    );

    let status: String = sqlx::query_scalar("SELECT status FROM request WHERE client_id = $1")
        .bind(cid)
        .fetch_one(&u.su)
        .await
        .unwrap();
    assert_eq!(status, "cancelled");

    let jumlah_cancelled_history: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM request_status_history WHERE status_ke = 'cancelled' \
         AND request_id IN (SELECT request_id FROM request WHERE client_id = $1)",
    )
    .bind(cid)
    .fetch_one(&u.su)
    .await
    .unwrap();
    assert_eq!(
        jumlah_cancelled_history, 1,
        "request dicatat cancelled tepat sekali"
    );

    let jumlah_audit_nonzero: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_konfigurasi WHERE entitas_jenis = 'client' \
         AND entitas_id = $1 AND jumlah_cancelled > 0",
    )
    .bind(cid)
    .fetch_one(&u.pool)
    .await
    .unwrap();
    assert_eq!(
        jumlah_audit_nonzero, 1,
        "AC-31: satu baris audit dengan angka bukan nol"
    );
}

/// AC-1..AC-7: tracing tidak memuat NIK atau payload sintetis (pola admin_log_pg).
#[tokio::test]
#[ignore]
async fn k20_tracing_tidak_memuat_nik_atau_payload() {
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    struct Penyangga(Arc<Mutex<Vec<u8>>>);
    struct Penulis(Arc<Mutex<Vec<u8>>>);
    impl Write for Penulis {
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
            Penulis(self.0.clone())
        }
    }

    let penyangga = Penyangga::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(penyangga.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"], None).await;
    let cid = u.klien(&unik("KlienTrace")).await;
    let uid = u.pengguna(&["platform_admin"], None).await.0;
    let kid = u.kredensial(cid, uid).await;
    let endpoint = u.endpoint(&unik("eptrace")).await;
    let dpool: Uuid = sqlx::query_scalar(
        "SELECT downstream_pool_id FROM downstream_endpoint WHERE downstream_endpoint_id = $1",
    )
    .bind(endpoint)
    .fetch_one(&u.su)
    .await
    .unwrap();
    let nik = "0000000000000055";
    request_queued(&u.su, cid, kid, endpoint, dpool, nik).await;

    let (st, _, _) = u
        .kirim(
            Method::GET,
            &format!("/admin/v1/credentials/{kid}/pratinjau-cabut"),
            Some(&pa),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK);

    let (st, _, _) = u
        .kirim(
            Method::POST,
            &format!("/admin/v1/credentials/{kid}/cabut"),
            Some(&pa),
            Some(ASAL),
            Some(json!({ "alasan": "uji tracing leak sintetis" })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);

    let log = String::from_utf8(penyangga.0.lock().unwrap().clone()).unwrap();
    assert!(!log.is_empty(), "penangkap tracing dipasang");
    assert!(!log.contains(nik), "NIK tidak muncul di tracing");
}

/// AC-29 (credential): jumlah_cancelled/jumlah_failed di audit == angka yang
/// dihasilkan (cabutCredential tidak mengembalikan HasilK20 lewat HTTP).
#[tokio::test]
#[ignore]
async fn credential_audit_angka_non_nol() {
    let u = siapkan().await;
    let (pa_id, pa) = u.pengguna(&["platform_admin"], None).await;
    let cid = u.klien(&unik("KlienAudit")).await;
    let kid = u.kredensial(cid, pa_id).await;
    let endpoint = u.endpoint(&unik("epaudit")).await;
    let dpool: Uuid = sqlx::query_scalar(
        "SELECT downstream_pool_id FROM downstream_endpoint WHERE downstream_endpoint_id = $1",
    )
    .bind(endpoint)
    .fetch_one(&u.su)
    .await
    .unwrap();
    request_queued(&u.su, cid, kid, endpoint, dpool, "0000000000000066").await;

    let alasan = "uji audit angka kredensial non-nol";
    let (st, _, _) = u
        .kirim(
            Method::POST,
            &format!("/admin/v1/credentials/{kid}/cabut"),
            Some(&pa),
            Some(ASAL),
            Some(json!({ "alasan": alasan })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);

    let audit = u
        .audit_terakhir("kredensial", kid)
        .await
        .expect("audit kredensial");
    assert_eq!(audit["aksi"], "cabut");
    assert_eq!(audit["jumlah_cancelled"], 1);
    assert_eq!(audit["jumlah_failed"], 0);
    assert_eq!(audit["alasan"], alasan);
}

// ---------------------------------------------------------------------------
// ARC-168: endpoint, downstream pool, upstream pool, provider (AC-28..AC-31)
// ---------------------------------------------------------------------------

/// Satu rute lengkap beserta client dan kredensialnya.
struct Rute {
    client: Uuid,
    kredensial: Uuid,
    provider: Uuid,
    upstream: Uuid,
    dpool: Uuid,
    endpoint: Uuid,
}

async fn rute(u: &Uji, tag: &str) -> Rute {
    let (pengguna, _) = u.pengguna(&["platform_admin"], None).await;
    let client = u.klien(&unik(&format!("Klien{tag}"))).await;
    let kredensial = u.kredensial(client, pengguna).await;
    let endpoint = u.endpoint(&unik(tag)).await;
    let (provider, dpool): (Uuid, Uuid) = sqlx::query_as(
        "SELECT provider_id, downstream_pool_id FROM downstream_endpoint \
          WHERE downstream_endpoint_id = $1",
    )
    .bind(endpoint)
    .fetch_one(&u.su)
    .await
    .unwrap();
    let upstream: Uuid = sqlx::query_scalar(
        "SELECT upstream_pool_id FROM downstream_pool_rute WHERE downstream_pool_id = $1",
    )
    .bind(dpool)
    .fetch_one(&u.su)
    .await
    .unwrap();
    // Provider dan endpoint hasil API selalu punya versi aktif; `aktifkan`
    // membaca ulang entitasnya, jadi fixture ikut menulis versi 1.
    sqlx::query(
        "INSERT INTO provider_config_versi (provider_id, versi, config_schema_version, log_mode, dokumen, dibuat_oleh) \
         VALUES ($1, 1, 1, 'metadata_only', $2, $3)",
    )
    .bind(provider)
    .bind(json!({
        "metode": "GET",
        "base_url": "https://penyedia.example.test/api",
        "body_template": "{}",
        "parameters": [],
        "respons_skema": { "status_skema": "status_skor" },
        "tls_trust": "ca_kustom",
        "log_mode": "metadata_only"
    }))
    .bind(pengguna)
    .execute(&u.su)
    .await
    .unwrap();
    sqlx::query("UPDATE provider SET config_versi_aktif = 1 WHERE provider_id = $1")
        .bind(provider)
        .execute(&u.su)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO endpoint_config_versi (downstream_endpoint_id, versi, config_schema_version, dokumen, dibuat_oleh) \
         VALUES ($1, 1, 1, '{}'::jsonb, $2)",
    )
    .bind(endpoint)
    .bind(pengguna)
    .execute(&u.su)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE downstream_endpoint SET config_versi_aktif = 1 WHERE downstream_endpoint_id = $1",
    )
    .bind(endpoint)
    .execute(&u.su)
    .await
    .unwrap();
    Rute {
        client,
        kredensial,
        provider,
        upstream,
        dpool,
        endpoint,
    }
}

/// Request `queued` yang sudah satu kali dicoba (`attempt_count = 1`, upstream
/// pool terisi): menunggu retry, jadi `failed` saat objeknya dinonaktifkan.
async fn request_menunggu_retry(su: &PgPool, r: &Rute, nik: &str) -> Uuid {
    let rid = Uuid::now_v7();
    let mut tx = su.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, downstream_pool_id, \
            provider_id, upstream_pool_id, provider_config_version, endpoint_config_version, mode, prioritas, \
            prioritas_efektif, status, metadata, received_at, ttl_expires_at, attempt_count) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 1, 'async', 1, 1, 'queued', '{}'::jsonb, $8, $8 + interval '1 day', 1)",
    )
    .bind(rid)
    .bind(r.client)
    .bind(r.kredensial)
    .bind(r.endpoint)
    .bind(r.dpool)
    .bind(r.provider)
    .bind(r.upstream)
    .bind(Utc::now())
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) \
         VALUES ($1, 1, NULL, 'queued', 'webserver')",
    )
    .bind(rid)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, \
            client_id, downstream_endpoint_id, provider_config_version, endpoint_config_version, nik, requested_at, \
            status_akses, error_class, log_mode, worker_id) \
         VALUES (gen_random_uuid(), $1, 1, $2, $3, $4, $5, 1, 1, $6, now(), 'gagal_provider', 'provider_sementara', \
            'metadata_only', 'w1')",
    )
    .bind(rid)
    .bind(r.provider)
    .bind(r.upstream)
    .bind(r.client)
    .bind(r.endpoint)
    .bind(nik)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    rid
}

/// Request `dispatched` dengan `attempt_count = 0`: sudah punya upstream pool
/// (EN-04: terisi saat dispatch), jadi ikut dibatalkan untuk keempat tipe.
async fn request_dispatched(su: &PgPool, r: &Rute, nik: &str) -> Uuid {
    let rid = request_queued(su, r.client, r.kredensial, r.endpoint, r.dpool, nik).await;
    let mut tx = su.begin().await.unwrap();
    sqlx::query(
        "UPDATE request SET status = 'dispatched', dispatch_key = gen_random_uuid(), \
                dispatched_at = $2, upstream_pool_id = $3 WHERE request_id = $1",
    )
    .bind(rid)
    .bind(Utc::now())
    .bind(r.upstream)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) \
         VALUES ($1, 2, 'queued', 'dispatched', 'webserver')",
    )
    .bind(rid)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    rid
}

/// Request `in_flight`: tidak pernah disentuh K-20 (AC-29).
async fn request_in_flight(su: &PgPool, r: &Rute) -> Uuid {
    let rid = Uuid::now_v7();
    let mut tx = su.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, downstream_pool_id, \
            provider_id, provider_config_version, endpoint_config_version, mode, prioritas, prioritas_efektif, \
            status, metadata, received_at, attempt_count) \
         SELECT $1, $2, $3, $4, $5, provider_id, 1, 1, 'sync', 1, 1, 'in_flight', '{}'::jsonb, $6, 1 \
           FROM downstream_endpoint WHERE downstream_endpoint_id = $4",
    )
    .bind(rid)
    .bind(r.client)
    .bind(r.kredensial)
    .bind(r.endpoint)
    .bind(r.dpool)
    .bind(Utc::now())
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) \
         VALUES ($1, 1, NULL, 'in_flight', 'webserver')",
    )
    .bind(rid)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    rid
}

/// Satu baris matriks: satu tipe entitas.
struct Kasus {
    nama: &'static str,
    /// Segmen path koleksi, mis. `endpoints`.
    path: &'static str,
    /// `entitas_jenis` di `audit_konfigurasi`.
    entitas: &'static str,
    /// Nama field id di entitas yang dikembalikan `aktifkan`.
    kunci_id: &'static str,
    id: fn(&Rute) -> Uuid,
    aktif_sql: &'static str,
    /// Request `queued` tanpa upstream pool ikut dibatalkan? Tidak untuk
    /// upstream pool (EN-04: baru terikat saat dispatch).
    queued_terdampak: bool,
    alasan_cancel: &'static str,
}

const KASUS: [Kasus; 4] = [
    Kasus {
        nama: "endpoint",
        path: "endpoints",
        entitas: "downstream_endpoint",
        kunci_id: "endpoint_id",
        id: |r| r.endpoint,
        aktif_sql: "SELECT aktif FROM downstream_endpoint WHERE downstream_endpoint_id = $1",
        queued_terdampak: true,
        alasan_cancel: "endpoint_nonaktif",
    },
    Kasus {
        nama: "dpool",
        path: "downstream-pools",
        entitas: "downstream_pool",
        kunci_id: "downstream_pool_id",
        id: |r| r.dpool,
        aktif_sql: "SELECT aktif FROM downstream_pool WHERE downstream_pool_id = $1",
        queued_terdampak: true,
        alasan_cancel: "pool_nonaktif",
    },
    Kasus {
        nama: "upool",
        path: "upstream-pools",
        entitas: "upstream_pool",
        kunci_id: "upstream_pool_id",
        id: |r| r.upstream,
        aktif_sql: "SELECT aktif FROM upstream_pool WHERE upstream_pool_id = $1",
        queued_terdampak: false,
        alasan_cancel: "pool_nonaktif",
    },
    Kasus {
        nama: "provider",
        path: "providers",
        entitas: "provider",
        kunci_id: "provider_id",
        id: |r| r.provider,
        aktif_sql: "SELECT aktif FROM provider WHERE provider_id = $1",
        queued_terdampak: true,
        alasan_cancel: "provider_nonaktif",
    },
];

async fn status_request(su: &PgPool, rid: Uuid) -> (String, Option<String>, Option<String>) {
    sqlx::query_as(
        "SELECT status, cancel_reason, failure_reason FROM request WHERE request_id = $1",
    )
    .bind(rid)
    .fetch_one(su)
    .await
    .unwrap()
}

/// Matriks satu tipe entitas lewat router produksi: guard, 404, pratinjau,
/// nonaktifkan (status request, audit, `in_flight`), idempotensi, aktifkan.
async fn matriks(k: &Kasus) {
    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"], None).await;
    let r = rute(&u, k.nama).await;
    let id = (k.id)(&r);
    let dasar = format!("/admin/v1/{}/{id}", k.path);
    let nik = "0000000000000042";

    // AC-3: tanpa sesi 401, Origin salah 403 tanpa perubahan data.
    let (st, h, _) = u
        .kirim(
            Method::GET,
            &format!("{dasar}/pratinjau-nonaktif"),
            None,
            None,
            None,
        )
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    no_store(&h);
    let (st, _, j) = u
        .kirim(
            Method::POST,
            &format!("{dasar}/nonaktifkan"),
            Some(&pa),
            Some("https://asing.example.test"),
            Some(json!({ "alasan": "uji origin salah sintetis" })),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert_eq!(j["kode"], "origin_tidak_valid");

    // AC-4: client_owner di luar kolom Peran, 403 tanpa perubahan data.
    let (_, co) = u.pengguna(&["client_owner"], Some(r.client)).await;
    let (st, _, _) = u
        .kirim(
            Method::POST,
            &format!("{dasar}/nonaktifkan"),
            Some(&co),
            Some(ASAL),
            Some(json!({ "alasan": "uji peran client owner" })),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 404 untuk id tak dikenal pada ketiga operasi.
    let hantu = Uuid::now_v7();
    for (method, sufiks, body) in [
        (Method::GET, "pratinjau-nonaktif", None),
        (
            Method::POST,
            "nonaktifkan",
            Some(json!({ "alasan": "uji id tidak dikenal" })),
        ),
        (Method::POST, "aktifkan", None),
    ] {
        let (st, h, _) = u
            .kirim(
                method,
                &format!("/admin/v1/{}/{hantu}/{sufiks}", k.path),
                Some(&pa),
                Some(ASAL),
                body,
            )
            .await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{} {sufiks}", k.nama);
        no_store(&h);
    }

    // Fixture: satu request per status.
    let queued = request_queued(&u.su, r.client, r.kredensial, r.endpoint, r.dpool, nik).await;
    let retry = request_menunggu_retry(&u.su, &r, nik).await;
    let dispatched = request_dispatched(&u.su, &r, nik).await;
    let in_flight = request_in_flight(&u.su, &r).await;
    let (cancelled, failed) = (1 + i64::from(k.queued_terdampak), 1);

    // AC-28: pratinjau hanya angka, tanpa request_id, NIK, atau payload.
    let (st, h, p) = u
        .kirim(
            Method::GET,
            &format!("{dasar}/pratinjau-nonaktif"),
            Some(&pa),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert_eq!(p["akan_cancelled"], cancelled, "{}", k.nama);
    assert_eq!(p["akan_failed"], failed, "{}", k.nama);
    assert!(p.get("dihitung_pada").is_some());
    assert!(!p.to_string().contains(nik));
    assert!(p.get("request_id").is_none());
    assert_eq!(
        p.get("upstream_aktif_terakhir_untuk").is_some(),
        k.nama == "upool",
        "keterangan tambahan hanya untuk upstream pool"
    );

    // Alasan terlalu pendek: 422 tanpa perubahan data.
    let (st, _, j) = u
        .kirim(
            Method::POST,
            &format!("{dasar}/nonaktifkan"),
            Some(&pa),
            Some(ASAL),
            Some(json!({ "alasan": "pendek" })),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(j["errors"][0]["field"], "alasan");
    assert_eq!(status_request(&u.su, queued).await.0, "queued");

    // AC-29: nonaktifkan, satu transisi per request, angka sama dengan audit.
    let alasan = "uji matriks K-20 ARC-168 sintetis";
    let (st, h, hasil) = u
        .kirim(
            Method::POST,
            &format!("{dasar}/nonaktifkan"),
            Some(&pa),
            Some(ASAL),
            Some(json!({ "alasan": alasan })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert_eq!(hasil["jumlah_cancelled"], cancelled, "{}", k.nama);
    assert_eq!(hasil["jumlah_failed"], failed, "{}", k.nama);
    assert!(!hasil.to_string().contains(nik));
    let (s_queued, alasan_cancel, _) = status_request(&u.su, queued).await;
    if k.queued_terdampak {
        assert_eq!(s_queued, "cancelled");
        assert_eq!(alasan_cancel.as_deref(), Some(k.alasan_cancel));
    } else {
        assert_eq!(
            s_queued, "queued",
            "EN-04: belum di-dispatch, tidak disentuh"
        );
    }
    let (s_dispatched, alasan_dispatched, _) = status_request(&u.su, dispatched).await;
    assert_eq!(s_dispatched, "cancelled", "{}", k.nama);
    assert_eq!(alasan_dispatched.as_deref(), Some(k.alasan_cancel));
    let (s_retry, _, gagal) = status_request(&u.su, retry).await;
    assert_eq!(s_retry, "failed");
    assert_eq!(gagal.as_deref(), Some("objek_nonaktif"));
    assert_eq!(status_request(&u.su, in_flight).await.0, "in_flight");
    let audit = u.audit_terakhir(k.entitas, id).await.expect("audit");
    assert_eq!(audit["aksi"], "nonaktifkan");
    assert_eq!(audit["alasan"], alasan);
    assert_eq!(audit["jumlah_cancelled"], hasil["jumlah_cancelled"]);
    assert_eq!(audit["jumlah_failed"], hasil["jumlah_failed"]);
    let aktif: bool = sqlx::query_scalar(k.aktif_sql)
        .bind(id)
        .fetch_one(&u.su)
        .await
        .unwrap();
    assert!(!aktif);

    // Idempotensi: nonaktifkan ulang tidak menutup apa pun lagi.
    let (st, _, ulang) = u
        .kirim(
            Method::POST,
            &format!("{dasar}/nonaktifkan"),
            Some(&pa),
            Some(ASAL),
            Some(json!({ "alasan": alasan })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(ulang["jumlah_cancelled"], 0);
    assert_eq!(ulang["jumlah_failed"], 0);
    let baris_bukan_nol: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_konfigurasi WHERE entitas_jenis = $1 AND entitas_id = $2 \
         AND aksi = 'nonaktifkan' AND jumlah_cancelled + jumlah_failed > 0",
    )
    .bind(k.entitas)
    .bind(id)
    .fetch_one(&u.pool)
    .await
    .unwrap();
    assert_eq!(baris_bukan_nol, 1);

    // AC-30: aktifkan mengembalikan entitas dan tidak mengubah request lama.
    let (st, h, entitas) = u
        .kirim(
            Method::POST,
            &format!("{dasar}/aktifkan"),
            Some(&pa),
            Some(ASAL),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{} {entitas}", k.nama);
    no_store(&h);
    assert_eq!(entitas["aktif"], true);
    assert_eq!(entitas[k.kunci_id], *id.to_string().as_str());
    assert_eq!(status_request(&u.su, retry).await.0, "failed");
    assert_eq!(status_request(&u.su, in_flight).await.0, "in_flight");
    assert_eq!(status_request(&u.su, queued).await.0, s_queued);
    assert_eq!(status_request(&u.su, dispatched).await.0, "cancelled");
    let audit = u.audit_terakhir(k.entitas, id).await.expect("audit");
    assert_eq!(audit["aksi"], "aktifkan");
    let aktif: bool = sqlx::query_scalar(k.aktif_sql)
        .bind(id)
        .fetch_one(&u.su)
        .await
        .unwrap();
    assert!(aktif);
}

#[tokio::test]
#[ignore]
async fn endpoint_matriks_k20() {
    matriks(&KASUS[0]).await;
}

#[tokio::test]
#[ignore]
async fn downstream_pool_matriks_k20() {
    matriks(&KASUS[1]).await;
}

#[tokio::test]
#[ignore]
async fn upstream_pool_matriks_k20() {
    matriks(&KASUS[2]).await;
}

#[tokio::test]
#[ignore]
async fn provider_matriks_k20() {
    matriks(&KASUS[3]).await;
}

/// AC-31: dua nonaktifkan bersamaan atas entitas yang sama, satu transisi per
/// request dan satu baris audit dengan angka bukan nol.
async fn paralel(k: &Kasus) {
    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"], None).await;
    let r = rute(&u, &format!("p{}", k.nama)).await;
    let id = (k.id)(&r);
    let retry = request_menunggu_retry(&u.su, &r, "0000000000000043").await;
    let queued = request_queued(
        &u.su,
        r.client,
        r.kredensial,
        r.endpoint,
        r.dpool,
        "0000000000000044",
    )
    .await;

    let path = format!("/admin/v1/{}/{id}/nonaktifkan", k.path);
    let kirim = |alasan: &'static str| {
        let req = Request::builder()
            .method(Method::POST)
            .uri(path.clone())
            .header(header::COOKIE, pa.as_str())
            .header(header::ORIGIN, ASAL)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "alasan": alasan }).to_string()))
            .unwrap();
        let app = u.app.clone();
        async move { app.oneshot(req).await.unwrap().status() }
    };
    let (a, b) = tokio::join!(
        kirim("uji paralel pertama sintetis"),
        kirim("uji paralel kedua sintetis")
    );
    assert_eq!((a, b), (StatusCode::OK, StatusCode::OK));

    for rid in [retry, queued] {
        let transisi: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM request_status_history WHERE request_id = $1 \
             AND status_ke IN ('cancelled', 'failed')",
        )
        .bind(rid)
        .fetch_one(&u.su)
        .await
        .unwrap();
        let diharapkan = i64::from(rid == retry || k.queued_terdampak);
        assert_eq!(
            transisi, diharapkan,
            "{}: satu transisi per request",
            k.nama
        );
    }
    let baris_bukan_nol: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_konfigurasi WHERE entitas_jenis = $1 AND entitas_id = $2 \
         AND jumlah_cancelled + jumlah_failed > 0",
    )
    .bind(k.entitas)
    .bind(id)
    .fetch_one(&u.pool)
    .await
    .unwrap();
    assert_eq!(baris_bukan_nol, 1, "{}: satu baris audit bukan nol", k.nama);
}

#[tokio::test]
#[ignore]
async fn endpoint_dua_nonaktifkan_bersamaan() {
    paralel(&KASUS[0]).await;
}

#[tokio::test]
#[ignore]
async fn downstream_pool_dua_nonaktifkan_bersamaan() {
    paralel(&KASUS[1]).await;
}

#[tokio::test]
#[ignore]
async fn upstream_pool_dua_nonaktifkan_bersamaan() {
    paralel(&KASUS[2]).await;
}

#[tokio::test]
#[ignore]
async fn provider_dua_nonaktifkan_bersamaan() {
    paralel(&KASUS[3]).await;
}

/// §3a.10: pratinjau upstream pool menandai downstream pool yang upstream
/// aktif terakhirnya adalah pool ini, dihitung dari konfigurasi.
#[tokio::test]
#[ignore]
async fn upstream_pool_pratinjau_aktif_terakhir() {
    let u = siapkan().await;
    let (_, pa) = u.pengguna(&["platform_admin"], None).await;
    let r = rute(&u, "terakhir").await;
    let path = format!("/admin/v1/upstream-pools/{}/pratinjau-nonaktif", r.upstream);

    let (st, _, p) = u
        .kirim(Method::GET, &path, Some(&pa), Some(ASAL), None)
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(p["dipakai_oleh_downstream_pool_count"], 1);
    assert_eq!(
        p["upstream_aktif_terakhir_untuk"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        p["upstream_aktif_terakhir_untuk"][0]["downstream_pool_id"],
        *r.dpool.to_string().as_str()
    );
    assert!(p["upstream_aktif_terakhir_untuk"][0]["nama"].is_string());

    // Upstream pool aktif kedua di rute yang sama: bukan lagi yang terakhir.
    let kedua = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, kuota_harian, batas_percobaan, retry_backoff_base_detik) \
         VALUES ($1, $2, $3, 1000, 3, 1)",
    )
    .bind(kedua)
    .bind(r.provider)
    .bind(unik("up-kedua"))
    .execute(&u.su)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id) VALUES ($1, $2)",
    )
    .bind(r.dpool)
    .bind(kedua)
    .execute(&u.su)
    .await
    .unwrap();
    let (_, _, p) = u
        .kirim(Method::GET, &path, Some(&pa), Some(ASAL), None)
        .await;
    assert_eq!(p["dipakai_oleh_downstream_pool_count"], 1);
    assert_eq!(
        p["upstream_aktif_terakhir_untuk"].as_array().unwrap().len(),
        0
    );
}
