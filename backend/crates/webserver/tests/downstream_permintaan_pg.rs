//! Uji integrasi operasi downstream terhadap PostgreSQL dan Redis sungguhan
//! (ARC-63, ARC-65, ARC-66). Penyimpan berjalan sebagai `pr_api`, jadi grant
//! kolom ikut teruji; fixture dan pemeriksaan memakai superuser. Redis disiapkan
//! dengan `pr:siap` untuk `run_id`-nya.
//!
//! `#[ignore]`: hanya jalan lewat `make -C backend uji-pg` (target
//! `uji-pg-cargo`). Variabel lingkungan yang hilang membuat uji panic, bukan
//! lulus diam-diam. NIK sintetis berawalan `00`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use chrono::{DateTime, Utc};
use poolrouter_core::kuota::pemulihan::{
    GalatPemulihan, KeadaanPool, SumberRebuild, baca_run_id, bangun_ulang, kunci_antre,
};
use poolrouter_core::retry::ErrorClass;
use redis::aio::MultiplexedConnection;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use tower::ServiceExt;
use uuid::Uuid;

use poolrouter_adapter_provider::dokumen::DokumenProvider;
use poolrouter_core::admin::NilaiAman;
use poolrouter_core::secret::Brankas;
use poolrouter_webserver::api::downstream::HasilElemen;
use poolrouter_webserver::api::downstream_sesi::{
    DownstreamState, KunciIdempoten, OperasiDownstream, TUNGGU_TOKEN_SYNC, router,
};
use poolrouter_webserver::api::enums::StatusHasil;
use poolrouter_webserver::downstream::downstream_sesi::SesiKlien;
use poolrouter_webserver::downstream::downstream_sesi::fake::FakeSession;
use poolrouter_webserver::downstream::downstream_sesi::{
    Fut as DFut, Hasil, PgSessionDownstream, SessionDownstream,
};
use poolrouter_webserver::downstream::kuota_gerbang::KuotaRedisMalas;
use poolrouter_webserver::downstream::pemanggil::{
    HasilProvider, Kirim, KlienProvider, PanggilanProvider, PanggilanSiap, PemanggilHttp,
    PemanggilProvider, SumberPg, Transport,
};
use poolrouter_webserver::downstream::pengenal::{Fut, GalatPengenal, PengenalDownstream};
use poolrouter_webserver::downstream::permintaan::{HasilPanggilan, PgPermintaan};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const NIK: &str = "0012345678901234";

fn env(nama: &str) -> String {
    std::env::var(nama)
        .unwrap_or_else(|_| panic!("{nama} belum diset; jalankan lewat `make -C backend uji-pg`"))
}

async fn kolam(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(8)
        .connect(url)
        .await
        .unwrap()
}

struct Kosong;

impl SumberRebuild for Kosong {
    async fn keadaan(&self, _: DateTime<Utc>) -> Result<Vec<KeadaanPool>, GalatPemulihan> {
        Ok(vec![])
    }
}

async fn redis_koneksi() -> MultiplexedConnection {
    redis::Client::open(env("POOLROUTER_TEST_REDIS"))
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap()
}

/// Redis kosong yang ditandai siap untuk `run_id`-nya (§5a.2).
async fn redis_siap() -> MultiplexedConnection {
    let mut c = redis_koneksi().await;
    redis::cmd("FLUSHALL")
        .query_async::<()>(&mut c)
        .await
        .unwrap();
    let run_id = baca_run_id(&mut c).await.unwrap();
    bangun_ulang(&mut c, &run_id, &Kosong, Utc::now())
        .await
        .unwrap();
    c
}

struct PengenalTetap(SesiKlien);

impl PengenalDownstream for PengenalTetap {
    fn kenali<'a>(&'a self, _h: &'a HeaderMap) -> Fut<'a, Result<SesiKlien, GalatPengenal>> {
        let k = self.0;
        Box::pin(async move { Ok(k) })
    }
}

struct MockProvider {
    hasil: HasilPanggilan,
    /// Jumlah panggilan yang benar-benar dikirim.
    jumlah: Arc<Mutex<u32>>,
    /// Latensi tetap mock dan latensi yang terukur di dalam mock (AC-32).
    tunda: Duration,
    latensi: Arc<Mutex<Vec<Duration>>>,
}

impl PemanggilProvider for MockProvider {
    fn siapkan(&self, _p: PanggilanProvider) -> DFut<'_, Hasil<PanggilanSiap>> {
        let hasil = self.hasil.clone();
        let jumlah = self.jumlah.clone();
        let latensi = self.latensi.clone();
        let tunda = self.tunda;
        Box::pin(async move {
            let kirim: Kirim = Box::new(move |_batas| -> DFut<'static, HasilProvider> {
                Box::pin(async move {
                    *jumlah.lock().unwrap() += 1;
                    if !tunda.is_zero() {
                        let t = Instant::now();
                        tokio::time::sleep(tunda).await;
                        latensi.lock().unwrap().push(t.elapsed());
                    }
                    HasilProvider {
                        hasil,
                        response_payload: None,
                    }
                })
            });
            Ok(PanggilanSiap {
                request_payload: None,
                kirim,
            })
        })
    }
}

