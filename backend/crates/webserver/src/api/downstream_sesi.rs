//! Badan handler operasi downstream (ARC-187, ARC-63, ARC-65, ARC-66).
//!
//! Bentuk wire ditetapkan `downstream-api.yaml`, yang dibangkitkan dari anotasi
//! di [`crate::api::downstream`]; modul ini hanya mengisi badannya.
//!
//! Urutan penolakan setelah autentikasi mengikuti `intake.rs` dan `gerbang.rs`:
//! body (400), input/metadata (422), `Idempotency-Key`, replay idempoten,
//! `objek_nonaktif` (403/503), lalu kuota (429). Setiap penolakan mengambil satu
//! token limiter penolakan client; baris `rejected` hanya ditulis selama token
//! ada dan hanya untuk penolakan yang punya `reject_reason`.
//!
//! Setiap jawaban lewat [`jawab`], [`problem`], atau `nirs_cache`, jadi
//! `Cache-Control: no-store` ada di setiap 2xx dan setiap problem (AC-2).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use poolrouter_core::kuota::store::{
    HasilIntake, HasilPenolakan, HasilToken, KonteksKuota, ModeJendela, PermintaanToken, detik_naik,
};
use poolrouter_core::kuota::sumber_pg::jendela_rate;
use poolrouter_core::status_api::{StatusApi, StatusInternal, StatusTerpetakan, status_api};
use serde_json::Value;
use uuid::Uuid;

use super::downstream::{
    EndpointKontrak, EnqueueResponse, FieldMetadata, HasilElemen, RequestStatus, SyncResult,
};
use super::enums::{ApiStatus, CancelReason, FailureReason, JenisObjek};
use super::problem::{JumlahNamaTidakCocok, Problem, ProblemKodeDown, ValidasiErrorDown};
use crate::downstream::downstream_sesi::{GalatDownstream, SesiKlien, SessionDownstream};
use crate::downstream::gerbang::{self, Jawaban};
use crate::downstream::intake::{self, TolakIntake};
use crate::downstream::kuota_gerbang::GerbangKuota;
use crate::downstream::layers::ApiError;
use crate::downstream::pemanggil::{PanggilanProvider, PemanggilProvider};
use crate::downstream::pengenal::{GalatPengenal, PengenalDownstream};
use crate::downstream::permintaan::{
    Antrean, BarisIdempoten, BarisStatus, HasilAntre, HasilBatal, HasilPanggilan, Mode, MulaiSync,
    Penolakan, PenyimpanPermintaan, PoolRute, RantaiEndpoint, SelesaiSync, TulisSync, akhir_sync,
};

/// State of the downstream API router: the credential resolver, the contract
/// store and the request operations. The admin pool never reaches this router
/// (ADR-0001 7.2).
pub struct DownstreamState {
    pub pengenal: Arc<dyn PengenalDownstream>,
    pub sesi: Arc<dyn SessionDownstream>,
    pub operasi: Arc<OperasiDownstream>,
}

/// Kunci HMAC sidik idempoten (`POOLROUTER_IDEMPOTENCY_HMAC_KEY`). `Debug`
/// tidak pernah mencetak isinya.
#[derive(Clone)]
pub struct KunciIdempoten(Arc<[u8]>);

impl KunciIdempoten {
    pub fn baru(isi: &[u8]) -> Self {
        Self(Arc::from(isi))
    }

