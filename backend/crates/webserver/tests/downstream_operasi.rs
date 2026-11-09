//! Uji router operasi downstream dengan penyimpan dan kuota di memori
//! (ARC-63, ARC-65, ARC-66). Tidak ada PostgreSQL atau Redis: yang diuji adalah
//! urutan gerbang, pemetaan HTTP, baris yang ditulis, dan panggilan kuota.
//! Perilaku PostgreSQL/Redis sungguhan ada di `downstream_permintaan_pg.rs`.
//!
//! NIK di sini sintetis dan berawalan `00`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use chrono::Utc;
use poolrouter_core::kuota::store::HasilToken;
use poolrouter_core::retry::ErrorClass;
use poolrouter_core::status_api::StatusInternal;
use serde_json::{Value, json};
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

use poolrouter_webserver::api::downstream::HasilElemen;
use poolrouter_webserver::api::downstream_sesi::{
    DownstreamState, KunciIdempoten, OperasiDownstream, TUNGGU_TOKEN_SYNC, router,
};
use poolrouter_webserver::api::enums::{JenisObjek, StatusHasil};
use poolrouter_webserver::downstream::downstream_sesi::SesiKlien;
use poolrouter_webserver::downstream::downstream_sesi::fake::FakeSession;
use poolrouter_webserver::downstream::intake::{FieldInputKontrak, FieldMetadataKontrak};
use poolrouter_webserver::downstream::kuota_gerbang::fake::{FakeKuota, status_token};
use poolrouter_webserver::downstream::pemanggil::{PanggilanProvider, PemanggilProvider};
use poolrouter_webserver::downstream::pengenal::{Fut, GalatPengenal, PengenalDownstream};
use poolrouter_webserver::downstream::permintaan::fake::{FakePermintaan, baris_kosong};
use poolrouter_webserver::downstream::permintaan::{
    HasilPanggilan, Mode, PoolRute, RantaiEndpoint,
};

const NIK: &str = "0012345678901234";

struct PengenalTetap(SesiKlien);

impl PengenalDownstream for PengenalTetap {
    fn kenali<'a>(&'a self, _headers: &'a HeaderMap) -> Fut<'a, Result<SesiKlien, GalatPengenal>> {
        let k = self.0;
        Box::pin(async move { Ok(k) })
    }
}

struct PemanggilTetap {
    hasil: HasilPanggilan,
    jumlah: Mutex<u32>,
    /// Latensi tiruan provider.
    tunda: Duration,
}

impl PemanggilProvider for PemanggilTetap {
    fn panggil(
        &self,
        _p: PanggilanProvider,
    ) -> poolrouter_webserver::downstream::downstream_sesi::Fut<'_, HasilPanggilan> {
        Box::pin(async move {
            *self.jumlah.lock().unwrap() += 1;
            if !self.tunda.is_zero() {
                tokio::time::sleep(self.tunda).await;
            }
            self.hasil.clone()
        })
    }
}

fn rantai(endpoint: Uuid) -> RantaiEndpoint {
    RantaiEndpoint {
        endpoint_id: endpoint,
        provider_id: Uuid::now_v7(),
        downstream_pool_id: Uuid::now_v7(),
        prioritas: 1,
        ttl_antrean_detik: 86_400,
        provider_config_version: 1,
        endpoint_config_version: 1,
        log_mode: "metadata_only".to_owned(),
        wajib_log_nik: true,
        field_input: ["NIK", "NAMA_LGKP"]
            .into_iter()
            .map(|n| FieldInputKontrak {
                nama: n.to_owned(),
                tipe: "string".to_owned(),
                wajib: false,
            })
            .collect(),
        field_metadata: vec![
            FieldMetadataKontrak {
                nama_field: "status_penerima_manfaat".to_owned(),
                wajib: true,
                nilai_diizinkan: Some(vec!["sesuai".to_owned(), "tidak_sesuai".to_owned()]),
            },
            FieldMetadataKontrak {
                nama_field: "catatan_rahasia".to_owned(),
                wajib: false,
                nilai_diizinkan: None,
            },
        ],
        metadata_rahasia: vec!["catatan_rahasia".to_owned()],
        nonaktif: None,
        rute: vec![PoolRute {
            upstream_pool_id: Uuid::now_v7(),
            kuota_harian: 1000,
            margin_kuota: 0.01,
            kalender: false,
            periode_mulai: Utc::now(),
            periode_selesai: Utc::now(),
            porsi_sync_maks: 0.2,
            retry_rasio: 0.1,
            retry_min_per_jam: 20,
            parameter_dispatch: json!({}),
        }],
        limiter_burst: 20,
        limiter_isi_ulang_mikro: 1_000_000,
    }
}

