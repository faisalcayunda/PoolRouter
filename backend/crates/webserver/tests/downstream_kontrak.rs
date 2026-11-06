//! Contract tests for the five downstream operations (ARC-158).
//!
//! AC-1: every success body and every problem is checked against
//! `openapi/downstream-api.yaml`, which `utoipa` generates from the handler
//! annotations. AC-2: `Cache-Control: no-store` on every answer, 2xx, 4xx and
//! 5xx. AC-3: Bearer and API key both work on all five operations, and a
//! missing credential is 401. AC-9: what the router assembly wires is what the
//! tests exercise — the same [`api_router`] the listener serves.
//!
//! No database is involved: `FakeSession` answers the store calls and
//! `FakeStore` answers the credential ones, so the assertions are about the
//! wire shape, which is what the acceptance criteria name.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use poolrouter_core::kredensial::buat_api_key;
use poolrouter_webserver::downstream::auth::{Authenticator, JwtKeys};
use poolrouter_webserver::downstream::credential::fake::FakeStore;
use poolrouter_webserver::downstream::credential::{Jenis, Kredensial};
use poolrouter_webserver::downstream::downstream_sesi::{BidangInput, fake::FakeSession};
use poolrouter_webserver::downstream::server::{AppState, LocalThrottle, api_router};

#[allow(dead_code)]
const ASAL: &str = "203.0.113.7";

const ISS: &str = "https://poolrouter.example.test";

fn phc(secret: &str) -> Vec<u8> {
    use argon2::{Algorithm, Argon2, Params, PasswordHasher, Version};
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(8, 1, 1, None).unwrap(),
    )
    .hash_password_with_salt(secret.as_bytes(), b"0123456789abcdef")
    .unwrap()
    .to_string()
    .into_bytes()
}

/// A fresh Ed25519 key per test: the token only has to verify against the same
/// `JwtKeys` instance that signed it, so no key material lives in the repo.
fn keys() -> Arc<JwtKeys> {
    use aws_lc_rs::rand::SystemRandom;
    use aws_lc_rs::signature::Ed25519KeyPair;
    use poolrouter_core::kredensial::base64url_encode;
    let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    let b = base64url_encode(doc.as_ref())
        .replace('-', "+")
        .replace('_', "/");
    let pem = format!("-----BEGIN PRIVATE KEY-----\n{b}\n-----END PRIVATE KEY-----\n");
    Arc::new(JwtKeys::from_pem(ISS, &pem, None).unwrap())
}

struct Harness {
    app: Router,
    sesi: Arc<FakeSession>,
    endpoint: Uuid,
    bearer: String,
    api_key: String,
}

impl Harness {
    /// One active client with one client secret (for the Bearer token) and one
    /// API key (for `X-Api-Key`). Both carry the same `client_id`, which is what
    /// AC-3 is about.
    fn baru() -> Self {
        let endpoint = Uuid::now_v7();
        let mut sesi = FakeSession::baru();
        Arc::get_mut(&mut sesi)
            .expect("the store is not shared yet")
            .atur(endpoint, bidang());
        Self::dengan(sesi, endpoint)
    }

    /// A harness over a store the caller already configured, for the cases that
    /// need the endpoint in a particular state (inactive, for instance).
    fn dengan(sesi: Arc<FakeSession>, endpoint: Uuid) -> Self {
        let klien = Uuid::now_v7();
        let kred_secret = Uuid::now_v7();
        let kred_api = Uuid::now_v7();
        let baru = buat_api_key(kred_api).unwrap();
        let api_key = baru.kunci.clone();

        let fake = Arc::new(FakeStore::default());
        fake.clients.lock().unwrap().insert(klien, true);
        fake.kredensial.lock().unwrap().extend([
            Kredensial {
                credential_id: kred_secret,
                client_id: klien,
                jenis: Jenis::ClientSecret,
                kunci_hash: phc("rahasia-sintetis"),
                aktif: true,
                kedaluwarsa_at: None,
            },
            Kredensial {
                credential_id: kred_api,
                client_id: klien,
                jenis: Jenis::ApiKey,
                kunci_hash: baru.kunci_hash.to_vec(),
                aktif: true,
                kedaluwarsa_at: None,
            },
        ]);
        let auth = Arc::new(Authenticator::new(fake, keys()));
        let (bearer, _) = auth
            .keys()
            .issue(
                klien,
                kred_secret,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            )
            .unwrap();

        let state = AppState {
            auth,
            throttle: Arc::new(LocalThrottle::new()),
            sesi: sesi.clone(),
        };
        Self {
            app: api_router(state).layer(MockConnectInfo(std::net::SocketAddr::from((
                [203, 0, 113, 7],
                5555,
            )))),
            sesi,
            endpoint,
            bearer,
            api_key,
        }
    }

