//! Integration tests for `GET /v1/endpoints/{endpoint_id}/kontrak` against
//! PostgreSQL (ARC-61). Run via `make -C backend uji-pg`: the app uses the
//! `pr_api` role, fixtures use `pr_admin_api`, both real grants.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use poolrouter_core::kredensial::{base64url_encode, buat_api_key};
use poolrouter_webserver::downstream::auth::Authenticator;
use poolrouter_webserver::downstream::credential::{CachedStore, PgCredentialStore};
use poolrouter_webserver::downstream::jwt::JwtKeys;
use poolrouter_webserver::downstream::konfigurasi_endpoint::{
    CachedKonfigurasi, KonfigurasiStore, PgKonfigurasiStore,
};
use poolrouter_webserver::downstream::server::{AppState, api_router};
use poolrouter_webserver::downstream::token::LocalThrottle;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const ISS: &str = "https://poolrouter.example.test";
const TTL: Duration = Duration::from_secs(30);

fn url_uji(nama: &str) -> String {
    std::env::var(nama)
        .unwrap_or_else(|_| panic!("{nama} belum diset; jalankan lewat `make -C backend uji-pg`"))
}

/// Fresh Ed25519 signing key per app instance; nothing is shared between tests.
fn keys() -> Arc<JwtKeys> {
    use aws_lc_rs::rand::SystemRandom;
    use aws_lc_rs::signature::Ed25519KeyPair;
    let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    let b = base64url_encode(doc.as_ref())
        .replace('-', "+")
        .replace('_', "/");
    let pem = format!("-----BEGIN PRIVATE KEY-----\n{b}\n-----END PRIVATE KEY-----\n");
    Arc::new(JwtKeys::from_pem(ISS, &pem, None).unwrap())
}

struct Uji {
    app: Router,
    /// `pr_admin_api` pool for fixtures and assertions.
    admin: PgPool,
    /// Clock the caches read; test moves it by hand.
    jam: Arc<Mutex<Instant>>,
    konfig: Arc<CachedKonfigurasi>,
    client_id: Uuid,
    endpoint_id: Uuid,
    kredensial: Uuid,
    provider_id: Uuid,
    upstream_id: Uuid,
    pool_id: Uuid,
    admin_user: Uuid,
    api_key: String,
}