struct Uji {
    app: Router,
    klien: SesiKlien,
    endpoint: Uuid,
    permintaan: Arc<FakePermintaan>,
    kuota: Arc<FakeKuota>,
    pemanggil: Option<Arc<PemanggilTetap>>,
}

impl Uji {
    fn baru() -> Self {
        Self::dengan(None)
    }

    fn dengan(hasil: Option<HasilPanggilan>) -> Self {
        Self::dengan_tunda(hasil, Duration::ZERO)
    }

    fn dengan_tunda(hasil: Option<HasilPanggilan>, tunda: Duration) -> Self {
        let klien = SesiKlien {
            client_id: Uuid::now_v7(),
            credential_id: Uuid::now_v7(),
        };
        let endpoint = Uuid::now_v7();
        let permintaan = FakePermintaan::baru();
        permintaan.atur_rantai(rantai(endpoint));
        let kuota = FakeKuota::baru();
        let pemanggil = hasil.map(|h| {
            Arc::new(PemanggilTetap {
                hasil: h,
                jumlah: Mutex::new(0),
                tunda,
            })
        });
        let operasi = Arc::new(OperasiDownstream {
            permintaan: permintaan.clone(),
            kuota: kuota.clone(),
            pemanggil: pemanggil.clone().map(|p| p as Arc<dyn PemanggilProvider>),
            kunci_idempoten: KunciIdempoten::baru(b"kunci-hmac-sintetis"),
            tunggu_token_sync: TUNGGU_TOKEN_SYNC,
        });
        let app = router(Arc::new(DownstreamState {
            pengenal: Arc::new(PengenalTetap(klien)),
            sesi: FakeSession::baru(),
            operasi,
        }));
        Self {
            app,
            klien,
            endpoint,
            permintaan,
            kuota,
            pemanggil,
        }
    }