/// KEK sintetis uji dalam bentuk 32 byte mentah (bukan KEK lingkungan mana pun).
const KEK_UJI: &str = "kek-uji-arc66-sync-jangan-dipake";

/// wiremock berjalan di http biasa; produksi memakai `TransportTls`.
struct TransportUji;

impl Transport for TransportUji {
    fn klien(&self, _dok: &DokumenProvider) -> Result<KlienProvider, String> {
        Ok(KlienProvider::tanpa_tls_untuk_uji(reqwest::Client::new()))
    }
}

fn hasil_sesuai() -> HasilPanggilan {
    let mut hasil = BTreeMap::new();
    hasil.insert(
        "NAMA_LGKP".to_owned(),
        HasilElemen {
            status: StatusHasil::Sesuai,
            skor: Some(90),
        },
    );
    HasilPanggilan::Berhasil {
        http_status: 200,
        hasil,
    }
}

struct Uji {
    app: Router,
    su: PgPool,
    api: PgPool,
    endpoint: Uuid,
    upstream: Uuid,
    client_id: Uuid,
    kred: Uuid,
    admin: Uuid,
    mock: Arc<MockProvider>,
}

impl Uji {
    async fn baru(kuota_harian: i32, hasil: HasilPanggilan) -> Self {
        Self::baru_tunda(kuota_harian, hasil, Duration::ZERO).await
    }

    async fn baru_tunda(kuota_harian: i32, hasil: HasilPanggilan, tunda: Duration) -> Self {
        let template = r#"{\"NIK\":\"{{input.NIK}}\",\"NAMA_LGKP\":\"{{input.NAMA_LGKP}}\",\"TRESHOLD\":\"{{endpoint.TRESHOLD}}\"}"#;
        let dokumen = format!("{{\"body_template\": \"{template}\"}}");
        let mock = Arc::new(MockProvider {
            hasil,
            jumlah: Arc::new(Mutex::new(0)),
            tunda,
            latensi: Arc::new(Mutex::new(Vec::new())),
        });
        let m = mock.clone();
        Self::bangun(
            kuota_harian,
            "metadata_only",
            &dokumen,
            move |_| m as Arc<dyn PemanggilProvider>,
            mock,
        )
        .await
    }