    pub fn isi(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for KunciIdempoten {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KunciIdempoten(<tersembunyi>)")
    }
}

/// Dependensi operasi submit, polling, dan pembatalan.
pub struct OperasiDownstream {
    pub permintaan: Arc<dyn PenyimpanPermintaan>,
    pub kuota: Arc<dyn GerbangKuota>,
    /// Klien provider jalur sync. `None` sampai ARC-64 dipasang: sync menjawab
    /// 503 tanpa menulis apa pun.
    pub pemanggil: Option<Arc<dyn PemanggilProvider>>,
    pub kunci_idempoten: KunciIdempoten,
    /// Batas tunggu token sync (§5a.4, default 200 ms).
    pub tunggu_token_sync: Duration,
}

/// Batas tunggu token sync bawaan (ADR-0001 rev 15 §5a.4).
pub const TUNGGU_TOKEN_SYNC: Duration = Duration::from_millis(200);

impl OperasiDownstream {
    /// Operasi di memori untuk uji: penyimpan dan kuota palsu, tanpa klien
    /// provider, kunci HMAC sintetis.
    pub fn palsu() -> Arc<Self> {
        Arc::new(Self {
            permintaan: crate::downstream::permintaan::fake::FakePermintaan::baru(),
            kuota: crate::downstream::kuota_gerbang::fake::FakeKuota::baru(),
            pemanggil: None,
            kunci_idempoten: KunciIdempoten::baru(b"kunci-hmac-sintetis-uji"),
            tunggu_token_sync: TUNGGU_TOKEN_SYNC,
        })
    }
}

/// `Cache-Control: no-store` on a response that already carries its body.
fn nirs_cache(mut res: Response) -> Response {
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

fn jawab(status: StatusCode, body: impl IntoResponse) -> Response {
    nirs_cache((status, body).into_response())
}

/// One `application/problem+json` answer with `no-store` attached.
fn problem(kode: ProblemKodeDown, status: StatusCode) -> Response {
    let p = Problem::new(kode, status.as_u16());
    let body = serde_json::to_vec(&p).unwrap_or_else(|_| b"{}".to_vec());
    nirs_cache(
        (
            status,
            [(header::CONTENT_TYPE, "application/problem+json")],
            body,
        )
            .into_response(),
    )
}

/// A store failure becomes the documented problem; the cause stays in the logs.
fn galat(e: GalatDownstream) -> Response {
    match e {
        GalatDownstream::EndpointTidakDikenal | GalatDownstream::RequestTidakDikenal => {
            problem(ProblemKodeDown::TidakDitemukan, StatusCode::NOT_FOUND)
        }
        GalatDownstream::ObjekNonaktif => {
            problem(ProblemKodeDown::ObjekNonaktif, StatusCode::FORBIDDEN)
        }
        GalatDownstream::Layanan => {
            let mut res = problem(
                ProblemKodeDown::LayananTidakTersedia,
                StatusCode::SERVICE_UNAVAILABLE,
            );
            res.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
            res
        }
    }
}

/// Authenticates the caller. Bearer OAuth2 and API key are both accepted, at
/// every operation, by the same resolver (AC-3).
#[allow(clippy::result_large_err)]
async fn masuk(state: &DownstreamState, headers: &HeaderMap) -> Result<SesiKlien, Response> {
    match state.pengenal.kenali(headers).await {
        Ok(k) => Ok(k),
        // The credential store did not answer: fail closed with 503, never a
        // false 401 that would make the client discard a valid credential.
        Err(GalatPengenal::Layanan) => {
            let mut res = problem(
                ProblemKodeDown::LayananTidakTersedia,
                StatusCode::SERVICE_UNAVAILABLE,
            );
            res.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
            Err(res)
        }
        Err(GalatPengenal::Ditolak) => {
            let mut res = problem(
                ProblemKodeDown::TidakTerautentikasi,
                StatusCode::UNAUTHORIZED,
            );
            res.headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
            Err(res)
        }
    }
}

/// An `{endpoint_id}` or `{queue_id}` that is not a UUID is simply unknown, so
/// it gets the same 404 as an id that exists but belongs to someone else.
#[allow(clippy::result_large_err)]
fn path_id(mentah: &str) -> Result<Uuid, Response> {
    Uuid::parse_str(mentah)
        .map_err(|_| problem(ProblemKodeDown::TidakDitemukan, StatusCode::NOT_FOUND))
}

// ---------------------------------------------------------------------------
// getEndpointKontrak
// ---------------------------------------------------------------------------

#[allow(clippy::result_large_err)]
pub(crate) async fn get_endpoint_kontrak(
    State(state): State<Arc<DownstreamState>>,
    headers: HeaderMap,
    Path(mentah): Path<String>,
) -> Response {
    let klien = match masuk(&state, &headers).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    let endpoint = match path_id(&mentah) {
        Ok(id) => id,
        Err(r) => return r,
    };
    match state.sesi.kontrak(klien, endpoint).await {
        Err(e) => galat(e),
        Ok(k) => jawab(
            StatusCode::OK,
            Json(EndpointKontrak {
                endpoint_id: k.endpoint_id,
                endpoint_config_version: i64::from(k.config_versi_aktif),
                // The provider template of the active version supplies these;
                // the metadata table does not carry them.
                field_input: Vec::new(),
                field_metadata: k
                    .input
                    .into_iter()
                    .map(|b| FieldMetadata {
                        nama_field: b.nama,
                        tipe: b.tipe,
                        wajib: b.wajib,
                        nilai_diizinkan: b
                            .nilai_diizinkan
                            .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok()),
                        deskripsi: b.deskripsi,
                        versi: i64::from(b.versi),
                    })
                    .collect(),
            }),
        ),
    }
}

// ---------------------------------------------------------------------------
// Bersama: penolakan, status API, kuota
// ---------------------------------------------------------------------------

/// Penolakan setelah autentikasi. Setiap varian mengambil satu token limiter
/// penolakan client (ADR-0001 rev 15 §4.1, ADR-0002 rev 5 D6).
enum Tolakan {
    Gerbang(Jawaban),
    Nonaktif(JenisObjek),
}

impl Tolakan {
    /// `(reject_reason, reject_http_status)` bila penolakan ini ditulis sebagai
    /// baris `rejected`. Idempotensi dan limiter tidak punya `reject_reason`
    /// (`chk_reject_http`), jadi tanpa baris.
    fn baris(&self) -> Option<(&'static str, u16)> {
        match self {
            Self::Gerbang(Jawaban::ValidasiGagal { status, .. }) => {
                Some(("validasi_gagal", *status))
            }
            Self::Gerbang(Jawaban::MetadataTidakValid { .. }) => {
                Some(("metadata_tidak_valid", 422))
            }
            Self::Gerbang(Jawaban::KuotaHabis { .. }) => Some(("kuota_habis", 429)),
            Self::Gerbang(Jawaban::PorsiSyncHabis { .. }) => Some(("porsi_sync_habis", 429)),
            Self::Gerbang(_) => None,
            Self::Nonaktif(JenisObjek::Endpoint) => Some(("objek_nonaktif", 403)),
            Self::Nonaktif(_) => Some(("objek_nonaktif", 503)),
        }
    }