    async fn kirim(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
        kunci: Option<&str>,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(k) = kunci {
            req = req.header("idempotency-key", k);
        }
        let body = match body {
            Some(b) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                Body::from(b)
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
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (
            status,
            headers,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn asinkron(&self, body: &Value, kunci: Option<&str>) -> (StatusCode, HeaderMap, Value) {
        let path = format!("/v1/endpoints/{}/async", self.endpoint);
        self.kirim(
            Method::POST,
            &path,
            Some(serde_json::to_vec(body).unwrap()),
            kunci,
        )
        .await
    }

    async fn sinkron(&self, body: &Value, kunci: Option<&str>) -> (StatusCode, HeaderMap, Value) {
        let path = format!("/v1/endpoints/{}/sync", self.endpoint);
        self.kirim(
            Method::POST,
            &path,
            Some(serde_json::to_vec(body).unwrap()),
            kunci,
        )
        .await
    }

    fn jumlah_baris(&self) -> usize {
        self.permintaan.semua().len()
    }
}

fn body_sah() -> Value {
    json!({
        "input": {"NIK": NIK, "NAMA_LGKP": "Budi Sintetis"},
        "metadata": {"status_penerima_manfaat": "sesuai", "catatan_rahasia": "rahasia-sintetis"}
    })
}

fn no_store(h: &HeaderMap) {
    assert_eq!(h.get(header::CACHE_CONTROL).unwrap(), "no-store");
}

// ------------------------------------------------------------------ async

#[tokio::test]
async fn async_valid_202_queued_dan_satu_baris() {
    // AC-10.
    let u = Uji::baru();
    let (s, h, v) = u.asinkron(&body_sah(), None).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    no_store(&h);
    assert_eq!(v["status"], "queued");
    let id: Uuid = serde_json::from_value(v["queue_id"].clone()).unwrap();
    let semua = u.permintaan.semua();
    assert_eq!(semua.len(), 1);
    assert_eq!(semua[0].0, id);
    assert_eq!(semua[0].1.status, StatusInternal::Queued);
    assert_eq!(semua[0].1.mode, Mode::Async);
    assert_eq!(u.kuota.hitung.lock().unwrap().intake, 1);
    assert_eq!(
        u.kuota.hitung.lock().unwrap().penolakan,
        0,
        "request diterima tidak memakai limiter penolakan"
    );
}

#[tokio::test]
async fn idempoten_sama_200_tanpa_baris_dan_tanpa_kuota_beda_422() {
    // AC-11.
    let u = Uji::baru();
    let (s1, _, v1) = u.asinkron(&body_sah(), Some("kunci-001")).await;
    assert_eq!(s1, StatusCode::ACCEPTED);
    let (s2, h2, v2) = u.asinkron(&body_sah(), Some("kunci-001")).await;
    assert_eq!(s2, StatusCode::OK);
    no_store(&h2);
    assert_eq!(v2["queue_id"], v1["queue_id"]);
    assert_eq!(v2["status"], "queued");
    assert_eq!(u.jumlah_baris(), 1);
    assert_eq!(
        u.kuota.hitung.lock().unwrap().intake,
        1,
        "replay tanpa cek kuota"
    );

    let mut beda = body_sah();
    beda["input"]["NAMA_LGKP"] = json!("Nama Lain Sintetis");
    let (s3, _, v3) = u.asinkron(&beda, Some("kunci-001")).await;
    assert_eq!(s3, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(v3["kode"], "idempotency_key_konflik");
    assert_eq!(u.jumlah_baris(), 1, "konflik tanpa baris");
    assert_eq!(u.kuota.hitung.lock().unwrap().penolakan, 1);
}

#[tokio::test]
async fn idempotency_key_tidak_sah_400_tanpa_baris() {
    let u = Uji::baru();
    let (s, _, v) = u.asinkron(&body_sah(), Some("ada spasi")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["kode"], "validasi_gagal");
    assert_eq!(u.jumlah_baris(), 0);
}

#[tokio::test]
async fn field_tak_dikenal_422_baris_rejected_tanpa_input_dan_tanpa_kuota() {
    // AC-13, ADR-0002 D6.
    let u = Uji::baru();
    let mut b = body_sah();
    b["input"]["TRESHOLD"] = json!(90);
    let (s, h, v) = u.asinkron(&b, Some("kunci-002")).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    no_store(&h);
    assert_eq!(v["kode"], "validasi_gagal");
    assert_eq!(v["errors"][0]["field"], "TRESHOLD");
    assert!(!v.to_string().contains(NIK), "NIK tidak ada di body error");
    let semua = u.permintaan.semua();
    assert_eq!(semua.len(), 1);
    let r = &semua[0].1;
    assert_eq!(r.status, StatusInternal::Rejected);
    assert_eq!(r.reject_reason, Some("validasi_gagal"));
    assert_eq!(r.reject_http, Some(422));
    assert!(r.input.is_none());
    assert!(r.idempoten.is_none(), "baris rejected tidak memegang kunci");
    assert!(
        r.metadata.get("catatan_rahasia").is_none(),
        "metadata Rahasia tidak disimpan"
    );
    assert_eq!(u.kuota.hitung.lock().unwrap().intake, 0);
}

#[tokio::test]
async fn body_bukan_json_400_baris_dengan_metadata_kosong() {
    let u = Uji::baru();
    let path = format!("/v1/endpoints/{}/async", u.endpoint);
    let (s, _, v) = u
        .kirim(Method::POST, &path, Some(b"{\"input\": ".to_vec()), None)
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["kode"], "validasi_gagal");
    let semua = u.permintaan.semua();
    assert_eq!(semua[0].1.reject_http, Some(400));
    assert_eq!(semua[0].1.metadata, json!({}));
}

#[tokio::test]
async fn limiter_habis_429_penolakan_berlebih_tanpa_baris() {
    // AC-13.
    let u = Uji::baru();
    *u.kuota.token_penolakan.lock().unwrap() = 0;
    let mut b = body_sah();
    b["input"]["PASSWORD"] = json!("x");
    let (s, h, v) = u.asinkron(&b, None).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(v["kode"], "penolakan_berlebih");
    assert_eq!(h.get(header::RETRY_AFTER).unwrap(), "2");
    let kunci: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(kunci.len(), 4, "body hanya empat field: {v}");
    assert_eq!(u.jumlah_baris(), 0);
}

#[tokio::test]
async fn kuota_habis_429_dengan_retry_after_dan_baris() {
    // AC-14.
    let u = Uji::baru();
    *u.kuota.kuota.lock().unwrap() = 0;
    let (s, h, v) = u.asinkron(&body_sah(), None).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(v["kode"], "kuota_habis");
    assert_eq!(h.get(header::RETRY_AFTER).unwrap(), "42");
    let semua = u.permintaan.semua();
    assert_eq!(semua[0].1.reject_reason, Some("kuota_habis"));
    assert_eq!(semua[0].1.reject_http, Some(429));
}

#[tokio::test]
async fn endpoint_nonaktif_403_pool_dan_provider_503_tanpa_retry_after() {
    // AC-15.
    for (jenis, http, teks) in [
        (JenisObjek::Endpoint, StatusCode::FORBIDDEN, "endpoint"),
        (
            JenisObjek::DownstreamPool,
            StatusCode::SERVICE_UNAVAILABLE,
            "downstream_pool",
        ),
        (
            JenisObjek::UpstreamPool,
            StatusCode::SERVICE_UNAVAILABLE,
            "upstream_pool",
        ),
        (
            JenisObjek::Provider,
            StatusCode::SERVICE_UNAVAILABLE,
            "provider",
        ),
    ] {
        let u = Uji::baru();
        u.permintaan.ubah_rantai(|r| r.nonaktif = Some(jenis));
        let (s, h, v) = u.asinkron(&body_sah(), None).await;
        assert_eq!(s, http, "{teks}");
        assert_eq!(v["kode"], "objek_nonaktif");
        assert_eq!(v["jenis_objek"], teks);
        assert!(h.get(header::RETRY_AFTER).is_none(), "{teks}");
        let semua = u.permintaan.semua();
        assert_eq!(semua[0].1.reject_reason, Some("objek_nonaktif"));
        assert_eq!(u.kuota.hitung.lock().unwrap().intake, 0);
    }
}

#[tokio::test]
async fn redis_mati_503_retry_after_5_tanpa_baris() {
    // AC-5.
    let u = Uji::baru();
    *u.kuota.mati.lock().unwrap() = true;
    let (s, h, v) = u.asinkron(&body_sah(), None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(v["kode"], "layanan_tidak_tersedia");
    assert_eq!(h.get(header::RETRY_AFTER).unwrap(), "5");
    assert_eq!(u.jumlah_baris(), 0);
}

#[tokio::test]
async fn endpoint_tidak_ditugaskan_404() {
    let u = Uji::baru();
    let path = format!("/v1/endpoints/{}/async", Uuid::now_v7());
    let (s, _, v) = u
        .kirim(
            Method::POST,
            &path,
            Some(serde_json::to_vec(&body_sah()).unwrap()),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(v["kode"], "tidak_ditemukan");
    assert_eq!(u.jumlah_baris(), 0);
}

// ------------------------------------------------------------------ polling dan cancel

#[tokio::test]
async fn polling_mengikuti_pemetaan_status_dan_tidak_menyentuh_kuota() {
    // AC-16, AC-17.
    let u = Uji::baru();
    let c = u.klien.client_id;
    let mut cases: Vec<(Uuid, &str)> = Vec::new();

    let id = Uuid::now_v7();
    let mut b = baris_kosong(c, Mode::Async, StatusInternal::Queued);
    b.attempt_count = 1;
    u.permintaan.sisipkan(id, b);
    cases.push((id, "processing"));

    let id = Uuid::now_v7();
    let mut b = baris_kosong(c, Mode::Async, StatusInternal::Done);
    b.attempt_count = 1;
    b.output = Some(json!({"NAMA_LGKP": {"status": "sesuai", "skor": 90}}));
    u.permintaan.sisipkan(id, b);
    cases.push((id, "done"));

    let id = Uuid::now_v7();
    let mut b = baris_kosong(c, Mode::Async, StatusInternal::Failed);
    b.attempt_count = 1;
    b.failure_reason = Some("provider_error_permanen".to_owned());
    u.permintaan.sisipkan(id, b);
    cases.push((id, "failed"));

    for (id, harap) in &cases {
        let (s, h, v) = u
            .kirim(Method::GET, &format!("/v1/requests/{id}"), None, None)
            .await;
        assert_eq!(s, StatusCode::OK);
        no_store(&h);
        assert_eq!(v["status"], *harap);
        assert_eq!(v.get("hasil").is_some(), *harap == "done", "{v}");
        assert_eq!(v.get("failure_reason").is_some(), *harap == "failed", "{v}");
        assert!(v.get("cancel_reason").is_none());
    }

    // rejected, milik client lain, dan tidak ada: 404 yang byte-sama.
    let ditolak = Uuid::now_v7();
    u.permintaan.sisipkan(
        ditolak,
        baris_kosong(c, Mode::Async, StatusInternal::Rejected),
    );
    let asing = Uuid::now_v7();
    u.permintaan.sisipkan(
        asing,
        baris_kosong(Uuid::now_v7(), Mode::Async, StatusInternal::Queued),
    );
    let mut badan = Vec::new();
    for id in [ditolak, asing, Uuid::now_v7()] {
        let (s, _, v) = u
            .kirim(Method::GET, &format!("/v1/requests/{id}"), None, None)
            .await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        badan.push(v);
    }
    assert!(badan.windows(2).all(|w| w[0] == w[1]));
    let h = u.kuota.hitung.lock().unwrap();
    assert_eq!(
        h.intake + h.penolakan + h.token,
        0,
        "polling tidak menyentuh Redis"
    );
}

#[tokio::test]
async fn cancel_queued_200_lalu_409_dan_processing_409() {
    // AC-18, AC-19.
    let u = Uji::baru();
    let (_, _, v) = u.asinkron(&body_sah(), None).await;
    let id = v["queue_id"].as_str().unwrap().to_owned();
    let (s, h, v) = u
        .kirim(
            Method::POST,
            &format!("/v1/requests/{id}/cancel"),
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    no_store(&h);
    assert_eq!(v["status"], "cancelled");
    assert_eq!(v["cancel_reason"], "dibatalkan_client");
    let semua = u.permintaan.semua();
    assert!(semua[0].1.input.is_none(), "P-02 menghapus input");

    let (s, _, v) = u
        .kirim(
            Method::POST,
            &format!("/v1/requests/{id}/cancel"),
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(v["kode"], "tidak_bisa_dibatalkan");
    assert_eq!(v["status_api"], "cancelled");

    // Menunggu retry (attempt_count > 0) adalah processing, tidak bisa dibatalkan.
    let retry = Uuid::now_v7();
    let mut b = baris_kosong(u.klien.client_id, Mode::Async, StatusInternal::Queued);
    b.attempt_count = 1;
    u.permintaan.sisipkan(retry, b);
    let (s, _, v) = u
        .kirim(
            Method::POST,
            &format!("/v1/requests/{retry}/cancel"),
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(v["status_api"], "processing");

    let (s, _, v) = u
        .kirim(
            Method::POST,
            &format!("/v1/requests/{}/cancel", Uuid::now_v7()),
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(v["kode"], "tidak_ditemukan");
}

// ------------------------------------------------------------------ sync

fn hasil_sesuai() -> HasilPanggilan {
    let mut hasil = BTreeMap::new();
    hasil.insert(
        "NAMA_LGKP".to_owned(),
        HasilElemen {
            status: StatusHasil::Sesuai,
            skor: Some(90),
        },
    );
    // Elemen yang tidak dikirim client tidak boleh muncul (U-43).
    hasil.insert(
        "TGL_LHR".to_owned(),
        HasilElemen {
            status: StatusHasil::Sesuai,
            skor: None,
        },
    );
    HasilPanggilan::Berhasil {
        http_status: 200,
        hasil,
    }
}

#[tokio::test]
async fn sync_tanpa_klien_provider_503_tanpa_baris_dan_tanpa_kuota() {
    let u = Uji::baru();
    let (s, h, v) = u.sinkron(&body_sah(), None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(v["kode"], "layanan_tidak_tersedia");
    assert_eq!(h.get(header::RETRY_AFTER).unwrap(), "5");
    assert_eq!(u.jumlah_baris(), 0);
    let hit = u.kuota.hitung.lock().unwrap();
    assert_eq!(hit.intake + hit.token + hit.tambah, 0);
}

#[tokio::test]
async fn sync_done_200_hanya_elemen_yang_dikirim() {
    // AC-27, AC-28.
    let u = Uji::dengan(Some(hasil_sesuai()));
    let (s, h, v) = u.sinkron(&body_sah(), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    no_store(&h);
    assert_eq!(v["status"], "done");
    assert_eq!(v["hasil"]["NAMA_LGKP"]["status"], "sesuai");
    assert!(v["hasil"].get("TGL_LHR").is_none());
    let semua = u.permintaan.semua();
    assert_eq!(semua.len(), 1);
    let r = &semua[0].1;
    assert_eq!(r.status, StatusInternal::Done);
    assert_eq!(
        r.history,
        vec![StatusInternal::InFlight, StatusInternal::Done]
    );
    assert_eq!(r.log.len(), 1);
    assert_eq!(r.log[0].1, Some("berhasil"));
    let hit = u.kuota.hitung.lock().unwrap();
    assert_eq!((hit.intake, hit.token, hit.tambah, hit.lepas), (1, 1, 1, 1));
    assert_eq!(*u.pemanggil.as_ref().unwrap().jumlah.lock().unwrap(), 1);
}

#[tokio::test]
async fn sync_gagal_dipetakan_502_504_dengan_request_id() {
    // AC-28.
    for (kelas, http, kode) in [
        (
            ErrorClass::Timeout,
            StatusCode::GATEWAY_TIMEOUT,
            "timeout_sync",
        ),
        (ErrorClass::Koneksi, StatusCode::BAD_GATEWAY, "retry_habis"),
        (
            ErrorClass::ProviderSementara,
            StatusCode::BAD_GATEWAY,
            "retry_habis",
        ),
        (
            ErrorClass::AuthDitolak,
            StatusCode::BAD_GATEWAY,
            "provider_error_permanen",
        ),
        (
            ErrorClass::Tls,
            StatusCode::BAD_GATEWAY,
            "provider_error_permanen",
        ),
    ] {
        let u = Uji::dengan(Some(HasilPanggilan::Gagal {
            kelas,
            http_status: Some(503),
        }));
        let (s, h, v) = u.sinkron(&body_sah(), None).await;
        assert_eq!(s, http, "{kelas:?}");
        assert_eq!(v["kode"], kode);
        assert!(h.get(header::RETRY_AFTER).is_none());
        let semua = u.permintaan.semua();
        assert_eq!(v["request_id"], json!(semua[0].0));
        assert_eq!(semua[0].1.status, StatusInternal::Failed);
        assert_eq!(semua[0].1.failure_reason.as_deref(), Some(kode));
    }
}

#[tokio::test]
async fn sync_dengan_idempotency_key_400_tanpa_baris_memakai_token_limiter() {
    // AC-25 (U-45h).
    let u = Uji::dengan(Some(hasil_sesuai()));
    let (s, _, v) = u.sinkron(&body_sah(), Some("kunci-sync")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["kode"], "idempotency_key_tidak_didukung");
    assert_eq!(u.jumlah_baris(), 0);
    assert_eq!(u.kuota.hitung.lock().unwrap().penolakan, 1);
    assert_eq!(*u.pemanggil.as_ref().unwrap().jumlah.lock().unwrap(), 0);
}

#[tokio::test]
async fn sync_porsi_habis_429_dengan_baris_dan_tanpa_panggilan() {
    // AC-26.
    let u = Uji::dengan(Some(hasil_sesuai()));
    *u.kuota.token_ditolak.lock().unwrap() =
        Some(HasilToken::PorsiSyncHabis(status_token(5_000_000)));
    let (s, h, v) = u.sinkron(&body_sah(), None).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(v["kode"], "porsi_sync_habis");
    assert_eq!(h.get(header::RETRY_AFTER).unwrap(), "5");
    let semua = u.permintaan.semua();
    assert_eq!(semua[0].1.reject_reason, Some("porsi_sync_habis"));
    assert_eq!(semua[0].1.mode, Mode::Sync);
    assert_eq!(*u.pemanggil.as_ref().unwrap().jumlah.lock().unwrap(), 0);
}

#[tokio::test]
async fn sync_kuota_token_habis_429_kuota_habis() {
    let u = Uji::dengan(Some(hasil_sesuai()));
    *u.kuota.token_ditolak.lock().unwrap() = Some(HasilToken::KuotaHabis(status_token(0)));
    let (s, h, v) = u.sinkron(&body_sah(), None).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(v["kode"], "kuota_habis");
    assert_eq!(h.get(header::RETRY_AFTER).unwrap(), "42");
    assert_eq!(*u.pemanggil.as_ref().unwrap().jumlah.lock().unwrap(), 0);
}

#[tokio::test]
async fn sync_client_putus_tidak_membatalkan_panggilan() {
    // AC-29: client memutus koneksi sebelum jawaban; panggilan upstream dan
    // transaksi kedua tetap selesai, slot dilepas.
    let u = Uji::dengan_tunda(Some(hasil_sesuai()), Duration::from_millis(100));
    let putus = tokio::time::timeout(Duration::from_millis(20), u.sinkron(&body_sah(), None)).await;
    assert!(putus.is_err(), "client harus memutus sebelum jawaban");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let semua = u.permintaan.semua();
    assert_eq!(semua.len(), 1);
    let r = &semua[0].1;
    assert_eq!(r.status, StatusInternal::Done);
    assert_eq!(r.log.len(), 1);
    assert_eq!(r.log[0].1, Some("berhasil"));
    assert_eq!(u.kuota.hitung.lock().unwrap().lepas, 1);
    assert_eq!(*u.pemanggil.as_ref().unwrap().jumlah.lock().unwrap(), 1);
}

// ------------------------------------------------------------------ AC-4

#[derive(Clone, Default)]
struct Penyangga(Arc<Mutex<Vec<u8>>>);

impl<'a> MakeWriter<'a> for Penyangga {
    type Writer = Penulis;
    fn make_writer(&'a self) -> Self::Writer {
        Penulis(self.0.clone())
    }
}

struct Penulis(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Penulis {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn nik_dan_nilai_input_tidak_masuk_log() {
    // AC-4: satu skenario sukses dan tiap jenis penolakan.
    let tulis = Penyangga::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(tulis.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let u = Uji::dengan(Some(hasil_sesuai()));
    u.asinkron(&body_sah(), Some("kunci-log")).await;
    u.sinkron(&body_sah(), None).await;
    let mut b = body_sah();
    b["input"]["TRESHOLD"] = json!(1);
    u.asinkron(&b, None).await;
    u.permintaan
        .ubah_rantai(|r| r.nonaktif = Some(JenisObjek::Provider));
    u.asinkron(&body_sah(), None).await;
    *u.kuota.token_penolakan.lock().unwrap() = 0;
    u.asinkron(&b, None).await;

    let log = String::from_utf8(tulis.0.lock().unwrap().clone()).unwrap();
    for rahasia in [NIK, "Budi Sintetis", "rahasia-sintetis"] {
        assert!(!log.contains(rahasia), "{rahasia} bocor ke log");
    }
}