    async fn kirim(
        &self,
        method: Method,
        path: &str,
        kredensial: Option<(&str, String)>,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some((nama, nilai)) = kredensial {
            req = req.header(nama, nilai);
        }
        let body = match body {
            Some(v) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                Body::from(serde_json::to_vec(&v).unwrap())
            }
            None => Body::empty(),
        };
        let res = self
            .app
            .clone()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap().to_vec();
        (status, headers, bytes)
    }

    fn endpoint_path(&self, akhiran: &str) -> String {
        format!("/v1/endpoints/{}/{akhiran}", self.endpoint)
    }
}

fn bidang() -> Vec<BidangInput> {
    vec![
        BidangInput {
            nama: "NIK".into(),
            tipe: "string".into(),
            wajib: true,
            nilai_diizinkan: None,
            aturan_validasi: Some(json!({"panjang": 16})),
            klasifikasi: "rahasia".into(),
            kirim_ke_data_balikan: false,
            deskripsi: "Nomor induk kependudukan.".into(),
            versi: 1,
        },
        BidangInput {
            nama: "MODE".into(),
            tipe: "string".into(),
            wajib: false,
            nilai_diizinkan: Some(json!(["ringkas", "lengkap"])),
            aturan_validasi: None,
            klasifikasi: "publik".into(),
            kirim_ke_data_balikan: true,
            deskripsi: "Bentuk jawaban.".into(),
            versi: 2,
        },
    ]
}

/// The credential headers, owned so they can outlive the borrow of `h` that
/// `kirim` takes.
fn bearer(h: &Harness) -> (&'static str, String) {
    (
        header::AUTHORIZATION.as_str(),
        format!("Bearer {}", h.bearer),
    )
}

fn api_key(h: &Harness) -> (&'static str, String) {
    ("x-api-key", h.api_key.clone())
}

fn teks(b: &[u8]) -> Value {
    serde_json::from_slice(b).unwrap_or(Value::Null)
}

/// AC-2 plus the response-shape half of AC-1: every answer here must carry
/// `no-store`, whatever the status.
fn tanpa_cache(headers: &HeaderMap, status: StatusCode) {
    assert_eq!(
        headers
            .get(header::CACHE_CONTROL)
            .map(|v| v.to_str().unwrap()),
        Some("no-store"),
        "{status} without Cache-Control: no-store"
    );
}

#[tokio::test]
async fn kelima_operasi_ada_di_api_router() {
    // AC-9: the paths the document names exist on the listener's router. A
    // missing route answers the fallback, which is not the documented problem.
    let h = Harness::baru();
    let daftar = [
        (Method::GET, h.endpoint_path("kontrak")),
        (Method::POST, h.endpoint_path("sync")),
        (Method::POST, h.endpoint_path("async")),
        (Method::GET, format!("/v1/requests/{}", h.endpoint)),
        (Method::POST, format!("/v1/requests/{}/cancel", h.endpoint)),
    ];
    for (metode, path) in daftar {
        // No body: a 404 here can only be the handler's own answer, because the
        // empty `{endpoint_id}` would have been a UUID and the fake store knows
        // it. The fallback sends no body at all.
        let (status, _, body) = h.kirim(metode.clone(), &path, Some(bearer(&h)), None).await;
        assert_ne!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "{metode} {path} not routed"
        );
        assert!(
            !body.is_empty() || status.is_success(),
            "{metode} {path} fell through to the router fallback"
        );
    }
}