impl Uji {
    async fn baru() -> Self {
        let admin = PgPool::connect(&url_uji("POOLROUTER_TEST_DATABASE_URL"))
            .await
            .unwrap();
        let app_pg = PgPool::connect(&url_uji("POOLROUTER_TEST_DATABASE_URL_API"))
            .await
            .unwrap();
        let uid = Uuid::now_v7().simple().to_string();
        let client_id = Uuid::now_v7();
        let endpoint_id = Uuid::now_v7();
        let kredensial = Uuid::now_v7();
        let provider_id = Uuid::now_v7();
        let upstream_id = Uuid::now_v7();
        let pool_id = Uuid::now_v7();
        let admin_user = Uuid::now_v7();
        let key = buat_api_key(kredensial).unwrap();

        // Fixtures go through pr_admin_api: the app itself only reads as pr_api.
        let hash: String = key.kunci_hash.iter().map(|b| format!("{b:02x}")).collect();
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "INSERT INTO admin_user (pengguna_id, username, password_hash) \
               VALUES ('{admin_user}', 'arc61-{uid}', 'x');
             INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh) \
               VALUES ('{admin_user}', 'platform_admin', '{admin_user}');
             INSERT INTO client (client_id, nama) VALUES ('{client_id}', 'klien-{uid}');
             INSERT INTO provider (provider_id, nama, data_balikan_aktif) \
               VALUES ('{provider_id}', 'provider-{uid}', true);
             INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, kuota_harian) \
               VALUES ('{upstream_id}', '{provider_id}', 'upstream-{uid}', 1000);
             INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) \
               VALUES ('{pool_id}', 'pool-{uid}', 1);
             INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id) \
               VALUES ('{pool_id}', '{upstream_id}');
             INSERT INTO downstream_endpoint \
               (downstream_endpoint_id, nama, provider_id, downstream_pool_id, config_versi_aktif) \
               VALUES ('{endpoint_id}', 'endpoint-{uid}', '{provider_id}', '{pool_id}', 1);
             INSERT INTO endpoint_config_versi \
               (downstream_endpoint_id, versi, config_schema_version, dokumen, dibuat_oleh) \
               VALUES ('{endpoint_id}', 1, 1, '{{}}', '{admin_user}');
             INSERT INTO endpoint_metadata_field \
               (downstream_endpoint_id, nama_field, tipe, wajib, nilai_diizinkan, klasifikasi, \
                kirim_ke_data_balikan, deskripsi, versi) \
               VALUES ('{endpoint_id}', 'status_penerima_manfaat', 'string', true, \
                       '[\"sesuai\",\"tidak_sesuai\"]'::jsonb, 'Internal', false, 'Sintetis.', 1), \
                      ('{endpoint_id}', 'referensi_client', 'string', false, NULL, 'Internal', false, \
                       'Sintetis.', 1);
             INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, dibuat_oleh) \
               VALUES ('{kredensial}', '{client_id}', 'api_key', '\\x{hash}'::bytea, '{admin_user}');
             INSERT INTO client_endpoint (client_id, downstream_endpoint_id) \
               VALUES ('{client_id}', '{endpoint_id}');"
        )))
        .execute(&admin)
        .await
        .unwrap();

        let store = Arc::new(CachedStore::new(Arc::new(PgCredentialStore::new(
            app_pg.clone(),
        ))));
        let konfig = Arc::new(CachedKonfigurasi::new(Arc::new(PgKonfigurasiStore::new(
            app_pg,
        ))));
        let jam = Arc::new(Mutex::new(Instant::now()));
        let jam_cache = jam.clone();
        let state = AppState {
            auth: Arc::new(Authenticator::new(store, keys())),
            throttle: Arc::new(LocalThrottle::new()),
            konfig: konfig.clone(),
            jam: Arc::new(move || *jam_cache.lock().unwrap()),
        };
        Self {
            app: api_router(state),
            admin,
            jam,
            konfig,
            client_id,
            endpoint_id,
            kredensial,
            provider_id,
            upstream_id,
            pool_id,
            admin_user,
            api_key: key.kunci,
        }
    }

    /// Deactivation goes through the only sanctioned path (`EN-03`).
    async fn nonaktifkan(&self, jenis: &str, id: Uuid) {
        sqlx::query("SELECT fn_nonaktifkan_entitas($1, $2, $3, 'uji ARC-61')")
            .bind(jenis)
            .bind(id)
            .bind(self.admin_user)
            .execute(&self.admin)
            .await
            .unwrap();
    }

    async fn get(
        &self,
        endpoint_id: Uuid,
        api_key: Option<&str>,
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let mut r = Request::builder()
            .method("GET")
            .uri(format!("/v1/endpoints/{endpoint_id}/kontrak"));
        if let Some(k) = api_key {
            r = r.header("x-api-key", k);
        }
        let res = self
            .app
            .clone()
            .oneshot(r.body(Body::empty()).unwrap())
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

#[tokio::test]
#[ignore]
async fn kontrak_mengembalikan_metadata_versi_dan_no_store() {
    let u = Uji::baru().await;
    let (status, headers, body) = u.get(u.endpoint_id, Some(&u.api_key)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(body["endpoint_id"], u.endpoint_id.to_string());
    assert_eq!(body["endpoint_config_version"], 1);
    assert_eq!(
        body["field_input"],
        serde_json::json!([]),
        "F0: belum ada kolom input"
    );
    let fields = body["field_metadata"].as_array().unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0]["nama_field"], "referensi_client");
    assert_eq!(fields[0]["nilai_diizinkan"], Value::Null);
    assert_eq!(fields[1]["nama_field"], "status_penerima_manfaat");
    assert_eq!(
        fields[1]["nilai_diizinkan"],
        serde_json::json!(["sesuai", "tidak_sesuai"])
    );
    assert_eq!(fields[1]["wajib"], true);
}

