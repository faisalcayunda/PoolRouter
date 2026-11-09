//! Endpoint berversi, kontrak metadata, dan `setClientEndpoints` lewat HTTP
//! terhadap PostgreSQL asli (ARC-54; AC-1..AC-5, AC-7, AC-25..AC-27).
//! Dijalankan oleh `make -C backend uji-pg`; `#[ignore]` karena butuh
//! `POOLROUTER_TEST_DATABASE_URL`.
//!
//! Router yang diuji adalah router admin produksi dengan cookie sesi dan
//! `Origin` asli, sebagai `pr_admin_api`. AC-27 membaca kontrak lewat
//! `PgKonfigurasiStore` sebagai `pr_api`, store yang dipakai
//! `GET /v1/endpoints/{id}/kontrak`. Semua nilai sintetis.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{DateTime, TimeZone, Utc};
use poolrouter_core::admin::sesi_store::SesiStore;
use poolrouter_core::admin::{AsalUi, TokenSesi};
use poolrouter_core::secret::Brankas;
use poolrouter_webserver::admin::pembatas::PembatasLogin;
use poolrouter_webserver::admin::sesi::{AdminState, COOKIE_SESI, router};
use poolrouter_webserver::downstream::konfigurasi_endpoint::{
    KonfigurasiStore, PgKonfigurasiStore,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const ASAL: &str = "https://ui.example.test";
const KEK_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const KUNCI_HMAC: &[u8] = b"kunci-hmac-sintetis-untuk-uji";
const ENDPOINTS: &str = "/admin/v1/endpoints";
const ALASAN: &str = "Perubahan kontrak metadata uji";

fn env(nama: &str) -> String {
    std::env::var(nama)
        .unwrap_or_else(|_| panic!("{nama} belum diset; jalankan lewat `make -C backend uji-pg`"))
}

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 5, 9, 0, 0).unwrap()
}

fn nama_unik(dasar: &str) -> String {
    format!("{dasar}-{}", Uuid::now_v7().simple())
}

struct Uji {
    app: Router,
    pool: PgPool,
    su: PgPool,
    cookie: String,
    pa: Uuid,
}

async fn siapkan() -> Uji {
    let pool = PgPool::connect(&env("POOLROUTER_TEST_DATABASE_URL"))
        .await
        .expect("database");
    let su = PgPool::connect(&env("POOLROUTER_TEST_DATABASE_URL_SU"))
        .await
        .expect("database su");
    let redis = env("POOLROUTER_TEST_REDIS");
    let state = AdminState::baru(
        pool.clone(),
        AsalUi::baru(ASAL).unwrap(),
        PembatasLogin::baru(redis::Client::open(redis.clone()).unwrap(), KUNCI_HMAC),
        Brankas::dari_isi_kek(KEK_HEX).unwrap(),
        redis::Client::open(redis).unwrap(),
        false,
    )
    .dengan_jam(Arc::new(t0));
    let app = router(state).layer(MockConnectInfo(SocketAddr::from(([203, 0, 113, 9], 4000))));

    let pa = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO admin_user (pengguna_id, username, password_hash) VALUES ($1, $2, 'x')",
    )
    .bind(pa)
    .bind(format!("sintetis-{}", pa.simple()))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh) \
         VALUES ($1, 'platform_admin', $1)",
    )
    .bind(pa)
    .execute(&pool)
    .await
    .unwrap();
    let token = TokenSesi::baru();
    SesiStore::baru(pool.clone())
        .terbitkan(pa, &token.hash(), Some("uji"), t0())
        .await
        .unwrap();
    let cookie = format!("{COOKIE_SESI}={}", token.sebagai_b64());
    Uji {
        app,
        pool,
        su,
        cookie,
        pa,
    }
}

