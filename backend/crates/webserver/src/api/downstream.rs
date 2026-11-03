//! Downstream API (consumed by BGN applications outside this repo). After
//! SHAA-175 closes, breaking changes need a new API version.
//!
//! Handlers are 501 stubs until SHAA-19x implements them; annotations are the
//! reviewed contract and must not change when stub bodies are replaced.

use std::collections::BTreeMap;

use axum::{Json, http::StatusCode};
use serde::{Deserialize, Serialize};
use utoipa::{OpenApi, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::enums::{ApiStatus, CancelReason, FailureReason, StatusHasil};
use super::problem::Problem;

// ---------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------

/// `application/x-www-form-urlencoded` body (R-05). Credentials may arrive via
/// HTTP Basic instead (U-3); neither value is ever echoed back.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenRequest {
    /// Fixed value `client_credentials` (R-05).
    pub grant_type: String,
    /// Alternative to HTTP Basic; secret must not appear in URLs.
    #[serde(default)]
    #[schema(example = "bgn-mpm-prod")]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
}

/// Bearer token (R-05). No `refresh_token`; the client repeats
/// `issueToken` after expiry. Response carries `Cache-Control: no-store`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TokenResponse {
    /// Literal `Bearer` prefix token, 900 s lifetime (R-05).
    pub access_token: String,
    /// Fixed `Bearer`.
    pub token_type: String,
    /// Lifetime in seconds; default 900 (R-05, configurable).
    #[schema(example = 900, minimum = 60, maximum = 3600)]
    pub expires_in: i64,
}

/// One input field of the endpoint contract (US-15).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FieldInput {
    pub nama: String,
    /// Logical type of the field, e.g. `string` or `number`.
    pub tipe: String,
    pub wajib: bool,
}

/// One metadata field of the endpoint contract (US-15).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FieldMetadata {
    pub nama_field: String,
    pub tipe: String,
    pub wajib: bool,
    /// Closed value list when the field is restricted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nilai_diizinkan: Option<Vec<String>>,
    pub deskripsi: String,
    /// Contract version of this metadata field.
    #[schema(example = 1)]
    pub versi: i64,
}

/// Response of `getEndpointKontrak` (US-15; U-4 path shape).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EndpointKontrak {
    pub endpoint_id: Uuid,
    /// Path modes this endpoint serves (U-4: the path decides the mode).
    pub mode_diizinkan: Vec<Mode>,
    pub field_input: Vec<FieldInput>,
    pub field_metadata: Vec<FieldMetadata>,
    #[schema(example = 1)]
    pub endpoint_config_version: i64,
}

/// Path mode of a downstream endpoint (U-4). Not writable in request bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    Async,
}

/// Envelope for both submit paths; `additionalProperties: false` at the
/// envelope level. Unknown `metadata` keys are rejected 422 (US-09).
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SubmitRequest {
    /// Provider input fields; unknown fields (including `TRESHOLD`,
    /// `USER_ID`, `PASSWORD`, `IP_USER`, `transactionId`) are rejected 422
    /// without echoing values (R-16).
    #[schema(value_type = Object)]
    pub input: serde_json::Value,
    /// Metadata per endpoint contract; unknown keys rejected 422.
    #[schema(value_type = Object)]
    pub metadata: serde_json::Value,
}

/// Result for one checked element (contract section 3.2.2, O-01/O-02).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HasilElemen {
    pub status: StatusHasil,
    /// 0..=100 or `null` when the provider gives no score.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0, maximum = 100, nullable = true)]
    pub skor: Option<i64>,
}

/// Synchronous response: the provider answer is already complete (ADR-0001
/// section 4.1). `Cache-Control: no-store`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SyncResult {
    pub request_id: Uuid,
    /// Always `done` on 200.
    pub status: ApiStatus,
    /// Only the elements the client sent, keyed by element name (U-6).
    pub hasil: BTreeMap<String, HasilElemen>,
}