    fn respons(self) -> Response {
        match self {
            Self::Gerbang(j) => nirs_cache(j.into_api_error().into_response()),
            // 403 untuk endpoint (keputusan kebijakan); 503 tanpa Retry-After
            // untuk pool/provider (§4.1).
            Self::Nonaktif(jenis) => {
                let status = if jenis == JenisObjek::Endpoint {
                    StatusCode::FORBIDDEN
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                };
                nirs_cache(
                    ApiError::new(status, ProblemKodeDown::ObjekNonaktif)
                        .jenis_objek(jenis)
                        .into_response(),
                )
            }
        }
    }
}

fn layanan_503() -> Response {
    nirs_cache(ApiError::unavailable().into_response())
}

/// Satu penolakan lewat limiter: token ada → baris `rejected` (bila punya
/// `reject_reason`) dan respons penolakan; token habis → 429
/// `penolakan_berlebih` tanpa baris. Redis mati → 503 tanpa baris.
async fn tolak(
    op: &OperasiDownstream,
    klien: SesiKlien,
    rantai: &Arc<RantaiEndpoint>,
    mode: Mode,
    received_at: DateTime<Utc>,
    t: Tolakan,
    metadata: Option<&Value>,
) -> Response {
    let limiter = match op
        .kuota
        .penolakan(
            klien.client_id.to_string(),
            rantai.limiter_isi_ulang_mikro,
            rantai.limiter_burst,
        )
        .await
    {
        Ok(h) => h,
        Err(_) => return layanan_503(),
    };
    if matches!(limiter, HasilPenolakan::Dibatasi { .. }) {
        return Tolakan::Gerbang(Jawaban::PenolakanBerlebih {
            retry_after: limiter.retry_after_detik().unwrap_or(1),
        })
        .respons();
    }
    if let Some((reason, http)) = t.baris() {
        // Body yang tidak terbaca tidak punya metadata (chk_null_body_tak_terbaca).
        let metadata = if reason == "validasi_gagal" && http == 400 {
            Value::Object(serde_json::Map::new())
        } else {
            rantai.metadata_untuk_tolak(metadata)
        };
        let tolakan = Penolakan {
            request_id: Uuid::now_v7(),
            mode,
            received_at,
            reject_reason: reason,
            http_status: http,
            metadata,
        };
        if op
            .permintaan
            .simpan_ditolak(klien, rantai.clone(), tolakan)
            .await
            .is_err()
        {
            return layanan_503();
        }
    }
    t.respons()
}

/// Status API dari status internal dan `attempt_count` (fungsi `core`, AC-7).
fn status_api_dari(status: StatusInternal, attempt_count: i32) -> Option<ApiStatus> {
    match status_api(status, attempt_count) {
        StatusTerpetakan::Status(s) => Some(match s {
            StatusApi::Queued => ApiStatus::Queued,
            StatusApi::Processing => ApiStatus::Processing,
            StatusApi::Done => ApiStatus::Done,
            StatusApi::Failed => ApiStatus::Failed,
            StatusApi::Expired => ApiStatus::Expired,
            StatusApi::Cancelled => ApiStatus::Cancelled,
        }),
        StatusTerpetakan::TidakBisaDipolling => None,
    }
}

fn enum_dari<T: serde::de::DeserializeOwned>(teks: Option<&str>) -> Option<T> {
    teks.and_then(|t| serde_json::from_value(Value::String(t.to_owned())).ok())
}

fn hasil_dari(output: Option<&Value>) -> Option<BTreeMap<String, HasilElemen>> {
    output.and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// Body `RequestStatus`: `hasil` hanya untuk `done`, `failure_reason` hanya
/// untuk `failed`, `cancel_reason` hanya untuk `cancelled` (AC-16).
fn request_status(b: &BarisStatus) -> Option<RequestStatus> {
    let status = status_api_dari(b.status, b.attempt_count)?;
    Some(RequestStatus {
        queue_id: b.request_id,
        status,
        hasil: if status == ApiStatus::Done {
            hasil_dari(b.output.as_ref())
        } else {
            None
        },
        failure_reason: if status == ApiStatus::Failed {
            enum_dari::<FailureReason>(b.failure_reason.as_deref())
        } else {
            None
        },
        cancel_reason: if status == ApiStatus::Cancelled {
            enum_dari::<CancelReason>(b.cancel_reason.as_deref())
        } else {
            None
        },
    })
}

fn konteks(rute: &PoolRute, requested_at: DateTime<Utc>) -> KonteksKuota {
    KonteksKuota {
        pool_id: rute.upstream_pool_id.to_string(),
        periode_mulai: rute.periode_mulai,
        requested_at,
        mode: if rute.kalender {
            ModeJendela::Kalender
        } else {
            ModeJendela::Bergulir
        },
        kuota_efektif: rute.kuota_efektif(),
    }
}

/// `Retry-After` untuk `kuota_habis`: sisa periode (`kalender`) atau waktu
/// sampai ember tertua keluar jendela (`bergulir_24j`), dibaca dari Redis.
async fn retry_after_kuota(
    op: &OperasiDownstream,
    rute: &PoolRute,
    sekarang: DateTime<Utc>,
) -> Option<u64> {
    if rute.kalender {
        let sisa = (rute.periode_selesai - sekarang).num_milliseconds().max(0);
        return Some(detik_naik(sisa.saturating_mul(1_000)));
    }
    op.kuota
        .baca(konteks(rute, sekarang))
        .await
        .ok()
        .map(|b| b.retry_after_detik.max(1))
}

/// Kuota intake (K-12) atas rute aktif, urut `upstream_pool_id`: rute pertama
/// yang lolos dipakai. `Err(Some(detik))`: kuota habis di semua rute, dengan
/// `Retry-After`; `Err(None)`: Redis tidak bisa memutuskan (503).
async fn ambil_kuota_intake<'r>(
    op: &OperasiDownstream,
    klien: SesiKlien,
    rantai: &'r RantaiEndpoint,
    sync: bool,
    sekarang: DateTime<Utc>,
) -> Result<&'r PoolRute, Option<u64>> {
    for rute in &rantai.rute {
        match op
            .kuota
            .intake(konteks(rute, sekarang), sync, klien.client_id.to_string())
            .await
        {
            Err(_) => return Err(None),
            Ok(HasilIntake::Lolos { .. }) => return Ok(rute),
            Ok(_) => {}
        }
    }
    let pertama = rantai.rute.first().ok_or(None)?;
    Err(retry_after_kuota(op, pertama, sekarang).await)
}