impl Uji {
    async fn kirim(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method.clone())
            .uri(path)
            .header(header::COOKIE, &self.cookie);
        if method != Method::GET {
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
        assert_eq!(
            res.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store",
            "AC-2"
        );
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// Provider dengan template yang merujuk `{{endpoint.TRESHOLD}}`.
    async fn provider(&self, data_balikan: bool, log_mode: &str) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO provider (provider_id, nama, data_balikan_aktif, config_versi_aktif) \
             VALUES ($1, $2, $3, 1)",
        )
        .bind(id)
        .bind(nama_unik("prov"))
        .bind(data_balikan)
        .execute(&self.su)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO provider_config_versi (provider_id, versi, config_schema_version, \
             log_mode, dokumen, dibuat_oleh) VALUES ($1, 1, 1, $2, $3, $4)",
        )
        .bind(id)
        .bind(log_mode)
        .bind(json!({
            "metode": "POST",
            "body_template": "{\"NIK\":\"{{input.NIK}}\",\"TRESHOLD\":\"{{endpoint.TRESHOLD}}\"}",
            "base_url": "https://prov.example.test",
            "parameters": [{ "nama": "NIK" }, { "nama": "TRESHOLD" }],
            "respons_skema": {},
            "tls_trust": "system",
            "log_mode": log_mode
        }))
        .bind(self.pa)
        .execute(&self.su)
        .await
        .unwrap();
        id
    }

    async fn pool(&self, provider: Uuid) -> Uuid {
        let up = Uuid::now_v7();
        let dp = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, kuota_harian) \
             VALUES ($1, $2, $3, 1000)",
        )
        .bind(up)
        .bind(provider)
        .bind(nama_unik("up"))
        .execute(&self.su)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) VALUES ($1, $2, 1)",
        )
        .bind(dp)
        .bind(nama_unik("dp"))
        .execute(&self.su)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id) VALUES ($1, $2)",
        )
        .bind(dp)
        .bind(up)
        .execute(&self.su)
        .await
        .unwrap();
        dp
    }

    async fn client(&self) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query("INSERT INTO client (client_id, nama) VALUES ($1, $2)")
            .bind(id)
            .bind(nama_unik("klien"))
            .execute(&self.su)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, dibuat_oleh) \
             VALUES ($1, $2, 'api_key', '\\x01', $3)",
        )
        .bind(Uuid::now_v7())
        .bind(id)
        .bind(self.pa)
        .execute(&self.su)
        .await
        .unwrap();
        id
    }

    async fn jumlah_audit(&self, jenis: &str, id: Uuid) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM audit_konfigurasi WHERE entitas_jenis = $1 AND entitas_id = $2",
        )
        .bind(jenis)
        .bind(id)
        .fetch_one(&self.su)
        .await
        .unwrap()
    }
}

fn field_status() -> Value {
    json!({
        "nama_field": "status_penerima_manfaat", "tipe": "enum", "wajib": true,
        "nilai_diizinkan": ["sesuai", "tidak_sesuai"], "klasifikasi": "Internal",
        "kirim_ke_data_balikan": true, "deskripsi": "Status penerima manfaat sintetis"
    })
}

fn body(provider: Uuid, pool: Uuid) -> Value {
    json!({
        "provider_id": provider,
        "downstream_pool_id": pool,
        "parameter_literal": { "TRESHOLD": "90" },
        "log_mode": "metadata_only",
        "metadata_fields": [field_status()],
        "config_schema_version": 1
    })
}