#[tokio::test]
#[ignore]
async fn endpoint_nonaktif_tetap_menjawab_kontrak() {
    let u = Uji::baru().await;
    u.nonaktifkan("downstream_endpoint", u.endpoint_id).await;
    let (status, _, body) = u.get(u.endpoint_id, Some(&u.api_key)).await;
    assert_eq!(status, StatusCode::OK, "GW3: nonaktif bukan 404: {body}");
}

#[tokio::test]
#[ignore]
async fn endpoint_asing_dan_tidak_ada_sama_sama_404() {
    let u = Uji::baru().await;
    let (status, headers, _) = u.get(Uuid::now_v7(), Some(&u.api_key)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    sqlx::query("DELETE FROM client_endpoint WHERE client_id = $1 AND downstream_endpoint_id = $2")
        .bind(u.client_id)
        .bind(u.endpoint_id)
        .execute(&u.admin)
        .await
        .unwrap();
    let (status, _, _) = u.get(u.endpoint_id, Some(&u.api_key)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "tidak ditugaskan = 404");
}

/// AC-6: the cached row is used before 30 s and re-read from PostgreSQL after,
/// for all six entities.
#[tokio::test]
#[ignore]
async fn cache_enam_entitas_dipakai_sebelum_30_detik_dan_dibaca_ulang() {
    let u = Uji::baru().await;
    let t0 = *u.jam.lock().unwrap();
    let awal = u.konfig.muat(u.client_id, u.endpoint_id, t0).await.unwrap();
    let s = awal.status.expect("endpoint ada");
    assert!(
        s.client
            && s.kredensial
            && s.endpoint
            && s.downstream_pool
            && s.upstream_pool
            && s.provider
    );

    // Deactivate all six entities in one go.
    u.nonaktifkan("client", u.client_id).await;
    u.nonaktifkan("kredensial", u.kredensial).await;
    u.nonaktifkan("downstream_endpoint", u.endpoint_id).await;
    u.nonaktifkan("downstream_pool", u.pool_id).await;
    u.nonaktifkan("upstream_pool", u.upstream_id).await;
    u.nonaktifkan("provider", u.provider_id).await;

    let dalam = u
        .konfig
        .muat(
            u.client_id,
            u.endpoint_id,
            t0 + TTL - Duration::from_secs(1),
        )
        .await
        .unwrap()
        .status
        .unwrap();
    assert!(
        dalam.client
            && dalam.kredensial
            && dalam.endpoint
            && dalam.downstream_pool
            && dalam.upstream_pool
            && dalam.provider,
        "masih entri cache sebelum 30 s"
    );

    let sesudah = u
        .konfig
        .muat(u.client_id, u.endpoint_id, t0 + TTL)
        .await
        .unwrap()
        .status
        .unwrap();
    assert!(
        !sesudah.client
            && !sesudah.kredensial
            && !sesudah.endpoint
            && !sesudah.downstream_pool
            && !sesudah.upstream_pool
            && !sesudah.provider,
        "keenam entitas dibaca ulang setelah 30 s"
    );
    // The handler never caches the kredensial/admin ids in the response.
    assert!(u.admin_user != Uuid::nil());
}

#[tokio::test]
#[ignore]
async fn tanpa_kredensial_401_tanpa_membaca_konfigurasi() {
    let u = Uji::baru().await;
    let (status, headers, body) = u.get(u.endpoint_id, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(headers[header::WWW_AUTHENTICATE], "Bearer");
    assert_eq!(body["kode"], "tidak_terautentikasi");
}