    /// Fixture rantai lengkap; `pemanggil` menerima pool `pr_api`.
    async fn bangun(
        kuota_harian: i32,
        log_mode: &str,
        dokumen: &str,
        pemanggil: impl FnOnce(PgPool) -> Arc<dyn PemanggilProvider>,
        mock: Arc<MockProvider>,
    ) -> Self {
        let su = kolam(&env("POOLROUTER_TEST_DATABASE_URL_SU")).await;
        let api = kolam(&env("POOLROUTER_TEST_DATABASE_URL_API")).await;
        let uid = Uuid::now_v7().simple().to_string();
        let admin = Uuid::now_v7();
        let client_id = Uuid::now_v7();
        let kred = Uuid::now_v7();
        let provider = Uuid::now_v7();
        let upstream = Uuid::now_v7();
        let pool = Uuid::now_v7();
        let endpoint = Uuid::now_v7();
        let dokumen = dokumen.replace('\'', "''");
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "INSERT INTO admin_user (pengguna_id, username, password_hash) \
               VALUES ('{admin}', 'arc63-{uid}', 'x');
             INSERT INTO client (client_id, nama) VALUES ('{client_id}', 'klien-{uid}');
             INSERT INTO provider (provider_id, nama, config_versi_aktif) \
               VALUES ('{provider}', 'provider-{uid}', 1);
             INSERT INTO provider_config_versi \
               (provider_id, versi, config_schema_version, log_mode, dokumen, dibuat_oleh) \
               VALUES ('{provider}', 1, 1, '{log_mode}', '{dokumen}'::jsonb, '{admin}');
             INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, kuota_harian, \
                                        margin_kuota, parameter_dispatch) \
               VALUES ('{upstream}', '{provider}', 'upstream-{uid}', {kuota_harian}, 0.001, \
                       '{{\"rate\": {{\"tps\": {{\"t_mikro\": 1000, \"burst\": 1000}}}}}}'::jsonb);
             INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) \
               VALUES ('{pool}', 'pool-{uid}', 1);
             INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id) \
               VALUES ('{pool}', '{upstream}');
             INSERT INTO downstream_endpoint \
               (downstream_endpoint_id, nama, provider_id, downstream_pool_id, config_versi_aktif) \
               VALUES ('{endpoint}', 'endpoint-{uid}', '{provider}', '{pool}', 1);
             INSERT INTO endpoint_config_versi \
               (downstream_endpoint_id, versi, config_schema_version, dokumen, dibuat_oleh) \
               VALUES ('{endpoint}', 1, 1, \
                       '{{\"parameter_literal\": {{\"TRESHOLD\": \"80\"}}}}', '{admin}');
             INSERT INTO endpoint_metadata_field \
               (downstream_endpoint_id, nama_field, tipe, wajib, nilai_diizinkan, klasifikasi, \
                kirim_ke_data_balikan, deskripsi, versi) \
               VALUES ('{endpoint}', 'status_penerima_manfaat', 'enum', true, \
                       '[\"sesuai\",\"tidak_sesuai\"]'::jsonb, 'Internal', false, 'Sintetis.', 1), \
                      ('{endpoint}', 'catatan', 'string', false, NULL, 'Rahasia', false, \
                       'Sintetis.', 1);
             INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, dibuat_oleh) \
               VALUES ('{kred}', '{client_id}', 'api_key', '\\x00'::bytea, '{admin}');
             INSERT INTO client_endpoint (client_id, downstream_endpoint_id) \
               VALUES ('{client_id}', '{endpoint}');"
        )))
        .execute(&su)
        .await
        .unwrap();

        let operasi = Arc::new(OperasiDownstream {
            permintaan: PgPermintaan::baru(api.clone()),
            kuota: KuotaRedisMalas::baru(
                redis::Client::open(env("POOLROUTER_TEST_REDIS")).unwrap(),
            ),
            pemanggil: Some(pemanggil(api.clone())),
            kunci_idempoten: KunciIdempoten::baru(b"kunci-hmac-sintetis-pg"),
            tunggu_token_sync: TUNGGU_TOKEN_SYNC,
        });
        let app = router(Arc::new(DownstreamState {
            pengenal: Arc::new(PengenalTetap(SesiKlien {
                client_id,
                credential_id: kred,
            })),
            sesi: FakeSession::baru(),
            operasi,
        }));
        Self {
            app,
            su,
            api,
            endpoint,
            upstream,
            client_id,
            kred,
            admin,
            mock,
        }
    }

    async fn kirim(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        kunci: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(k) = kunci {
            req = req.header("idempotency-key", k);
        }
        let body = match body {
            Some(b) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                Body::from(serde_json::to_vec(b).unwrap())
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
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn asinkron(&self, body: &Value, kunci: Option<&str>) -> (StatusCode, Value) {
        let path = format!("/v1/endpoints/{}/async", self.endpoint);
        self.kirim(Method::POST, &path, Some(body), kunci).await
    }

    async fn sinkron(&self, body: &Value) -> (StatusCode, Value) {
        let path = format!("/v1/endpoints/{}/sync", self.endpoint);
        self.kirim(Method::POST, &path, Some(body), None).await
    }

    async fn skalar_i64(&self, sql: &'static str, id: Uuid) -> i64 {
        sqlx::query_scalar::<_, i64>(sql)
            .bind(id)
            .fetch_one(&self.su)
            .await
            .unwrap()
    }

    async fn jumlah_request(&self) -> i64 {
        self.skalar_i64(
            "SELECT count(*) FROM request WHERE client_id = $1",
            self.client_id,
        )
        .await
    }
}

fn body_sah() -> Value {
    json!({
        "input": {"NIK": NIK, "NAMA_LGKP": "Budi Sintetis"},
        "metadata": {"status_penerima_manfaat": "sesuai", "catatan": "rahasia-sintetis"}
    })
}

fn uuid_dari(v: &Value, kunci: &str) -> Uuid {
    serde_json::from_value(v[kunci].clone()).unwrap()
}

/// AC-10: satu transaksi, satu baris `queued` dan satu baris history.
#[tokio::test]
#[ignore]
async fn async_queued_satu_baris_dan_history() {
    redis_siap().await;
    let u = Uji::baru(1000, hasil_sesuai()).await;
    let (s, v) = u.asinkron(&body_sah(), None).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let id = uuid_dari(&v, "queue_id");
    assert_eq!(id.get_version_num(), 7, "queue_id UUIDv7");
    let r = sqlx::query(
        "SELECT status, mode, attempt_count, dispatch_key IS NOT NULL AS ada_kunci, \
                extract(epoch FROM ttl_expires_at - received_at)::int8 AS ttl, \
                input_payload IS NOT NULL AS ada_input \
           FROM request WHERE request_id = $1",
    )
    .bind(id)
    .fetch_one(&u.su)
    .await
    .unwrap();
    assert_eq!(r.get::<String, _>("status"), "queued");
    assert_eq!(r.get::<String, _>("mode"), "async");
    assert_eq!(r.get::<i32, _>("attempt_count"), 0);
    assert!(r.get::<bool, _>("ada_kunci"));
    assert!(r.get::<bool, _>("ada_input"));
    assert_eq!(r.get::<i64, _>("ttl"), 86_400);
    let h = u
        .skalar_i64(
            "SELECT count(*) FROM request_status_history WHERE request_id = $1",
            id,
        )
        .await;
    assert_eq!(h, 1);
}

/// AC-11 dan AC-12: replay tidak menulis dan tidak menyentuh counter intake;
/// sidik beda 422; dua request bersamaan dengan kunci sama, satu baris.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn idempoten_replay_konflik_dan_balapan() {
    let mut r = redis_siap().await;
    let u = Uji::baru(100_000, hasil_sesuai()).await;
    let (s1, v1) = u.asinkron(&body_sah(), Some("kunci-replay")).await;
    assert_eq!(s1, StatusCode::ACCEPTED);
    let antre_awal: Option<i64> = redis::cmd("GET")
        .arg(kunci_antre(&u.upstream.to_string()))
        .query_async(&mut r)
        .await
        .unwrap();
    let (s2, v2) = u.asinkron(&body_sah(), Some("kunci-replay")).await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(v2["queue_id"], v1["queue_id"]);
    let antre_akhir: Option<i64> = redis::cmd("GET")
        .arg(kunci_antre(&u.upstream.to_string()))
        .query_async(&mut r)
        .await
        .unwrap();
    assert_eq!(
        antre_awal, antre_akhir,
        "replay tidak menyentuh counter intake"
    );

    let mut beda = body_sah();
    beda["input"]["NAMA_LGKP"] = json!("Nama Lain Sintetis");
    let (s3, v3) = u.asinkron(&beda, Some("kunci-replay")).await;
    assert_eq!(s3, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(v3["kode"], "idempotency_key_konflik");
    assert_eq!(u.jumlah_request().await, 1);

    for i in 0..50 {
        let kunci = format!("kunci-balap-{i}");
        let b = body_sah();
        let (a, c) = tokio::join!(u.asinkron(&b, Some(&kunci)), u.asinkron(&b, Some(&kunci)));
        assert!(a.0.is_success() && c.0.is_success(), "{a:?} {c:?}");
        assert_eq!(a.1["queue_id"], c.1["queue_id"], "iterasi {i}");
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM request WHERE client_id = $1 AND idempotency_key = $2",
        )
        .bind(u.client_id)
        .bind(&kunci)
        .fetch_one(&u.su)
        .await
        .unwrap();
        assert_eq!(n, 1, "iterasi {i}");
    }
}