/// Isi `NIK` yang sah untuk kolom `transaction_log.nik` (C-01): 16 digit.
fn nik_dari(input: &Value) -> Option<String> {
    input
        .get("NIK")
        .and_then(Value::as_str)
        .filter(|s| s.len() == 16 && s.bytes().all(|b| b.is_ascii_digit()))
        .map(str::to_owned)
}

/// Validasi body bersama kedua jalur, urutan `intake.rs`: JSON tanpa kunci
/// ganda (400), envelope, `input` (422), `metadata` (422), lalu NIK wajib bila
/// provider mewajibkan log NIK (C-01).
fn validasi_body(
    rantai: &RantaiEndpoint,
    body: &[u8],
) -> Result<(Value, Value), (Jawaban, Option<Value>)> {
    let nilai = intake::baca_body(body).map_err(|t| (gerbang::jawaban_untuk(t), None))?;
    let Some(objek) = nilai.as_object() else {
        return Err((
            gerbang::jawaban_untuk(TolakIntake::InputTidakValid {
                errors: vec![ValidasiErrorDown {
                    field: "input".to_owned(),
                    kode: "tipe_salah".to_owned(),
                    batas: None,
                }],
                jumlah_nama_tidak_cocok: None,
            }),
            None,
        ));
    };
    let input = objek.get("input").cloned().unwrap_or(Value::Null);
    let metadata = objek.get("metadata").cloned().unwrap_or(Value::Null);
    let asing: Vec<ValidasiErrorDown> = objek
        .keys()
        .filter(|k| k.as_str() != "input" && k.as_str() != "metadata")
        .filter(|k| {
            (1..=64).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
        .map(|k| ValidasiErrorDown {
            field: k.clone(),
            kode: "field_tak_dikenal".to_owned(),
            batas: None,
        })
        .collect();
    let jumlah_asing = objek
        .keys()
        .filter(|k| k.as_str() != "input" && k.as_str() != "metadata")
        .count();
    if jumlah_asing > 0 {
        let tak_wajar = u32::try_from(jumlah_asing - asing.len()).unwrap_or(u32::MAX);
        return Err((
            gerbang::jawaban_untuk(TolakIntake::InputTidakValid {
                errors: asing,
                jumlah_nama_tidak_cocok: JumlahNamaTidakCocok::try_from(tak_wajar).ok(),
            }),
            Some(metadata),
        ));
    }
    if let Err(t) = intake::validasi_input(&input, &rantai.field_input) {
        return Err((gerbang::jawaban_untuk(t), Some(metadata)));
    }
    if let Err(t) = intake::validasi_metadata(&metadata, &rantai.field_metadata) {
        return Err((gerbang::jawaban_untuk(t), Some(metadata)));
    }
    if rantai.wajib_log_nik && nik_dari(&input).is_none() {
        return Err((
            gerbang::jawaban_untuk(TolakIntake::InputTidakValid {
                errors: vec![ValidasiErrorDown {
                    field: "NIK".to_owned(),
                    kode: "wajib".to_owned(),
                    batas: None,
                }],
                jumlah_nama_tidak_cocok: None,
            }),
            Some(metadata),
        ));
    }
    Ok((input, metadata))
}

/// Ambil `Referensi-Client`; nilai yang tidak sah menjadi 422 `validasi_gagal`.
fn referensi_client(headers: &HeaderMap) -> Result<Option<String>, Jawaban> {
    let Some(v) = headers.get("referensi-client") else {
        return Ok(None);
    };
    let salah = || Jawaban::ValidasiGagal {
        status: 422,
        detail: None,
        errors: vec![ValidasiErrorDown {
            field: "Referensi-Client".to_owned(),
            kode: "tidak_valid".to_owned(),
            batas: Some(128.0),
        }],
        jumlah_nama_tidak_cocok: None,
    };
    let s = v.to_str().map_err(|_| salah())?;
    // varchar(128): ditolak, bukan dipotong, supaya dua referensi tidak menyatu.
    if s.len() > 128 {
        return Err(salah());
    }
    Ok(Some(s.to_owned()))
}

fn header_idempoten(headers: &HeaderMap) -> Result<Option<&str>, ()> {
    match headers.get("idempotency-key") {
        None => Ok(None),
        Some(v) => v.to_str().map(Some).map_err(|_| ()),
    }
}

/// Memuat rantai endpoint; 404 untuk endpoint yang tidak ditugaskan.
#[allow(clippy::result_large_err)]
async fn muat(
    state: &DownstreamState,
    klien: SesiKlien,
    endpoint: Uuid,
    sekarang: DateTime<Utc>,
) -> Result<Arc<RantaiEndpoint>, Response> {
    match state
        .operasi
        .permintaan
        .muat_rantai(klien, endpoint, sekarang)
        .await
    {
        Err(e) => Err(galat(e)),
        Ok(None) => Err(galat(GalatDownstream::EndpointTidakDikenal)),
        Ok(Some(r)) => Ok(Arc::new(r)),
    }
}

// ---------------------------------------------------------------------------
// enqueueAsync (ARC-63)
// ---------------------------------------------------------------------------

pub(crate) async fn enqueue_async(
    State(state): State<Arc<DownstreamState>>,
    headers: HeaderMap,
    Path(mentah): Path<String>,
    body: Bytes,
) -> Response {
    let klien = match masuk(&state, &headers).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    let endpoint = match path_id(&mentah) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let received_at = Utc::now();
    let rantai = match muat(&state, klien, endpoint, received_at).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    let op = state.operasi.as_ref();
    let mode = Mode::Async;

    let referensi = match referensi_client(&headers) {
        Ok(r) => r,
        Err(j) => {
            return tolak(
                op,
                klien,
                &rantai,
                mode,
                received_at,
                Tolakan::Gerbang(j),
                None,
            )
            .await;
        }
    };
    let (input, metadata) = match validasi_body(&rantai, &body) {
        Ok(v) => v,
        Err((j, meta)) => {
            return tolak(
                op,
                klien,
                &rantai,
                mode,
                received_at,
                Tolakan::Gerbang(j),
                meta.as_ref(),
            )
            .await;
        }
    };
    let kunci = match header_idempoten(&headers)
        .map_err(|_| TolakIntake::IdempotencyKeyTidakSah)
        .and_then(|h| intake::baca_idempotency_key(h, true))
    {
        Ok(k) => k,
        Err(t) => {
            let j = gerbang::jawaban_untuk(t);
            return tolak(
                op,
                klien,
                &rantai,
                mode,
                received_at,
                Tolakan::Gerbang(j),
                Some(&metadata),
            )
            .await;
        }
    };

    // Replay sebelum cek kuota: kunci + sidik sama tidak menyentuh Redis
    // (§4.1, AC-11).
    let kanonik = serde_json::json!({"input": input, "metadata": metadata});
    let idempoten = kunci.map(|k| {
        let sidik = intake::sidik_idempoten(op.kunci_idempoten.isi(), endpoint, &kanonik);
        (k.as_str().to_owned(), sidik)
    });
    if let Some((k, sidik)) = &idempoten {
        match op.permintaan.cari_idempoten(klien, k.clone()).await {
            Err(e) => return galat(e),
            Ok(Some(ada)) => {
                return jawab_idempoten(op, klien, &rantai, received_at, &ada, sidik, &metadata)
                    .await;
            }
            Ok(None) => {}
        }
    }

    if let Some(jenis) = rantai.nonaktif {
        return tolak(
            op,
            klien,
            &rantai,
            mode,
            received_at,
            Tolakan::Nonaktif(jenis),
            Some(&metadata),
        )
        .await;
    }

    match ambil_kuota_intake(op, klien, &rantai, false, received_at).await {
        Ok(_) => {}
        Err(None) => return layanan_503(),
        Err(Some(retry_after)) => {
            let t = Tolakan::Gerbang(Jawaban::KuotaHabis { retry_after });
            return tolak(op, klien, &rantai, mode, received_at, t, Some(&metadata)).await;
        }
    }

    let request_id = Uuid::now_v7();
    let antrean = Antrean {
        request_id,
        dispatch_key: dispatch_key(),
        received_at,
        input,
        metadata: metadata.clone(),
        idempoten: idempoten.clone(),
        referensi_client: referensi,
    };
    match op
        .permintaan
        .simpan_antre(klien, rantai.clone(), antrean)
        .await
    {
        Err(e) => galat(e),
        Ok(HasilAntre::Baru) => {
            tracing::info!(%request_id, "downstream: request async masuk antrean");
            jawab(
                StatusCode::ACCEPTED,
                Json(EnqueueResponse {
                    queue_id: request_id,
                    status: ApiStatus::Queued,
                }),
            )
        }
        // Kalah balapan unique constraint: jawab seperti replay (AC-12).
        Ok(HasilAntre::Ada(ada)) => {
            let sidik = idempoten.map(|(_, s)| s).unwrap_or_default();
            jawab_idempoten(op, klien, &rantai, received_at, &ada, &sidik, &metadata).await
        }
    }
}

/// Kunci sama: sidik sama → 200 status terkini; sidik beda → 422
/// `idempotency_key_konflik` tanpa baris.
async fn jawab_idempoten(
    op: &OperasiDownstream,
    klien: SesiKlien,
    rantai: &Arc<RantaiEndpoint>,
    received_at: DateTime<Utc>,
    ada: &BarisIdempoten,
    sidik: &str,
    metadata: &Value,
) -> Response {
    if ada.sidik != sidik {
        let t = Tolakan::Gerbang(Jawaban::IdempotencyKeyKonflik);
        return tolak(
            op,
            klien,
            rantai,
            Mode::Async,
            received_at,
            t,
            Some(metadata),
        )
        .await;
    }
    match status_api_dari(ada.status, ada.attempt_count) {
        Some(status) => jawab(
            StatusCode::OK,
            Json(EnqueueResponse {
                queue_id: ada.request_id,
                status,
            }),
        ),
        None => layanan_503(),
    }
}

/// `dispatch_key` dari CSPRNG (K-05).
fn dispatch_key() -> Uuid {
    let mut acak = [0u8; 16];
    if aws_lc_rs::rand::fill(&mut acak).is_err() {
        // Tanpa CSPRNG tidak ada kunci acak yang layak; UUIDv7 masih memuat
        // 74 bit acak dari sumber yang sama dengan `request_id`.
        return Uuid::now_v7();
    }
    uuid::Builder::from_random_bytes(acak).into_uuid()
}

// ---------------------------------------------------------------------------
// getRequest / cancelRequest (ARC-65)
// ---------------------------------------------------------------------------

pub(crate) async fn get_request(
    State(state): State<Arc<DownstreamState>>,
    headers: HeaderMap,
    Path(mentah): Path<String>,
) -> Response {
    let klien = match masuk(&state, &headers).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    let id = match path_id(&mentah) {
        Ok(id) => id,
        Err(r) => return r,
    };
    // Polling tidak menyentuh Redis dan tidak memotong kuota (AC-16).
    match state.operasi.permintaan.status(klien, id).await {
        Err(e) => galat(e),
        Ok(None) => galat(GalatDownstream::RequestTidakDikenal),
        Ok(Some(b)) => match request_status(&b) {
            Some(s) => jawab(StatusCode::OK, Json(s)),
            None => galat(GalatDownstream::RequestTidakDikenal),
        },
    }
}

pub(crate) async fn cancel_request(
    State(state): State<Arc<DownstreamState>>,
    headers: HeaderMap,
    Path(mentah): Path<String>,
) -> Response {
    let klien = match masuk(&state, &headers).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    let id = match path_id(&mentah) {
        Ok(id) => id,
        Err(r) => return r,
    };
    match state.operasi.permintaan.batalkan(klien, id).await {
        Err(e) => galat(e),
        Ok(HasilBatal::TidakAda) => galat(GalatDownstream::RequestTidakDikenal),
        Ok(HasilBatal::Dibatalkan) => {
            tracing::info!(request_id = %id, "downstream: request dibatalkan client");
            jawab(
                StatusCode::OK,
                Json(RequestStatus {
                    queue_id: id,
                    status: ApiStatus::Cancelled,
                    hasil: None,
                    failure_reason: None,
                    cancel_reason: Some(CancelReason::DibatalkanClient),
                }),
            )
        }
        Ok(HasilBatal::TidakBisa(b)) => match status_api_dari(b.status, b.attempt_count) {
            Some(s) => nirs_cache(
                ApiError::new(StatusCode::CONFLICT, ProblemKodeDown::TidakBisaDibatalkan)
                    .status_api(s)
                    .into_response(),
            ),
            None => galat(GalatDownstream::RequestTidakDikenal),
        },
    }
}

// ---------------------------------------------------------------------------
// submitSync (ARC-66)
// ---------------------------------------------------------------------------

/// Respons gagal sync: 502/504 dengan `request_id`, tanpa `Retry-After`.
fn gagal_sync(request_id: Uuid, failure_reason: Option<&str>) -> Response {
    let (status, kode) = match failure_reason {
        Some("timeout_sync") => (StatusCode::GATEWAY_TIMEOUT, ProblemKodeDown::TimeoutSync),
        Some("retry_habis") => (StatusCode::BAD_GATEWAY, ProblemKodeDown::RetryHabis),
        Some("provider_error_permanen") => (
            StatusCode::BAD_GATEWAY,
            ProblemKodeDown::ProviderErrorPermanen,
        ),
        _ => return layanan_503(),
    };
    nirs_cache(
        ApiError::new(status, kode)
            .request_id(request_id)
            .into_response(),
    )
}

/// Hanya elemen yang dikirim client (U-43, ADR-0002 P-1).
fn saring_hasil(
    hasil: BTreeMap<String, HasilElemen>,
    input: &Value,
) -> BTreeMap<String, HasilElemen> {
    hasil
        .into_iter()
        .filter(|(k, _)| input.get(k).is_some())
        .collect()
}

fn sukses_sync(request_id: Uuid, hasil: BTreeMap<String, HasilElemen>) -> Response {
    jawab(
        StatusCode::OK,
        Json(SyncResult {
            request_id,
            status: ApiStatus::Done,
            hasil,
        }),
    )
}

/// Parameter token sync dari `parameter_dispatch` pool. `None` bila bentuknya
/// tidak sah: fail-closed, sama dengan rebuild.
fn permintaan_token(rute: &PoolRute, request_id: Uuid) -> Option<PermintaanToken> {
    let p = &rute.parameter_dispatch;
    let timeout = p
        .get("timeout_total_upstream_detik")
        .and_then(Value::as_i64)
        .unwrap_or(TIMEOUT_UPSTREAM_BAWAAN_DETIK);
    Some(PermintaanToken {
        request_id: request_id.to_string(),
        nomor_percobaan: 1,
        sync: true,
        maks_konkurensi: p
            .get("maks_konkurensi")
            .and_then(Value::as_i64)
            .unwrap_or(MAKS_KONKURENSI_BAWAAN),
        ttl_slot_mikro: (timeout + 60) * 1_000_000,
        rasio_retry: rute.retry_rasio,
        retry_min: rute.retry_min_per_jam,
        porsi_sync: rute.porsi_sync_maks,
        jendela_rate: jendela_rate(p).ok()?,
    })
}

/// Token sync dengan batas tunggu (`tunggu_token_sync`, default 200 ms, §5a.4).
/// Penolakan berbentuk rate ditunggu selama masih muat dalam batas.
async fn ambil_token_sync(
    op: &OperasiDownstream,
    ctx: &KonteksKuota,
    p: &PermintaanToken,
) -> Result<(), Option<Jawaban>> {
    let batas = tokio::time::Instant::now() + op.tunggu_token_sync;
    loop {
        let jawab = op
            .kuota
            .token(ctx.clone(), p.clone())
            .await
            .map_err(|_| None)?;
        let tunggu = match &jawab {
            HasilToken::Diberikan(_) => return Ok(()),
            // `Retry-After` kuota dihitung pemanggil dari jendela kuota.
            HasilToken::KuotaHabis(_) => return Err(Some(Jawaban::KuotaHabis { retry_after: 0 })),
            HasilToken::SlotGanda(_) => return Err(None),
            HasilToken::RateLimit(s)
            | HasilToken::PorsiSyncHabis(s)
            | HasilToken::KonkurensiPenuh(s)
            | HasilToken::RetryBudgetHabis(s)
            | HasilToken::AnggaranHabis(s) => Duration::from_micros(s.tunggu_mikro.max(1) as u64),
        };
        if tokio::time::Instant::now() + tunggu > batas {
            return Err(Some(Jawaban::PorsiSyncHabis {
                retry_after: jawab.retry_after_detik().unwrap_or(1),
            }));
        }
        tokio::time::sleep(tunggu).await;
    }
}

pub(crate) async fn submit_sync(
    State(state): State<Arc<DownstreamState>>,
    headers: HeaderMap,
    Path(mentah): Path<String>,
    body: Bytes,
) -> Response {
    let klien = match masuk(&state, &headers).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    let endpoint = match path_id(&mentah) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let received_at = Utc::now();
    let rantai = match muat(&state, klien, endpoint, received_at).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    let op = state.operasi.as_ref();
    // Tanpa klien provider (ARC-64) tidak ada panggilan yang bisa dilakukan:
    // 503 sebelum kuota disentuh dan sebelum baris apa pun ditulis.
    let Some(pemanggil) = op.pemanggil.clone() else {
        return layanan_503();
    };
    let mode = Mode::Sync;

    let referensi = match referensi_client(&headers) {
        Ok(r) => r,
        Err(j) => {
            return tolak(
                op,
                klien,
                &rantai,
                mode,
                received_at,
                Tolakan::Gerbang(j),
                None,
            )
            .await;
        }
    };
    let (input, metadata) = match validasi_body(&rantai, &body) {
        Ok(v) => v,
        Err((j, meta)) => {
            return tolak(
                op,
                klien,
                &rantai,
                mode,
                received_at,
                Tolakan::Gerbang(j),
                meta.as_ref(),
            )
            .await;
        }
    };
    // Sync tidak mendukung Idempotency-Key: 400 tanpa baris, tetap memakai satu
    // token limiter penolakan (AC-25, U-45h).
    if headers.contains_key("idempotency-key") {
        let t = Tolakan::Gerbang(Jawaban::IdempotencyKeyTidakDidukung);
        return tolak(op, klien, &rantai, mode, received_at, t, Some(&metadata)).await;
    }
    if let Some(jenis) = rantai.nonaktif {
        return tolak(
            op,
            klien,
            &rantai,
            mode,
            received_at,
            Tolakan::Nonaktif(jenis),
            Some(&metadata),
        )
        .await;
    }
    let rute = match ambil_kuota_intake(op, klien, &rantai, true, received_at).await {
        Ok(r) => r.clone(),
        Err(None) => return layanan_503(),
        Err(Some(retry_after)) => {
            let t = Tolakan::Gerbang(Jawaban::KuotaHabis { retry_after });
            return tolak(op, klien, &rantai, mode, received_at, t, Some(&metadata)).await;
        }
    };

    let request_id = Uuid::now_v7();
    let ctx = konteks(&rute, received_at);
    let Some(p) = permintaan_token(&rute, request_id) else {
        tracing::error!(pool = %rute.upstream_pool_id, "downstream: parameter_dispatch tidak sah");
        return layanan_503();
    };
    match ambil_token_sync(op, &ctx, &p).await {
        Ok(()) => {}
        Err(None) => return layanan_503(),
        Err(Some(Jawaban::KuotaHabis { .. })) => {
            let Some(retry_after) = retry_after_kuota(op, &rute, received_at).await else {
                return layanan_503();
            };
            let t = Tolakan::Gerbang(Jawaban::KuotaHabis { retry_after });
            return tolak(op, klien, &rantai, mode, received_at, t, Some(&metadata)).await;
        }
        Err(Some(j)) => {
            return tolak(
                op,
                klien,
                &rantai,
                mode,
                received_at,
                Tolakan::Gerbang(j),
                Some(&metadata),
            )
            .await;
        }
    }
    let pool_id = rute.upstream_pool_id.to_string();

    // Transaksi pertama: request `in_flight`, history, log `status_akses` NULL.
    // Periode dan ember dihitung dari `requested_at` yang sama dengan baris log
    // (§5a.1 butir 1d).
    let kode_transaksi = Uuid::now_v7();
    let requested_at = Utc::now();
    let ctx = konteks(&rute, requested_at);
    let mulai = MulaiSync {
        request_id,
        kode_transaksi,
        upstream_pool_id: rute.upstream_pool_id,
        received_at,
        requested_at,
        input: input.clone(),
        metadata,
        nik: nik_dari(&input),
        referensi_client: referensi,
    };
    if let Err(e) = op.permintaan.mulai_sync(klien, rantai.clone(), mulai).await {
        // Tidak ada yang ditulis: slot dikembalikan.
        let _ = op.kuota.lepas(pool_id, request_id.to_string(), 1).await;
        return galat(e);
    }
    // INCR setelah log ter-commit, sebelum panggilan. Gagal atau slot
    // kedaluwarsa: tidak memanggil upstream, log dibiarkan untuk reconciler
    // (§5a.4).
    match op.kuota.tambah(ctx, request_id.to_string(), 1).await {
        Ok(t) if t.boleh_memanggil_upstream() => {}
        _ => return layanan_503(),
    }

    // Panggilan dan transaksi kedua berjalan di task sendiri: client yang
    // memutus koneksi tidak membatalkan panggilan upstream (AC-29).
    let permintaan = op.permintaan.clone();
    let kuota = op.kuota.clone();
    let panggilan = PanggilanProvider {
        request_id,
        kode_transaksi,
        provider_id: rantai.provider_id,
        provider_config_version: rantai.provider_config_version,
        endpoint_config_version: rantai.endpoint_config_version,
        input: input.clone(),
    };
    let tugas = tokio::spawn(async move {
        let mulai = std::time::Instant::now();
        let hasil = pemanggil.panggil(panggilan).await;
        let durasi = i32::try_from(mulai.elapsed().as_millis()).unwrap_or(i32::MAX);
        let selesai = SelesaiSync {
            request_id,
            kode_transaksi,
            responded_at: Utc::now(),
            duration_ms: durasi,
            hasil: hasil.clone(),
        };
        let tulis = permintaan.selesai_sync(klien, selesai).await;
        // Slot dilepas setelah log dilengkapi (§5a.1 butir 5).
        let _ = kuota.lepas(pool_id, request_id.to_string(), 1).await;
        (hasil, tulis)
    });
    let Ok((hasil, tulis)) = tugas.await else {
        return layanan_503();
    };
    match tulis {
        Ok(TulisSync::Ditulis) => match hasil {
            HasilPanggilan::Berhasil { hasil, .. } => {
                sukses_sync(request_id, saring_hasil(hasil, &input))
            }
            gagal @ HasilPanggilan::Gagal { .. } => {
                gagal_sync(request_id, akhir_sync(&gagal).failure_reason)
            }
        },
        // Reconciler menang: respons mengikuti PostgreSQL (T-06, AC-28).
        Ok(TulisSync::Kalah(Some(b))) => match b.status {
            StatusInternal::Done => sukses_sync(
                request_id,
                saring_hasil(hasil_dari(b.output.as_ref()).unwrap_or_default(), &input),
            ),
            StatusInternal::Failed => gagal_sync(request_id, b.failure_reason.as_deref()),
            _ => layanan_503(),
        },
        Ok(TulisSync::Kalah(None)) | Err(_) => layanan_503(),
    }
}

/// `timeout_total_upstream_detik` bila `parameter_dispatch` tidak menyebutnya,
/// sama dengan `sumber_pg`.
const TIMEOUT_UPSTREAM_BAWAAN_DETIK: i64 = 30;
/// `maks_konkurensi` bila `parameter_dispatch` tidak menyebutnya.
const MAKS_KONKURENSI_BAWAAN: i64 = 10;

/// Only the router assembly uses this; kept next to the handlers it wires.
pub fn router(state: Arc<DownstreamState>) -> axum::Router<()> {
    use axum::routing::{get, post};

    axum::Router::new()
        .route(
            "/v1/endpoints/{endpoint_id}/kontrak",
            get(get_endpoint_kontrak),
        )
        .route("/v1/endpoints/{endpoint_id}/sync", post(submit_sync))
        .route("/v1/endpoints/{endpoint_id}/async", post(enqueue_async))
        .route("/v1/requests/{queue_id}", get(get_request))
        .route("/v1/requests/{queue_id}/cancel", post(cancel_request))
        .with_state(state)
}