fn kode_error(b: &Value) -> Vec<(String, String)> {
    b["errors"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|e| {
                    (
                        e["field"].as_str().unwrap_or_default().to_owned(),
                        e["kode"].as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
#[ignore = "butuh PostgreSQL; jalankan lewat `make -C backend uji-pg`"]
async fn endpoint_berversi_dan_versi_lama_utuh() {
    let u = siapkan().await;
    let provider = u.provider(true, "full_payload").await;
    let pool = u.pool(provider).await;

    let (s, b) = u
        .kirim(Method::POST, ENDPOINTS, Some(body(provider, pool)))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id: Uuid = b["endpoint_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(b["endpoint_config_version"], 1);
    assert_eq!(b["parameter_literal"], json!({ "TRESHOLD": "90" }));
    assert_eq!(b["metadata_fields"], json!([field_status()]));
    assert_eq!(b["log_mode"], "metadata_only");
    assert_eq!(b["aktif"], true);

    let (s, g) = u
        .kirim(Method::GET, &format!("{ENDPOINTS}/{id}"), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(g, b);

    let v1: String = sqlx::query_scalar(
        "SELECT dokumen::text FROM endpoint_config_versi WHERE downstream_endpoint_id = $1 AND versi = 1",
    )
    .bind(id)
    .fetch_one(&u.su)
    .await
    .unwrap();

    let mut ubah = body(provider, pool);
    ubah["parameter_literal"] = json!({ "TRESHOLD": "80" });
    ubah["metadata_fields"] = json!([
        field_status(),
        { "nama_field": "kode_wilayah", "tipe": "string", "wajib": false,
          "klasifikasi": "Terbatas", "kirim_ke_data_balikan": false, "deskripsi": "Kode wilayah" }
    ]);
    ubah["alasan"] = json!(ALASAN);
    let (s, b2) = u
        .kirim(Method::PUT, &format!("{ENDPOINTS}/{id}"), Some(ubah))
        .await;
    assert_eq!(s, StatusCode::OK, "{b2}");
    assert_eq!(b2["endpoint_config_version"], 2);
    assert_eq!(b2["parameter_literal"]["TRESHOLD"], "80");

    let v1_sesudah: String = sqlx::query_scalar(
        "SELECT dokumen::text FROM endpoint_config_versi WHERE downstream_endpoint_id = $1 AND versi = 1",
    )
    .bind(id)
    .fetch_one(&u.su)
    .await
    .unwrap();
    assert_eq!(v1, v1_sesudah, "V-03: versi lama byte-sama");

    // Field yang tidak berubah mempertahankan versinya; field baru versi 2.
    let versi: Vec<(String, i32)> = sqlx::query_as(
        "SELECT nama_field::text, versi FROM endpoint_metadata_field \
         WHERE downstream_endpoint_id = $1 ORDER BY nama_field",
    )
    .bind(id)
    .fetch_all(&u.su)
    .await
    .unwrap();
    assert_eq!(
        versi,
        vec![
            ("kode_wilayah".to_owned(), 2),
            ("status_penerima_manfaat".to_owned(), 1)
        ]
    );
    assert_eq!(u.jumlah_audit("downstream_endpoint", id).await, 2);

    let (s, daftar) = u.kirim(Method::GET, ENDPOINTS, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        daftar
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["endpoint_id"] == b["endpoint_id"])
    );
    let (s, _) = u
        .kirim(
            Method::GET,
            &format!("{ENDPOINTS}/{}", Uuid::now_v7()),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "butuh PostgreSQL; jalankan lewat `make -C backend uji-pg`"]
async fn endpoint_422_literal_mr1_log_mode_tanpa_baris() {
    let u = siapkan().await;
    let provider = u.provider(true, "metadata_only").await;
    let pool = u.pool(provider).await;
    let sebelum: i64 = sqlx::query_scalar("SELECT count(*) FROM downstream_endpoint")
        .fetch_one(&u.su)
        .await
        .unwrap();

    let mut b = body(provider, pool);
    b["parameter_literal"] = json!({});
    b["metadata_fields"] = json!([]);
    b["log_mode"] = json!("full_payload");
    let (s, r) = u.kirim(Method::POST, ENDPOINTS, Some(b)).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{r}");
    assert_eq!(r["kode"], "metadata_wajib_hilang", "M-R1");
    let e = kode_error(&r);
    assert!(e.contains(&("parameter_literal".into(), "literal_tidak_ada".into())));
    assert!(e.contains(&("log_mode".into(), "lebih_longgar_dari_provider".into())));
    assert!(e.contains(&("metadata_fields".into(), "metadata_wajib_hilang".into())));

    // Field tak dikenal dan nilai enum tertutup.
    let mut asing = body(provider, pool);
    asing["nama"] = json!("x");
    let (s, r) = u.kirim(Method::POST, ENDPOINTS, Some(asing)).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(kode_error(&r).contains(&("nama".into(), "tidak_dikenal".into())));
    let mut ringkas = body(provider, pool);
    ringkas["log_mode"] = json!("ringkas");
    let (s, _) = u.kirim(Method::POST, ENDPOINTS, Some(ringkas)).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);

    // Provider dan pool yang tidak ada.
    let (s, r) = u
        .kirim(
            Method::POST,
            ENDPOINTS,
            Some(body(Uuid::now_v7(), Uuid::now_v7())),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let e = kode_error(&r);
    assert!(e.contains(&("provider_id".into(), "tidak_ada".into())));
    assert!(e.contains(&("downstream_pool_id".into(), "tidak_ada".into())));

    let sesudah: i64 = sqlx::query_scalar("SELECT count(*) FROM downstream_endpoint")
        .fetch_one(&u.su)
        .await
        .unwrap();
    assert_eq!(sebelum, sesudah, "422 tidak menulis endpoint");

    // Update tanpa alasan.
    let (s, ok) = u
        .kirim(Method::POST, ENDPOINTS, Some(body(provider, pool)))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{ok}");
    let id = ok["endpoint_id"].as_str().unwrap();
    let (s, r) = u
        .kirim(
            Method::PUT,
            &format!("{ENDPOINTS}/{id}"),
            Some(body(provider, pool)),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        kode_error(&r),
        vec![("alasan".to_owned(), "wajib".to_owned())]
    );
}

#[tokio::test]
#[ignore = "butuh PostgreSQL; jalankan lewat `make -C backend uji-pg`"]
async fn set_client_endpoints_mengganti_penugasan_dan_kontrak_terbaca() {
    let u = siapkan().await;
    let provider = u.provider(true, "metadata_only").await;
    let pool = u.pool(provider).await;
    let client = u.client().await;

    let mut id = Vec::new();
    for _ in 0..2 {
        let (s, b) = u
            .kirim(Method::POST, ENDPOINTS, Some(body(provider, pool)))
            .await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        id.push(b["endpoint_id"].as_str().unwrap().parse::<Uuid>().unwrap());
    }
    let path = format!("/admin/v1/clients/{client}/endpoints");

    let (s, b) = u
        .kirim(
            Method::PUT,
            &path,
            Some(json!({ "endpoint_ids": [id[0], id[1]] })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["client_id"], json!(client));

    // AC-27: endpoint buatan admin API yang ditugaskan langsung punya kontrak
    // di store yang dipakai `GET /v1/endpoints/{id}/kontrak`.
    let api = PgPool::connect(&env("POOLROUTER_TEST_DATABASE_URL_API"))
        .await
        .unwrap();
    let store = PgKonfigurasiStore::new(api);
    let entri = store.muat(client, id[0], Instant::now()).await.unwrap();
    let kontrak = entri.kontrak.expect("kontrak tersedia");
    assert_eq!(kontrak.endpoint_config_version, 1);
    assert_eq!(
        kontrak.field_metadata[0].nama_field,
        "status_penerima_manfaat"
    );

    // Penggantian penuh: hanya id[1] yang tersisa.
    let (s, _) = u
        .kirim(Method::PUT, &path, Some(json!({ "endpoint_ids": [id[1]] })))
        .await;
    assert_eq!(s, StatusCode::OK);
    let tersisa: Vec<Uuid> = sqlx::query_scalar(
        "SELECT downstream_endpoint_id FROM client_endpoint WHERE client_id = $1",
    )
    .bind(client)
    .fetch_all(&u.su)
    .await
    .unwrap();
    assert_eq!(tersisa, vec![id[1]]);
    assert!(
        store
            .muat(client, id[0], Instant::now())
            .await
            .unwrap()
            .kontrak
            .is_none()
    );

    // Endpoint tak ada: 422 dengan indeks, penugasan tidak berubah.
    let (s, b) = u
        .kirim(
            Method::PUT,
            &path,
            Some(json!({ "endpoint_ids": [id[0], Uuid::now_v7()] })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        kode_error(&b),
        vec![("endpoint_ids/1".to_owned(), "tidak_ada".to_owned())]
    );

    // Himpunan kosong mencabut semua (U-7).
    let (s, _) = u
        .kirim(Method::PUT, &path, Some(json!({ "endpoint_ids": [] })))
        .await;
    assert_eq!(s, StatusCode::OK);
    let sisa: i64 = sqlx::query_scalar("SELECT count(*) FROM client_endpoint WHERE client_id = $1")
        .bind(client)
        .fetch_one(&u.su)
        .await
        .unwrap();
    assert_eq!(sisa, 0);
    assert_eq!(
        u.jumlah_audit("client", client).await,
        3,
        "satu audit per penugasan sukses"
    );

    let (s, _) = u
        .kirim(
            Method::PUT,
            &format!("/admin/v1/clients/{}/endpoints", Uuid::now_v7()),
            Some(json!({ "endpoint_ids": [] })),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let _ = &u.pool;
}