/// AC-13: baris `rejected` tanpa payload, tanpa metadata Rahasia, tanpa kunci.
#[tokio::test]
#[ignore]
async fn penolakan_menulis_baris_rejected_tanpa_payload() {
    redis_siap().await;
    let u = Uji::baru(1000, hasil_sesuai()).await;
    let mut b = body_sah();
    b["input"]["TRESHOLD"] = json!(10);
    let (s, v) = u.asinkron(&b, Some("kunci-tolak")).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let r = sqlx::query(
        "SELECT status, reject_reason, reject_http_status, input_payload IS NULL AS tanpa_input, \
                idempotency_key IS NULL AS tanpa_kunci, metadata ? 'catatan' AS ada_rahasia, \
                metadata ->> 'status_penerima_manfaat' AS status_meta \
           FROM request WHERE client_id = $1",
    )
    .bind(u.client_id)
    .fetch_one(&u.su)
    .await
    .unwrap();
    assert_eq!(r.get::<String, _>("status"), "rejected");
    assert_eq!(r.get::<String, _>("reject_reason"), "validasi_gagal");
    assert_eq!(r.get::<i32, _>("reject_http_status"), 422);
    assert!(r.get::<bool, _>("tanpa_input"));
    assert!(r.get::<bool, _>("tanpa_kunci"));
    assert!(!r.get::<bool, _>("ada_rahasia"));
    assert_eq!(r.get::<String, _>("status_meta"), "sesuai");

    // Body bukan JSON: 400 dengan metadata `{}`.
    let path = format!("/v1/endpoints/{}/async", u.endpoint);
    let req = Request::builder()
        .method(Method::POST)
        .uri(&path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{\"input\": {\"NIK\": \"1\", \"NIK\": \"2\"}}"))
        .unwrap();
    let res = u.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(u.jumlah_request().await, 2);
}