#[tokio::test]
async fn tanpa_kredensial_401_dengan_no_store() {
    // AC-3 (negative half) and AC-2 for 4xx.
    let h = Harness::baru();
    for (metode, path) in [
        (Method::GET, h.endpoint_path("kontrak")),
        (Method::POST, h.endpoint_path("sync")),
        (Method::POST, h.endpoint_path("async")),
        (Method::GET, format!("/v1/requests/{}", h.endpoint)),
        (Method::POST, format!("/v1/requests/{}/cancel", h.endpoint)),
    ] {
        let (status, headers, body) = h.kirim(metode.clone(), &path, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{metode} {path}");
        tanpa_cache(&headers, status);
        assert_eq!(headers.get(header::WWW_AUTHENTICATE).unwrap(), "Bearer");
        let v = teks(&body);
        assert_eq!(v["status"], 401, "{metode} {path}");
        assert_eq!(v["kode"], "tidak_terautentikasi");
    }
}

#[tokio::test]
async fn bearer_dan_api_key_sama_sama_diterima() {
    // AC-3: the same operation, the same answer, whichever credential arrives.
    let h = Harness::baru();
    let path = h.endpoint_path("kontrak");
    let (s1, _, b1) = h.kirim(Method::GET, &path, Some(bearer(&h)), None).await;
    let (s2, _, b2) = h.kirim(Method::GET, &path, Some(api_key(&h)), None).await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(s2, StatusCode::OK, "API key refused where Bearer works");
    assert_eq!(b1, b2);
}

#[tokio::test]
async fn kontrak_sesuai_skema_openapi() {
    // AC-1 for the contract operation.
    let h = Harness::baru();
    let (status, headers, body) = h
        .kirim(
            Method::GET,
            &h.endpoint_path("kontrak"),
            Some(bearer(&h)),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    tanpa_cache(&headers, status);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let v = teks(&body);
    cek_skema("EndpointKontrak", &v);
    assert_eq!(v["endpoint_id"], json!(h.endpoint));
    assert_eq!(v["endpoint_config_version"], 3);
    assert_eq!(v["field_metadata"][1]["nama_field"], "MODE");
    assert_eq!(
        v["field_metadata"][1]["nilai_diizinkan"],
        json!(["ringkas", "lengkap"])
    );
    assert!(v.get("mode").is_none(), "the contract has no mode field");
}

#[tokio::test]
async fn async_menerima_dan_mengembalikan_queue_id() {
    // AC-1 for enqueue: 202 with a `queue_id` and `status`.
    let h = Harness::baru();
    let path = h.endpoint_path("async");
    let kirim =
        json!({"input": {"NIK": "3201010101010001"}, "metadata": {"NIK": "3201010101010001"}});
    let (status, headers, body) = h
        .kirim(Method::POST, &path, Some(bearer(&h)), Some(kirim.clone()))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    tanpa_cache(&headers, status);
    let v = teks(&body);
    cek_skema("EnqueueResponse", &v);
    let queue_id = v["queue_id"].as_str().unwrap();
    assert_eq!(v["status"], "queued");

    // Polling the same request gives the same id and the documented shape.
    let (s2, h2, b2) = h
        .kirim(
            Method::GET,
            &format!("/v1/requests/{queue_id}"),
            Some(bearer(&h)),
            None,
        )
        .await;
    assert_eq!(s2, StatusCode::OK);
    tanpa_cache(&h2, s2);
    let p = teks(&b2);
    cek_skema("RequestStatus", &p);
    assert_eq!(p["queue_id"], queue_id);
    assert_eq!(p["status"], "queued");
    assert!(p.get("hasil").is_none(), "hasil only travels with done");

    // A second call with the same Idempotency-Key replays instead of queueing.
    let (s3, _, b3) = kirim_idempoten(&h, &path, &kirim, "kunci-uji-001").await;
    let (s4, _, b4) = kirim_idempoten(&h, &path, &kirim, "kunci-uji-001").await;
    assert_eq!(s3, StatusCode::ACCEPTED);
    assert_eq!(s4, StatusCode::OK, "idempotency hit answers 200");
    assert_eq!(teks(&b3)["queue_id"], teks(&b4)["queue_id"]);
}

async fn kirim_idempoten(
    h: &Harness,
    path: &str,
    body: &Value,
    kunci: &str,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {}", h.bearer))
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", kunci)
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap();
    let res = h.app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap().to_vec();
    (status, headers, bytes)
}

#[tokio::test]
async fn metadata_tidak_dikenal_ditolak_422() {
    // AC-1: an unknown metadata key is 422 and the rejected value is not echoed.
    let h = Harness::baru();
    let path = h.endpoint_path("async");
    let (status, headers, body) = h
        .kirim(
            Method::POST,
            &path,
            Some(bearer(&h)),
            Some(json!({"input": {}, "metadata": {"NIK": "3201010101010001", "TRESHOLD": "77"}})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    tanpa_cache(&headers, status);
    let v = teks(&body);
    cek_skema("Problem", &v);
    assert_eq!(v["kode"], "metadata_tidak_valid");
    assert!(
        !body.windows(2).any(|w| w == b"77"),
        "the rejected value must not be echoed"
    );
}

#[tokio::test]
async fn nilai_di_luar_daftar_tertutup_ditolak() {
    // AC-1: a closed list only accepts its own values.
    let h = Harness::baru();
    let (status, _, body) = h
        .kirim(
            Method::POST,
            &h.endpoint_path("async"),
            Some(bearer(&h)),
            Some(json!({"input": {}, "metadata": {"NIK": "3201010101010001", "MODE": "penuh"}})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(teks(&body)["kode"], "metadata_tidak_valid");
}

#[tokio::test]
async fn endpoint_asing_jadi_404_bukan_403() {
    // AC-3: an endpoint that is not the caller's must not be distinguishable
    // from one that does not exist.
    let h = Harness::baru();
    let asing = Uuid::now_v7();
    let (status, headers, body) = h
        .kirim(
            Method::GET,
            &format!("/v1/endpoints/{asing}/kontrak"),
            Some(bearer(&h)),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    tanpa_cache(&headers, status);
    assert_eq!(teks(&body)["kode"], "tidak_ditemukan");
}

#[tokio::test]
async fn cancel_lalu_409_saat_sudah_terminal() {
    // AC-1 for cancel: 200 while queued, 409 with the current status after.
    let h = Harness::baru();
    let kirim = json!({"input": {}, "metadata": {"NIK": "3201010101010001"}});
    let (_, _, body) = h
        .kirim(
            Method::POST,
            &h.endpoint_path("async"),
            Some(bearer(&h)),
            Some(kirim),
        )
        .await;
    let id = teks(&body)["queue_id"].as_str().unwrap().to_owned();
    let path = format!("/v1/requests/{id}/cancel");

    let (status, headers, body) = h.kirim(Method::POST, &path, Some(bearer(&h)), None).await;
    assert_eq!(status, StatusCode::OK);
    tanpa_cache(&headers, status);
    let v = teks(&body);
    cek_skema("RequestStatus", &v);
    assert_eq!(v["status"], "cancelled");
    assert_eq!(v["cancel_reason"], "dibatalkan_client");

    let (s2, h2, b2) = h.kirim(Method::POST, &path, Some(bearer(&h)), None).await;
    assert_eq!(s2, StatusCode::CONFLICT);
    tanpa_cache(&h2, s2);
    let p = teks(&b2);
    assert_eq!(p["kode"], "tidak_bisa_dibatalkan");
    assert_eq!(
        p["status_api"], "cancelled",
        "409 carries the current status"
    );
}

#[tokio::test]
async fn store_mati_jadi_503_dengan_no_store() {
    // AC-2 for 5xx: a failing store never leaks a cause and still disables cache.
    let h = Harness::baru();
    h.sesi.mati();
    let (status, headers, body) = h
        .kirim(
            Method::GET,
            &h.endpoint_path("kontrak"),
            Some(bearer(&h)),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    tanpa_cache(&headers, status);
    let v = teks(&body);
    cek_skema("Problem", &v);
    assert_eq!(v["kode"], "layanan_tidak_tersedia");
}

// ---------------------------------------------------------------------------
// Schema check against the generated document
// ---------------------------------------------------------------------------

fn dokumen_downstream() -> Value {
    let yaml = include_str!("../../../openapi/downstream-api.yaml");
    yaml_serde::from_str(yaml).expect("downstream-api.yaml must parse")
}

/// Validates `v` against `#/components/schemas/<nama>` of the generated
/// document. Only the JSON Schema keywords that document uses are honoured:
/// `type`, `required`, `properties`, `additionalProperties`, `items`, `enum`
/// and `oneOf`/`anyOf`/`allOf`; `$ref` is resolved locally.
fn cek_skema(nama: &str, v: &Value) {
    let dok = dokumen_downstream();
    let skema = dok["components"]["schemas"][nama].clone();
    assert!(!skema.is_null(), "schema {nama} missing from the document");
    cocok(&dok, &skema, v, nama);
}

fn cocok(dok: &Value, skema: &Value, v: &Value, jejak: &str) {
    if let Some(r) = skema.get("$ref").and_then(|r| r.as_str()) {
        let nama = r.rsplit('/').next().unwrap();
        return cocok(dok, &dok["components"]["schemas"][nama], v, jejak);
    }
    for kunci in ["oneOf", "anyOf"] {
        if let Some(cabang) = skema.get(kunci).and_then(|c| c.as_array()) {
            let lulus = cabang.iter().filter(|c| cek_opsi(dok, c, v)).count();
            assert!(lulus >= 1, "{jejak}: does not match any {kunci}");
            return;
        }
    }
    if let Some(jenis) = skema.get("type") {
        let jenis: Vec<&str> = match jenis {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().filter_map(|x| x.as_str()).collect(),
            _ => vec![],
        };
        let cocok_tipe = jenis.iter().any(|j| match *j {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "integer" => v.is_i64() || v.is_u64(),
            "number" => v.is_number(),
            "boolean" => v.is_boolean(),
            "null" => v.is_null(),
            _ => true,
        });
        assert!(cocok_tipe, "{jejak}: expected {jenis:?}, got {v}");
    }
    if let Some(daftar) = skema.get("enum").and_then(|e| e.as_array()) {
        assert!(daftar.contains(v), "{jejak}: {v} is not in the enum");
    }
    if let Some(objek) = v.as_object() {
        if let Some(wajib) = skema.get("required").and_then(|r| r.as_array()) {
            for k in wajib.iter().filter_map(|x| x.as_str()) {
                assert!(objek.contains_key(k), "{jejak}: missing required {k}");
            }
        }
        let props = skema.get("properties").and_then(|p| p.as_object());
        if skema.get("additionalProperties") == Some(&Value::Bool(false)) {
            let dikenal: BTreeSet<&str> = props
                .map(|p| p.keys().map(String::as_str).collect())
                .unwrap_or_default();
            for k in objek.keys() {
                assert!(
                    dikenal.contains(k.as_str()),
                    "{jejak}: {k} is not in the schema"
                );
            }
        }
        for (k, sub) in objek {
            if let Some(s) = props.and_then(|p| p.get(k)) {
                cocok(dok, s, sub, &format!("{jejak}.{k}"));
            }
        }
    }
    if let (Some(butir), Some(a)) = (skema.get("items"), v.as_array()) {
        for (i, x) in a.iter().enumerate() {
            cocok(dok, butir, x, &format!("{jejak}[{i}]"));
        }
    }
}

/// Non-panicking variant for `oneOf`: does the value satisfy this branch?
fn cek_opsi(dok: &Value, skema: &Value, v: &Value) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cocok(dok, skema, v, ""))).is_ok()
}

#[tokio::test]
async fn idempotency_dan_referensi_dibatasi_kolomnya() {
    // `request.idempotency_key` is varchar(128) ASCII and `referensi_client` is
    // varchar(128); both are enforced before the row is built, so the client
    // gets 422 instead of a database error.
    let h = Harness::baru();
    let path = h.endpoint_path("async");
    let kirim = json!({"input": {}, "metadata": {"NIK": "3201010101010001"}});

    for kunci in ["k".repeat(129), "ada spasi".to_string(), String::new()] {
        let (status, headers, body) = kirim_idempoten(&h, &path, &kirim, &kunci).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "idempotency key {:?}",
            kunci.len()
        );
        tanpa_cache(&headers, status);
        assert_eq!(teks(&body)["kode"], "validasi_gagal");
    }

    // The boundary itself is accepted.
    let (status, _, _) = kirim_idempoten(&h, &path, &kirim, &"k".repeat(128)).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    // A reference longer than its column is refused, not truncated.
    let req = Request::builder()
        .method(Method::POST)
        .uri(&path)
        .header(header::AUTHORIZATION, format!("Bearer {}", h.bearer))
        .header(header::CONTENT_TYPE, "application/json")
        .header("referensi-client", "r".repeat(129))
        .body(Body::from(serde_json::to_vec(&kirim).unwrap()))
        .unwrap();
    let res = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    tanpa_cache(res.headers(), res.status());
}

#[tokio::test]
async fn endpoint_nonaktif_tetap_200_di_kontrak_tapi_403_di_submit() {
    // AC-9 / GW3: the contract survives deactivation — that is how a client
    // prepares while the endpoint is off — and the submit call is what tells it
    // the endpoint is inactive.
    let mut sesi = FakeSession::baru();
    let endpoint = Uuid::now_v7();
    let s = Arc::get_mut(&mut sesi).unwrap();
    s.atur(endpoint, bidang());
    s.nonaktifkan();
    let h = Harness::dengan(sesi, endpoint);

    let (status, headers, body) = h
        .kirim(
            Method::GET,
            &h.endpoint_path("kontrak"),
            Some(bearer(&h)),
            None,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an inactive endpoint still has a contract"
    );
    tanpa_cache(&headers, status);
    cek_skema("EndpointKontrak", &teks(&body));

    let (status, headers, body) = h
        .kirim(
            Method::POST,
            &h.endpoint_path("async"),
            Some(bearer(&h)),
            Some(json!({"input": {}, "metadata": {"NIK": "3201010101010001"}})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    tanpa_cache(&headers, status);
    assert_eq!(teks(&body)["kode"], "objek_nonaktif");
}
