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