/// AC-16 sampai AC-19: polling, pembatalan dengan purge P-02, dan dua
/// pembatalan bersamaan yang menghasilkan tepat satu transisi.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn polling_cancel_dan_cancel_bersamaan() {
    redis_siap().await;
    let u = Uji::baru(100_000, hasil_sesuai()).await;
    for i in 0..20 {
        let (_, v) = u.asinkron(&body_sah(), None).await;
        let id = uuid_dari(&v, "queue_id");
        let (s, p) = u
            .kirim(Method::GET, &format!("/v1/requests/{id}"), None, None)
            .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(p["status"], "queued");

        let path = format!("/v1/requests/{id}/cancel");
        let (a, b) = tokio::join!(
            u.kirim(Method::POST, &path, None, None),
            u.kirim(Method::POST, &path, None, None)
        );
        let mut kode = [a.0, b.0];
        kode.sort();
        assert_eq!(kode, [StatusCode::OK, StatusCode::CONFLICT], "iterasi {i}");
        let r = sqlx::query(
            "SELECT status, cancel_reason, cancelled_by, webhook_status, \
                    input_payload IS NULL AS terhapus, payload_purged_at IS NOT NULL AS dipurge, \
                    (SELECT count(*) FROM request_status_history h WHERE h.request_id = r.request_id) \
                      AS history \
               FROM request r WHERE request_id = $1",
        )
        .bind(id)
        .fetch_one(&u.su)
        .await
        .unwrap();
        assert_eq!(r.get::<String, _>("status"), "cancelled");
        assert_eq!(r.get::<String, _>("cancel_reason"), "dibatalkan_client");
        assert_eq!(r.get::<String, _>("cancelled_by"), "client");
        assert_eq!(r.get::<String, _>("webhook_status"), "tidak_dikirim");
        assert!(r.get::<bool, _>("terhapus"));
        assert!(r.get::<bool, _>("dipurge"));
        assert_eq!(r.get::<i64, _>("history"), 2, "iterasi {i}");
    }

    // Request rejected tidak bisa dipolling: 404 sama dengan id yang tidak ada.
    let mut b = body_sah();
    b["input"]["PASSWORD"] = json!("x");
    u.asinkron(&b, None).await;
    let ditolak: Uuid = sqlx::query_scalar(
        "SELECT request_id FROM request WHERE client_id = $1 AND status = 'rejected'",
    )
    .bind(u.client_id)
    .fetch_one(&u.su)
    .await
    .unwrap();
    let (s1, b1) = u
        .kirim(Method::GET, &format!("/v1/requests/{ditolak}"), None, None)
        .await;
    let (s2, b2) = u
        .kirim(
            Method::GET,
            &format!("/v1/requests/{}", Uuid::now_v7()),
            None,
            None,
        )
        .await;
    assert_eq!((s1, s2), (StatusCode::NOT_FOUND, StatusCode::NOT_FOUND));
    assert_eq!(b1, b2);
}

/// AC-27, AC-28: log percobaan sebelum panggilan, lalu satu transaksi untuk
/// log dan status akhir.
#[tokio::test]
#[ignore]
async fn sync_done_dan_timeout_menulis_log_lengkap() {
    redis_siap().await;
    let u = Uji::baru(1000, hasil_sesuai()).await;
    let (s, v) = u.sinkron(&body_sah()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["hasil"]["NAMA_LGKP"]["status"], "sesuai");
    let id = uuid_dari(&v, "request_id");
    let r = sqlx::query(
        "SELECT r.status, r.attempt_count, r.final_kode_transaksi = l.kode_transaksi AS final_cocok, \
                l.status_akses, l.nik = $2 AS nik_tercatat, \
                (SELECT count(*) FROM request_status_history h WHERE h.request_id = r.request_id) \
                  AS history \
           FROM request r JOIN transaction_log l ON l.request_id = r.request_id \
          WHERE r.request_id = $1",
    )
    .bind(id)
    .bind(NIK)
    .fetch_one(&u.su)
    .await
    .unwrap();
    assert_eq!(r.get::<String, _>("status"), "done");
    assert_eq!(r.get::<i32, _>("attempt_count"), 1);
    assert!(r.get::<bool, _>("final_cocok"));
    assert_eq!(r.get::<String, _>("status_akses"), "berhasil");
    assert!(r.get::<bool, _>("nik_tercatat"));
    assert_eq!(r.get::<i64, _>("history"), 2);

    let u = Uji::baru(
        1000,
        HasilPanggilan::Gagal {
            kelas: ErrorClass::Timeout,
            http_status: None,
        },
    )
    .await;
    let (s, v) = u.sinkron(&body_sah()).await;
    assert_eq!(s, StatusCode::GATEWAY_TIMEOUT, "{v}");
    let id = uuid_dari(&v, "request_id");
    let r = sqlx::query(
        "SELECT r.status, r.failure_reason, l.status_akses, l.error_class \
           FROM request r JOIN transaction_log l ON l.request_id = r.request_id \
          WHERE r.request_id = $1",
    )
    .bind(id)
    .fetch_one(&u.su)
    .await
    .unwrap();
    assert_eq!(r.get::<String, _>("status"), "failed");
    assert_eq!(r.get::<String, _>("failure_reason"), "timeout_sync");
    assert_eq!(r.get::<String, _>("status_akses"), "gagal_jaringan");
    assert_eq!(r.get::<String, _>("error_class"), "timeout");
}