/// 202: new async request. 200 with the same shape: replay of an identical
/// `Idempotency-Key` fingerprint, no new request (U-5).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EnqueueResponse {
    pub queue_id: Uuid,
    /// `queued` on 202; the current status on 200 replay.
    pub status: ApiStatus,
}

/// Polling response (S-01..S-05) and webhook body (section 2.1). Status never
/// goes backwards; `hasil` only when `done`, `failure_reason` only when
/// `failed`, `cancel_reason` only when `cancelled`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RequestStatus {
    pub queue_id: Uuid,
    pub status: ApiStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hasil: Option<BTreeMap<String, HasilElemen>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<FailureReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<CancelReason>,
}

// ---------------------------------------------------------------------------
// Handlers (501 stubs)
// ---------------------------------------------------------------------------

fn stub() -> (StatusCode, Json<Problem>) {
    (StatusCode::NOT_IMPLEMENTED, Json(Problem::stub()))
}

/// Issue a downstream access token (R-05, US-14). Credentials via HTTP Basic
/// or body (U-3); 4xx bodies are problem+json with the RFC 6749 `error` field
/// (U-3), except 429.
#[utoipa::path(
    post,
    path = "/v1/oauth/token",
    operation_id = "issueToken",
    tag = "downstream",
    request_body(content = TokenRequest, content_type = "application/x-www-form-urlencoded", description = "grant_type=client_credentials; kredensial juga boleh lewat HTTP Basic (U-3)"),
    responses(
        (status = 200, description = "Token baru (R-05); tanpa refresh_token", body = TokenResponse, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("token" = (summary = "Token bearer 900 detik", value = json!({"access_token": "bearer eyJhbGciOiJFUzI1NiJ9.example.example", "token_type": "Bearer", "expires_in": 900}))))
        ),
        (status = 400, description = "JSON tidak valid, grant tidak didukung, atau kredensial tidak lengkap", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("validasi_gagal" = (summary = "grant tidak didukung (U-3)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#validasi_gagal", "title": "Validasi gagal", "status": 400, "kode": "validasi_gagal", "error": "unsupported_grant_type"}))))
        ),
        (status = 401, description = "Kredensial salah; penyebab tidak dibedakan (U-1)", body = Problem, content_type = "application/problem+json", headers(
            ("WWW-Authenticate" = String, description = "Basic untuk endpoint token"),
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("tidak_terautentikasi" = (summary = "client_id/client_secret salah", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#tidak_terautentikasi", "title": "Tidak terautentikasi", "status": 401, "kode": "tidak_terautentikasi", "error": "invalid_client"}))))
        ),
        (status = 429, description = "Limiter GCRA per client_id (60/menit, burst 10) atau per IP (120/menit) habis (R-05)", body = Problem, content_type = "application/problem+json", headers(
            ("Retry-After" = String, description = "Detik hingga token berikutnya (GCRA)"),
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("token_dibatasi" = (summary = "Limiter token habis; tanpa field error (U-3)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#token_dibatasi", "title": "Token dibatasi", "status": 429, "kode": "token_dibatasi"}))))
        )
    ),
    security(())
)]
async fn issue_token(axum::Json(_body): axum::Json<TokenRequest>) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Contract of one endpoint: inputs, metadata, allowed mode (US-15).
#[utoipa::path(
    get,
    path = "/v1/endpoints/{endpoint_id}/kontrak",
    operation_id = "getEndpointKontrak",
    tag = "downstream",
    params(("endpoint_id" = Uuid, Path, description = "Endpoint yang ditugaskan kepada client pemanggil")),
    responses(
        (status = 200, description = "Kontrak endpoint aktif", body = EndpointKontrak, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Down),
        (status = 404, description = "Endpoint tidak dikenal atau bukan milik client (tidak dibedakan)", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("tidak_ditemukan" = (summary = "Bukan milik client atau tidak ada", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#tidak_ditemukan", "title": "Tidak ditemukan", "status": 404, "kode": "tidak_ditemukan"}))))
        ),
        (status = 503, response = Problem503Layanan)
    ),
    security(("oauth2" = []), ("apiKey" = []))
)]
async fn get_endpoint_kontrak(
    axum::extract::Path(_endpoint_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Synchronous submit (US-03, 07, 08, 09). `Idempotency-Key` must NOT be
/// present (400 `idempotency_key_tidak_didukung` when it is).
#[utoipa::path(
    post,
    path = "/v1/endpoints/{endpoint_id}/sync",
    operation_id = "submitSync",
    tag = "downstream",
    params(("endpoint_id" = Uuid, Path, description = "Endpoint sinkron (U-4: mode ditentukan path)")),
    request_body(content = SubmitRequest, content_type = "application/json", description = "Amplop input + metadata; additionalProperties false"),
    responses(
        (status = 200, description = "Jawaban provider lengkap (status done)", body = SyncResult, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("done" = (summary = "Semua elemen sesuai", value = json!({"request_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "done", "hasil": {"status_penerima_manfaat": {"status": "sesuai", "skor": null}}}))))
        ),
        (status = 400, description = "JSON tidak valid/kunci ganda (validasi_gagal) atau Idempotency-Key pada sync (idempotency_key_tidak_didukung)", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("idempotency_key_tidak_didukung" = (summary = "Header Idempotency-Key dikirim ke path sync", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#idempotency_key_tidak_didukung", "title": "Idempotency key tidak didukung", "status": 400, "kode": "idempotency_key_tidak_didukung"}))), ("validasi_gagal" = (summary = "Body bukan JSON yang sah", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#validasi_gagal", "title": "Validasi gagal", "status": 400, "kode": "validasi_gagal", "detail": "Body bukan JSON yang sah."}))))
        ),
        (status = 401, response = Problem401Down),
        (status = 403, description = "Endpoint nonaktif (U-7: penugasan menentukan akses)", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("objek_nonaktif" = (summary = "Endpoint dinonaktifkan", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#objek_nonaktif", "title": "Objek nonaktif", "status": 403, "kode": "objek_nonaktif", "jenis_objek": "endpoint"}))))
        ),
        (status = 404, response = Problem404Down),
        (status = 422, description = "Field input/metadata tidak sesuai kontrak; nilai tidak di-echo (R-16)", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("validasi_gagal" = (summary = "Field input tak dikenal (termasuk TRESHOLD, USER_ID, PASSWORD, IP_USER, transactionId)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#validasi_gagal", "title": "Validasi gagal", "status": 422, "kode": "validasi_gagal", "errors": [{"field": "input", "kode": "field_tak_dikenal"}]}))), ("metadata_tidak_valid" = (summary = "Kunci metadata tidak ada di kontrak endpoint", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#metadata_tidak_valid", "title": "Metadata tidak valid", "status": 422, "kode": "metadata_tidak_valid", "errors": [{"field": "metadata", "kode": "field_tak_dikenal"}]}))))
        ),
        (status = 429, description = "Kuota periode habis, porsi sync/jam berjalan penuh, atau limiter penolakan aktif; semua dengan Retry-After (U-8)", body = Problem, content_type = "application/problem+json", headers(
            ("Retry-After" = String, description = "Detik hingga percobaan berikutnya (GCRA)"),
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("kuota_habis" = (summary = "Kuota periode upstream pool habis", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#kuota_habis", "title": "Kuota habis", "status": 429, "kode": "kuota_habis"}))), ("porsi_sync_habis" = (summary = "Porsi sync atau rate limit jam berjalan penuh (satu kode, U-8)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#porsi_sync_habis", "title": "Porsi sync habis", "status": 429, "kode": "porsi_sync_habis"}))), ("penolakan_berlebih" = (summary = "Limiter penolakan client aktif; hanya empat field (R-10)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#penolakan_berlebih", "title": "Penolakan berlebih", "status": 429, "kode": "penolakan_berlebih"}))))
        ),
        (status = 502, description = "retry_habis atau provider_error_permanen, dengan request_id, tanpa Retry-After", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("retry_habis" = (summary = "Jendela retry habis (attempt efektif 1 di F0, contract v0.7.10)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#retry_habis", "title": "Retry habis", "status": 502, "kode": "retry_habis", "request_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f"}))), ("provider_error_permanen" = (summary = "Kelas kegagalan tidak diulang (auth/kuota provider/kode tak dipetakan/respons invalid/TLS)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#provider_error_permanen", "title": "Kesalahan provider permanen", "status": 502, "kode": "provider_error_permanen", "request_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f"}))))
        ),
        (status = 503, description = "Pool/provider nonaktif setelah enqueue (objek_nonaktif, tanpa Retry-After) atau Redis fail-closed (layanan_tidak_tersedia, Retry-After: 5)", body = Problem, content_type = "application/problem+json", headers(
            ("Retry-After" = String, description = "Hanya ada pada layanan_tidak_tersedia (5 detik); tidak ada pada objek_nonaktif (R-10)"),
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("objek_nonaktif" = (summary = "Upstream pool atau provider dinonaktifkan", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#objek_nonaktif", "title": "Objek nonaktif", "status": 503, "kode": "objek_nonaktif", "jenis_objek": "provider"}))), ("layanan_tidak_tersedia" = (summary = "Redis tidak siap (fail-closed)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#layanan_tidak_tersedia", "title": "Layanan tidak tersedia", "status": 503, "kode": "layanan_tidak_tersedia"}))))
        ),
        (status = 504, description = "Panggilan sync tidak selesai dalam batas waktu", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("timeout_sync" = (summary = "Waktu tunggu provider habis", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#timeout_sync", "title": "Timeout sync", "status": 504, "kode": "timeout_sync", "request_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f"}))))
        )
    ),
    security(("oauth2" = []), ("apiKey" = []))
)]
async fn submit_sync(
    axum::extract::Path(_endpoint_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<SubmitRequest>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Asynchronous submit (US-07). Optional `Idempotency-Key` header (U-2):
/// identical key+fingerprint replays 200 with the current status; same key,
/// different fingerprint is 422 `idempotency_key_konflik`.
#[utoipa::path(
    post,
    path = "/v1/endpoints/{endpoint_id}/async",
    operation_id = "enqueueAsync",
    tag = "downstream",
    params(("endpoint_id" = Uuid, Path, description = "Endpoint asinkron (U-4: mode ditentukan path)"), ("Idempotency-Key" = Option<String>, Header, description = "Opsional; ASCII printable, maksimal 128 byte (U-2). Kunci sama + sidik sama → 200 replay; kunci sama + sidik beda → 422")),
    request_body(content = SubmitRequest, content_type = "application/json", description = "Amplop input + metadata; additionalProperties false"),
    responses(
        (status = 202, description = "Request baru masuk antrean (U-5)", body = EnqueueResponse, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("queued" = (summary = "Request baru", value = json!({"queue_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "queued"}))))
        ),
        (status = 200, description = "Replay kunci idempoten yang sudah ada; tidak ada request baru (U-5)", body = EnqueueResponse, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("replay" = (summary = "Kunci dan sidik sama", value = json!({"queue_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "done"}))))
        ),
        (status = 400, response = Problem400Down),
        (status = 401, response = Problem401Down),
        (status = 403, response = Problem403ObjekNonaktif),
        (status = 404, response = Problem404Down),
        (status = 422, description = "Field tak dikenal, metadata tidak valid, atau kunci idempoten dengan sidik berbeda", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("validasi_gagal" = (summary = "Field input tak dikenal", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#validasi_gagal", "title": "Validasi gagal", "status": 422, "kode": "validasi_gagal", "errors": [{"field": "input", "kode": "field_tak_dikenal"}]}))), ("metadata_tidak_valid" = (summary = "Kunci metadata tidak ada di kontrak endpoint", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#metadata_tidak_valid", "title": "Metadata tidak valid", "status": 422, "kode": "metadata_tidak_valid", "errors": [{"field": "metadata", "kode": "field_tak_dikenal"}]}))), ("idempotency_key_konflik" = (summary = "Kunci sama, isi berbeda", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#idempotency_key_konflik", "title": "Konflik idempotency key", "status": 422, "kode": "idempotency_key_konflik"}))))
        ),
        (status = 429, description = "Kuota periode habis atau limiter penolakan aktif; dengan Retry-After", body = Problem, content_type = "application/problem+json", headers(
            ("Retry-After" = String, description = "Detik hingga percobaan berikutnya (GCRA)"),
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("kuota_habis" = (summary = "Kuota periode upstream pool habis", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#kuota_habis", "title": "Kuota habis", "status": 429, "kode": "kuota_habis"}))), ("penolakan_berlebih" = (summary = "Limiter penolakan client aktif; hanya empat field (R-10)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#penolakan_berlebih", "title": "Penolakan berlebih", "status": 429, "kode": "penolakan_berlebih"}))))
        ),
        (status = 503, description = "Pool/provider nonaktif (objek_nonaktif, tanpa Retry-After) atau Redis fail-closed (layanan_tidak_tersedia, Retry-After: 5)", body = Problem, content_type = "application/problem+json", headers(
            ("Retry-After" = String, description = "Hanya ada pada layanan_tidak_tersedia (5 detik); tidak ada pada objek_nonaktif (R-10)"),
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("objek_nonaktif" = (summary = "Upstream pool atau provider dinonaktifkan", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#objek_nonaktif", "title": "Objek nonaktif", "status": 503, "kode": "objek_nonaktif", "jenis_objek": "provider"}))), ("layanan_tidak_tersedia" = (summary = "Redis tidak siap (fail-closed)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#layanan_tidak_tersedia", "title": "Layanan tidak tersedia", "status": 503, "kode": "layanan_tidak_tersedia"}))))
        )
    ),
    security(("oauth2" = []), ("apiKey" = []))
)]
async fn enqueue_async(
    axum::extract::Path(_endpoint_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<SubmitRequest>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Poll a request (US-17). Does not consume quota; `done` stays readable at
/// least for the downstream pool TTL since `completed_at` (S-05).
#[utoipa::path(
    get,
    path = "/v1/requests/{queue_id}",
    operation_id = "getRequest",
    tag = "downstream",
    params(("queue_id" = Uuid, Path, description = "queue_id dari enqueueAsync")),
    responses(
        (status = 200, description = "Status terkini; tidak pernah mundur", body = RequestStatus, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("done" = (summary = "Request selesai", value = json!({"queue_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "done", "hasil": {"status_penerima_manfaat": {"status": "tidak_sesuai", "skor": 40}}}))))
        ),
        (status = 401, response = Problem401Down),
        (status = 404, response = Problem404Down),
        (status = 503, response = Problem503Layanan)
    ),
    security(("oauth2" = []), ("apiKey" = []))
)]
async fn get_request(
    axum::extract::Path(_queue_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Cancel a queued request (US-19). Terminal requests answer 409
/// `tidak_bisa_dibatalkan` with the current status in the body.
#[utoipa::path(
    post,
    path = "/v1/requests/{queue_id}/cancel",
    operation_id = "cancelRequest",
    tag = "downstream",
    params(("queue_id" = Uuid, Path, description = "queue_id dari enqueueAsync")),
    responses(
        (status = 200, description = "Request dibatalkan", body = RequestStatus, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("cancelled" = (summary = "Dibatalkan sebelum diproses", value = json!({"queue_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "cancelled", "cancel_reason": "dibatalkan_client"}))))
        ),
        (status = 401, response = Problem401Down),
        (status = 404, response = Problem404Down),
        (status = 409, description = "Request sudah terminal; body memuat status API saat ini", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("tidak_bisa_dibatalkan" = (summary = "Status sudah done/failed/expired/cancelled", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#tidak_bisa_dibatalkan", "title": "Tidak bisa dibatalkan", "status": 409, "kode": "tidak_bisa_dibatalkan", "status_api": "done"}))))
        ),
        (status = 503, response = Problem503Layanan)
    ),
    security(("oauth2" = []), ("apiKey" = []))
)]
async fn cancel_request(
    axum::extract::Path(_queue_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Webhook registration handler: kept for the spec only; never routed.
/// Body is the same shape as `RequestStatus` (same mapping function,
/// ADR-0001 section 3a.6).
#[utoipa::path(
    post,
    path = "requestTerminal",
    operation_id = "requestTerminal",
    tag = "downstream",
    params(
        ("webhook-id" = String, Header, description = "request_id (= queue_id); tetap sama saat retry di F1"),
        ("webhook-timestamp" = String, Header, description = "Detik Unix; client menolak selisih > 5 menit"),
        ("webhook-signature" = String, Header, description = "v1,<base64(HMAC-SHA256(kunci, \"{webhook-id}.{webhook-timestamp}.{body}\"))>; kunci = byte hasil decode secret whsec_ (Standard Webhooks)"),
        ("poolrouter-secret-versi" = i64, Header, description = "secret_versi yang dipakai; informatif, tidak ikut ditandatangani (contract W-08)")
    ),
    request_body(content = RequestStatus, content_type = "application/json", description = "Status terminal: done, failed, expired, atau cancelled. Tanpa payload dan tanpa NIK"),
    responses(
        (status = 200, description = "2xx apa pun dari client berarti terkirim; selain itu gagal dan tidak diulang di F0", headers(
            ("Cache-Control" = String, description = "Respons client; contoh no-store")
        )
        )
    ),
    security(())
)]
fn _request_terminal_webhook() {}

// ---------------------------------------------------------------------------
// Reusable problem responses (R-10: one example per kode)
// ---------------------------------------------------------------------------

/// 401 on authenticated paths: `WWW-Authenticate: Bearer`, no request row,
/// no quota consumed (U-1, R-16).
#[derive(utoipa::ToResponse)]
#[response(description = "Token atau API key tidak valid atau kedaluwarsa; tidak disimpan", content_type = "application/problem+json", headers(
    ("WWW-Authenticate" = String, description = "Bearer untuk endpoint resource"),
    ("Cache-Control" = String, description = "Selalu no-store (R-16)")
), examples(
    ("tidak_terautentikasi" = (summary = "Token kedaluwarsa atau salah", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#tidak_terautentikasi", "title": "Tidak terautentikasi", "status": 401, "kode": "tidak_terautentikasi"})))
))]
pub struct Problem401Down(pub Problem);

/// 400 on downstream paths (malformed JSON).
#[derive(utoipa::ToResponse)]
#[response(description = "Body bukan JSON yang sah", content_type = "application/problem+json", headers(
    ("Cache-Control" = String, description = "Selalu no-store (R-16)")
), examples(
    ("validasi_gagal" = (summary = "JSON tidak dapat diurai", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#validasi_gagal", "title": "Validasi gagal", "status": 400, "kode": "validasi_gagal"})))
))]
pub struct Problem400Down(pub Problem);

/// 404 on downstream paths: unknown or other client's resource, not
/// distinguished (U-1).
#[derive(utoipa::ToResponse)]
#[response(description = "Tidak ditemukan atau bukan milik client (tidak dibedakan)", content_type = "application/problem+json", headers(
    ("Cache-Control" = String, description = "Selalu no-store (R-16)")
), examples(
    ("tidak_ditemukan" = (summary = "queue_id/endpoint_id tidak dikenal", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#tidak_ditemukan", "title": "Tidak ditemukan", "status": 404, "kode": "tidak_ditemukan"})))
))]
pub struct Problem404Down(pub Problem);

/// 403 `objek_nonaktif` for the endpoint itself (before enqueue).
#[derive(utoipa::ToResponse)]
#[response(description = "Endpoint nonaktif", content_type = "application/problem+json", headers(
    ("Cache-Control" = String, description = "Selalu no-store (R-16)")
), examples(
    ("objek_nonaktif" = (summary = "Endpoint dinonaktifkan", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#objek_nonaktif", "title": "Objek nonaktif", "status": 403, "kode": "objek_nonaktif", "jenis_objek": "endpoint"})))
))]
pub struct Problem403ObjekNonaktif(pub Problem);

/// 503 `layanan_tidak_tersedia`: Redis fail-closed; always `Retry-After: 5`
/// (ADR-0001 section 5a.7).
#[derive(utoipa::ToResponse)]
#[response(description = "Redis fail-closed atau komponen tidak siap", content_type = "application/problem+json", headers(
    ("Retry-After" = String, description = "Selalu 5 detik"),
    ("Cache-Control" = String, description = "Selalu no-store (R-16)")
), examples(
    ("layanan_tidak_tersedia" = (summary = "Redis tidak siap", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#layanan_tidak_tersedia", "title": "Layanan tidak tersedia", "status": 503, "kode": "layanan_tidak_tersedia"})))
))]
pub struct Problem503Layanan(pub Problem);

// ---------------------------------------------------------------------------
// Document assembly
// ---------------------------------------------------------------------------

/// Adds the downstream security schemes (oauth2 client credentials + API key
/// header) to the document components.
pub struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{
            ApiKey, ApiKeyValue, ClientCredentials, Flow, OAuth2, Scopes, SecurityScheme,
        };
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "oauth2",
            SecurityScheme::OAuth2(OAuth2::new([Flow::ClientCredentials(
                ClientCredentials::new("/v1/oauth/token", Scopes::new()),
            )])),
        );
        components.add_security_scheme(
            "apiKey",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::with_description(
                "X-API-Key",
                "API key downstream (contract section 3.1b); token oauth2 adalah pilihan pertama",
            ))),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    info(title = "PoolRouter Downstream API", description = "Kontrak downstream PoolRouter F0 (SHAA-175). Error memakai RFC 9457 problem+json dengan kode tertutup; rincian tiap kode ada di docs/masalah.md. Semua respons no-store (R-16)."),
    paths(issue_token, get_endpoint_kontrak, submit_sync, enqueue_async, get_request, cancel_request),
    components(schemas(
        TokenRequest, TokenResponse, EndpointKontrak, FieldInput, FieldMetadata, Mode,
        SubmitRequest, HasilElemen, SyncResult, EnqueueResponse, RequestStatus,
        super::enums::ApiStatus, super::enums::FailureReason, super::enums::CancelReason,
        super::enums::StatusHasil, super::problem::ProblemKode, super::problem::Problem,
        super::problem::ValidasiError, super::problem::Posisi, super::enums::JenisObjek
    ), responses(Problem401Down, Problem400Down, Problem404Down, Problem403ObjekNonaktif, Problem503Layanan)),
    modifiers(&SecurityAddon),
    tags((name = "downstream", description = "Endpoint yang dipanggil aplikasi BGN (MPM/LKS/SIPHR)"))
)]
pub struct DownstreamDoc;

/// Webhook document fragment; its `paths` are moved into `webhooks:` at
/// generation time because the utoipa derive has no webhooks attribute.
#[derive(OpenApi)]
#[openapi(
    info(title = "PoolRouter Downstream API webhooks"),
    paths(_request_terminal_webhook),
    components(schemas(
        RequestStatus,
        super::enums::ApiStatus,
        super::enums::FailureReason,
        super::enums::CancelReason,
        super::enums::StatusHasil,
        super::problem::Problem
    ))
)]
pub struct WebhookDoc;

/// Assembled downstream document, `webhooks:` included.
pub fn doc() -> utoipa::openapi::OpenApi {
    let mut doc = DownstreamDoc::openapi();
    let webhook = WebhookDoc::openapi();
    doc.webhooks = Some(webhook.paths);
    doc
}

pub fn yaml() -> Result<String, Box<dyn std::error::Error>> {
    Ok(doc().to_yaml()?)
}

/// Router with the six operation stubs (utoipa-axum keeps annotations and
/// routes from the same functions).
pub fn router() -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(issue_token))
        .routes(routes!(get_endpoint_kontrak))
        .routes(routes!(submit_sync))
        .routes(routes!(enqueue_async))
        .routes(routes!(get_request))
        .routes(routes!(cancel_request))
}