/// AC-30 (M-02, M-03): `kuota_efektif = N`, N + 5 sync berturut-turut, paling
/// banyak N panggilan, dan baris log sama dengan jumlah panggilan.
#[tokio::test]
#[ignore]
async fn sync_tidak_melewati_kuota_efektif() {
    redis_siap().await;
    // floor(5 * (1 - 0,001)) = 4.
    let n = 4;
    let u = Uji::baru(5, hasil_sesuai()).await;
    for _ in 0..(n + 5) {
        let (s, v) = u.sinkron(&body_sah()).await;
        assert!(
            s == StatusCode::OK || s == StatusCode::TOO_MANY_REQUESTS,
            "{s} {v}"
        );
    }
    let panggilan = i64::from(*u.mock.jumlah.lock().unwrap());
    assert!(panggilan <= n, "{panggilan} panggilan untuk kuota {n}");
    let log = u
        .skalar_i64(
            "SELECT count(*) FROM transaction_log WHERE client_id = $1",
            u.client_id,
        )
        .await;
    assert_eq!(log, panggilan, "M-03: satu baris log per panggilan");
}

/// AC-5: `pr:siap` tidak ada → 503 `layanan_tidak_tersedia`, tanpa baris.
#[tokio::test]
#[ignore]
async fn redis_belum_siap_503_tanpa_baris() {
    let mut c = redis_koneksi().await;
    redis::cmd("FLUSHALL")
        .query_async::<()>(&mut c)
        .await
        .unwrap();
    let u = Uji::baru(1000, hasil_sesuai()).await;
    let (s, v) = u.asinkron(&body_sah(), None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    assert_eq!(v["kode"], "layanan_tidak_tersedia");
    let (s, _) = u.sinkron(&body_sah()).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(u.jumlah_request().await, 0);
    assert_eq!(*u.mock.jumlah.lock().unwrap(), 0);
}

/// AC-31 (NFR-02, smoke/relatif): 1.000 enqueue valid, p95 dilaporkan.
#[tokio::test]
#[ignore]
async fn smoke_p95_enqueue_async() {
    redis_siap().await;
    let u = Uji::baru(1_000_000, hasil_sesuai()).await;
    let mut durasi: Vec<Duration> = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let t = Instant::now();
        let (s, _) = u.asinkron(&body_sah(), None).await;
        durasi.push(t.elapsed());
        assert_eq!(s, StatusCode::ACCEPTED);
    }
    durasi.sort();
    let p95 = durasi[949];
    println!(
        "ARC-63 AC-31 smoke/relatif: 1000 enqueue async, p95 = {:.1} ms, p50 = {:.1} ms",
        p95.as_secs_f64() * 1000.0,
        durasi[499].as_secs_f64() * 1000.0
    );
}

/// AC-32 (NFR-01, smoke/relatif): overhead sync = p95 end-to-end dikurangi
/// rata-rata latensi mock (metode `docs/perf/rencana-uji-beban.md`).
#[tokio::test]
#[ignore]
async fn smoke_overhead_sync() {
    redis_siap().await;
    let u = Uji::baru_tunda(1_000_000, hasil_sesuai(), Duration::from_millis(20)).await;
    let mut durasi: Vec<Duration> = Vec::with_capacity(200);
    for _ in 0..200 {
        let t = Instant::now();
        let (s, v) = u.sinkron(&body_sah()).await;
        durasi.push(t.elapsed());
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    durasi.sort();
    let p95 = durasi[189].as_secs_f64() * 1000.0;
    let latensi = u.mock.latensi.lock().unwrap().clone();
    let rata = latensi.iter().map(Duration::as_secs_f64).sum::<f64>() * 1000.0
        / latensi.len().max(1) as f64;
    println!(
        "ARC-66 AC-32 smoke/relatif: 200 sync, p95 end-to-end = {p95:.1} ms, \
         rata-rata mock = {rata:.1} ms, overhead = {:.1} ms",
        p95 - rata
    );
}

// ------------------------------------------- ARC-66: jalur sync dengan PemanggilHttp

const SECRET_PG: &str = "rahasia-provider-sintetis-pg";

fn respons_dukcapil() -> Value {
    json!({"content": [{
        "NAMA_LGKP": "Sesuai (90)",
        "TMPT_LHR": "Tidak Sesuai",
        "UID": SECRET_PG
    }]})
}

/// Fixture `full_payload` dengan pemanggil produksi (`PemanggilHttp` +
/// `SumberPg` sebagai `pr_api`) ke wiremock, plus secret terenkripsi KEK uji.
/// Mengembalikan penanda `{{secret.<nama>}}` yang diharapkan di payload log.
async fn uji_http(server: &MockServer, kuota_harian: i32) -> (Uji, String) {
    let nama = format!("USER_ID_{}", Uuid::now_v7().simple());
    let penanda = format!("{{{{secret.{nama}}}}}");
    let template = r#"{"NIK":"{{input.NIK}}","NAMA_LGKP":"{{input.NAMA_LGKP}}","USER_ID":"SECRET","transactionId":"{{system.kode_transaksi}}","TRESHOLD":"{{endpoint.TRESHOLD}}"}"#
        .replace("SECRET", &penanda);
    let dokumen = json!({
        "metode": "POST",
        "base_url": format!("{}/api", server.uri()),
        "body_template": template,
        "tls_trust": "system",
        "respons_skema": {
            "parser": "status_skor",
            "literal": [
                {"teks": "Sesuai", "status": "sesuai"},
                {"teks": "Tidak Sesuai", "status": "tidak_sesuai"}
            ],
            "parameter_output": [{"nama": "NAMA_LGKP"}, {"nama": "TMPT_LHR"}]
        },
        "parameters": [],
        "log_mode": "full_payload"
    })
    .to_string();
    let cadangan = Arc::new(MockProvider {
        hasil: hasil_sesuai(),
        jumlah: Arc::new(Mutex::new(0)),
        tunda: Duration::ZERO,
        latensi: Arc::new(Mutex::new(Vec::new())),
    });
    let u = Uji::bangun(
        kuota_harian,
        "full_payload",
        &dokumen,
        |api| {
            Arc::new(PemanggilHttp::baru(
                SumberPg::baru(api, Brankas::dari_isi_kek(KEK_UJI).unwrap()),
                TransportUji,
            )) as Arc<dyn PemanggilProvider>
        },
        cadangan,
    )
    .await;
    let amplop = Brankas::dari_isi_kek(KEK_UJI)
        .unwrap()
        .enkripsi(&nama, &NilaiAman::baru(SECRET_PG.to_owned()))
        .unwrap();
    sqlx::query(
        "INSERT INTO provider_secret (nama_secret, secret_ciphertext, secret_dek_terbungkus, \
                secret_nonce, kek_id, diubah_oleh) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&nama)
    .bind(amplop.ciphertext.clone())
    .bind(amplop.dek_terbungkus.clone())
    .bind(amplop.nonce.to_vec())
    .bind(amplop.kek_id.clone())
    .bind(u.admin)
    .execute(&u.su)
    .await
    .unwrap();
    (u, penanda)
}

/// AC-27, AC-28 end-to-end: render dengan secret dari `provider_secret`,
/// `request_payload`/`response_payload` mengikuti `full_payload` dengan secret
/// sebagai penanda, `transactionId` = `kode_transaksi` log, dan kontrak
/// endpoint memuat field input dari template (ADR-0002 D6).
#[tokio::test]
#[ignore]
async fn sync_http_done_menulis_payload_log_tersamar() {
    redis_siap().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(respons_dukcapil()))
        .mount(&server)
        .await;
    let (u, penanda) = uji_http(&server, 1000).await;
    let (s, v) = u.sinkron(&body_sah()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        v["hasil"]["NAMA_LGKP"],
        json!({"status": "sesuai", "skor": 90})
    );
    assert!(v["hasil"].get("TMPT_LHR").is_none(), "{v}");
    let id = uuid_dari(&v, "request_id");

    let r = sqlx::query(
        "SELECT r.status, r.final_kode_transaksi = l.kode_transaksi AS final_cocok, \
                l.kode_transaksi, l.log_mode, l.status_akses, l.provider_http_status, \
                l.responded_at IS NOT NULL AS ada_respons, l.request_payload, l.response_payload \
           FROM request r JOIN transaction_log l ON l.request_id = r.request_id \
          WHERE r.request_id = $1",
    )
    .bind(id)
    .fetch_one(&u.su)
    .await
    .unwrap();
    assert_eq!(r.get::<String, _>("status"), "done");
    assert!(r.get::<bool, _>("final_cocok"));
    assert_eq!(r.get::<String, _>("log_mode"), "full_payload");
    assert_eq!(r.get::<String, _>("status_akses"), "berhasil");
    assert_eq!(r.get::<Option<i32>, _>("provider_http_status"), Some(200));
    assert!(r.get::<bool, _>("ada_respons"));
    let kode: Uuid = r.get("kode_transaksi");
    assert_eq!(kode.get_version_num(), 7);
    let rp = r
        .get::<Option<sqlx::types::Json<Value>>, _>("request_payload")
        .unwrap()
        .0;
    assert_eq!(rp["body"]["USER_ID"], penanda.as_str());
    assert_eq!(rp["body"]["transactionId"], kode.to_string());
    assert!(!rp.to_string().contains(SECRET_PG), "{rp}");
    let resp = r
        .get::<Option<sqlx::types::Json<Value>>, _>("response_payload")
        .unwrap()
        .0;
    assert_eq!(resp["content"][0]["UID"], penanda.as_str());
    assert!(!resp.to_string().contains(SECRET_PG), "{resp}");

    let diterima = server.received_requests().await.unwrap();
    assert_eq!(diterima.len(), 1);
    let body: Value = serde_json::from_slice(&diterima[0].body).unwrap();
    assert_eq!(body["USER_ID"], SECRET_PG);
    assert_eq!(body["transactionId"], kode.to_string());
    assert_eq!(body["TRESHOLD"], "80");

    let k = PgSessionDownstream::baru(u.api.clone())
        .kontrak(
            SesiKlien {
                client_id: u.client_id,
                credential_id: u.kred,
            },
            u.endpoint,
        )
        .await
        .unwrap();
    let nama: Vec<&str> = k.field_input.iter().map(|f| f.nama.as_str()).collect();
    assert_eq!(nama, ["NIK", "NAMA_LGKP"]);
}

/// AC-30 dengan pemanggil produksi: `kuota_efektif = N`, N + 5 sync, paling
/// banyak N panggilan yang diterima mock dan baris log = panggilan.
#[tokio::test]
#[ignore]
async fn sync_http_tidak_melewati_kuota_efektif() {
    redis_siap().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(respons_dukcapil()))
        .mount(&server)
        .await;
    let n = 4;
    let (u, _) = uji_http(&server, 5).await;
    for _ in 0..(n + 5) {
        let (s, v) = u.sinkron(&body_sah()).await;
        assert!(
            s == StatusCode::OK || s == StatusCode::TOO_MANY_REQUESTS,
            "{s} {v}"
        );
    }
    let panggilan = i64::try_from(server.received_requests().await.unwrap().len()).unwrap();
    assert!(panggilan <= n, "{panggilan} panggilan untuk kuota {n}");
    let log = u
        .skalar_i64(
            "SELECT count(*) FROM transaction_log WHERE client_id = $1",
            u.client_id,
        )
        .await;
    assert_eq!(log, panggilan, "M-03: satu baris log per panggilan");
}

/// R-07 (`chk_tls`): kegagalan TLS tanpa respons HTTP menulis `responded_at`
/// NULL. Sebelum ARC-66 kolom itu selalu diisi, sehingga constraint menolak
/// transaksi kedua dan sync menjawab 503 alih-alih 502.
#[tokio::test]
#[ignore]
async fn sync_tls_502_responded_at_null() {
    redis_siap().await;
    let u = Uji::baru(
        1000,
        HasilPanggilan::Gagal {
            kelas: ErrorClass::Tls,
            http_status: None,
        },
    )
    .await;
    let (s, v) = u.sinkron(&body_sah()).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY, "{v}");
    assert_eq!(v["kode"], "provider_error_permanen");
    let id = uuid_dari(&v, "request_id");
    let r = sqlx::query(
        "SELECT r.status, l.status_akses, l.error_class, l.responded_at IS NULL AS tanpa_respons \
           FROM request r JOIN transaction_log l ON l.request_id = r.request_id \
          WHERE r.request_id = $1",
    )
    .bind(id)
    .fetch_one(&u.su)
    .await
    .unwrap();
    assert_eq!(r.get::<String, _>("status"), "failed");
    assert_eq!(r.get::<String, _>("status_akses"), "gagal_jaringan");
    assert_eq!(r.get::<String, _>("error_class"), "tls");
    assert!(r.get::<bool, _>("tanpa_respons"));
}

/// AC-32 (NFR-01, smoke/relatif) dengan pemanggil produksi: overhead sync =
/// p95 end-to-end dikurangi latensi tetap wiremock (20 ms). Dicatat, bukan
/// target NFR.
#[tokio::test]
#[ignore]
async fn smoke_overhead_sync_http() {
    redis_siap().await;
    let server = MockServer::start().await;
    let tunda = Duration::from_millis(20);
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(respons_dukcapil())
                .set_delay(tunda),
        )
        .mount(&server)
        .await;
    let (u, _) = uji_http(&server, 1_000_000).await;
    let mut durasi: Vec<Duration> = Vec::with_capacity(200);
    for _ in 0..200 {
        let t = Instant::now();
        let (s, v) = u.sinkron(&body_sah()).await;
        durasi.push(t.elapsed());
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    durasi.sort();
    let p95 = durasi[189].as_secs_f64() * 1000.0;
    let mock = tunda.as_secs_f64() * 1000.0;
    println!(
        "ARC-66 AC-32 smoke/relatif (PemanggilHttp + wiremock): 200 sync, \
         p95 end-to-end = {p95:.1} ms, latensi mock = {mock:.1} ms, overhead = {:.1} ms",
        p95 - mock
    );
}
