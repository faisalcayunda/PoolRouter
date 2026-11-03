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

/// Body `application/x-www-form-urlencoded`. Kredensial boleh dikirim lewat
/// HTTP Basic; tidak ada nilainya yang dikembalikan pada respons.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenRequest {
    /// Nilai tetap `client_credentials`.
    pub grant_type: String,
    /// Alternative to HTTP Basic; secret must not appear in URLs.
    #[serde(default)]
    #[schema(example = "bgn-mpm-prod")]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
}

/// Token Bearer. Tidak ada `refresh_token`; klien memanggil `issueToken` lagi
/// setelah kedaluwarsa. Respons membawa `Cache-Control: no-store`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TokenResponse {
    /// JWT mentah tanpa prefiks `Bearer`. Kirim sebagai
    /// `Authorization: <token_type> <access_token>`.
    pub access_token: String,
    /// Selalu `Bearer`; prefiks ini ada di sini, tidak pernah di `access_token`.
    pub token_type: TipeToken,
    /// Masa berlaku dalam detik; selalu 900.
    #[schema(example = 900)]
    pub expires_in: i64,
}

/// Nilai `token_type` tertutup; satu-satunya anggota adalah `Bearer`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, ToSchema)]
pub enum TipeToken {
    #[default]
    #[serde(rename = "Bearer")]
    Bearer,
}

/// Satu field input pada kontrak endpoint.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FieldInput {
    pub nama: String,
    /// Tipe logis field, misalnya `string` atau `number`.
    pub tipe: String,
    pub wajib: bool,
}

/// Satu field metadata pada kontrak endpoint.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FieldMetadata {
    pub nama_field: String,
    pub tipe: String,
    pub wajib: bool,
    /// Daftar nilai tertutup bila field dibatasi.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nilai_diizinkan: Option<Vec<String>>,
    pub deskripsi: String,
    /// Versi kontrak field metadata ini.
    #[schema(example = 1)]
    pub versi: i64,
}

/// Respons `getEndpointKontrak`. Path yang menentukan sinkron atau asinkron, jadi
/// tidak ada field mode yang dikirim.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EndpointKontrak {
    pub endpoint_id: Uuid,
    pub field_input: Vec<FieldInput>,
    pub field_metadata: Vec<FieldMetadata>,
    #[schema(example = 1)]
    pub endpoint_config_version: i64,
}

/// Envelope untuk kedua jalur pengiriman; `additionalProperties: false` di
/// tingkat envelope. Kunci `metadata` yang tidak dikenal ditolak 422.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SubmitRequest {
    /// Field input milik provider; field yang tidak dikenal (termasuk `TRESHOLD`,
    /// `USER_ID`, `PASSWORD`, `IP_USER`, `transactionId`) ditolak 422 tanpa
    /// mengembalikan nilainya.
    #[schema(value_type = Object)]
    pub input: serde_json::Value,
    /// Metadata sesuai kontrak endpoint; kunci yang tidak dikenal ditolak 422.
    #[schema(value_type = Object)]
    pub metadata: serde_json::Value,
}

/// Hasil untuk satu elemen yang diperiksa.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HasilElemen {
    pub status: StatusHasil,
    /// 0..=100, atau `null` bila provider tidak memberi skor.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0, maximum = 100, nullable = true)]
    pub skor: Option<i64>,
}

/// Respons sinkron: jawaban provider sudah lengkap. `Cache-Control: no-store`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SyncResult {
    pub request_id: Uuid,
    /// Selalu `done` pada 200.
    pub status: ApiStatus,
    /// Hanya elemen yang dikirim klien, dengan nama elemen sebagai kunci.
    pub hasil: BTreeMap<String, HasilElemen>,
}

/// 202: permintaan asinkron baru. 200 dengan bentuk yang sama: pengulangan
/// permintaan dengan `Idempotency-Key` dan sidik yang identik, tanpa permintaan
/// baru.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EnqueueResponse {
    pub queue_id: Uuid,
    /// `queued` pada 202; status terkini pada 200 hasil pengulangan.
    pub status: ApiStatus,
}

/// Respons polling dan body webhook. Status tidak pernah mundur; `hasil` hanya
/// saat `done`, `failure_reason` hanya saat `failed`, `cancel_reason` hanya saat
/// `cancelled`.
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

/// Menerbitkan access token downstream. Kredensial boleh lewat HTTP Basic atau
/// body; body 4xx adalah problem+json dengan field `error` menurut RFC 6749,
/// kecuali 429.
#[utoipa::path(
    post,
    path = "/v1/oauth/token",
    operation_id = "issueToken",
    tag = "downstream",
    request_body(content = TokenRequest, content_type = "application/x-www-form-urlencoded", description = "grant_type=client_credentials; kredensial juga boleh lewat HTTP Basic", example = json!({"grant_type": "client_credentials", "client_id": "bgn-mpm", "client_secret": "<secret>"})),
    responses(
        (status = 200, description = "Token baru; tanpa refresh_token", body = TokenResponse, headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("token" = (summary = "Token bearer 900 detik", value = json!({"access_token": "eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJjbGllbnQifQ.c2lnbmF0dXJl", "token_type": "Bearer", "expires_in": 900}))))
        ),
        (status = 400, description = "JSON tidak valid, grant tidak didukung, atau kredensial tidak lengkap", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("validasi_gagal" = (summary = "grant tidak didukung", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#validasi_gagal", "title": "Validasi gagal", "status": 400, "kode": "validasi_gagal", "error": "unsupported_grant_type"}))))
        ),
        (status = 401, description = "Kredensial salah; penyebab tidak dibedakan", body = Problem, content_type = "application/problem+json", headers(
            ("WWW-Authenticate" = String, description = "Basic untuk endpoint token"),
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("tidak_terautentikasi" = (summary = "client_id/client_secret salah", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#tidak_terautentikasi", "title": "Tidak terautentikasi", "status": 401, "kode": "tidak_terautentikasi", "error": "invalid_client"}))))
        ),
        (status = 429, description = "Limiter GCRA per client_id (60/menit, burst 10) atau per IP (120/menit) habis", body = Problem, content_type = "application/problem+json", headers(
            ("Retry-After" = i64, description = "Detik bulat (minimal 1) hingga token throttle berikutnya tersedia"),
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("token_dibatasi" = (summary = "Limiter token habis; tanpa field error", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#token_dibatasi", "title": "Token dibatasi", "status": 429, "kode": "token_dibatasi"}))))
        )
    ),
    security(())
)]
async fn issue_token(axum::Json(_body): axum::Json<TokenRequest>) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Kontrak satu endpoint: field input, field metadata, dan pola pemanggilan yang
/// diizinkan.
#[utoipa::path(
    get,
    path = "/v1/endpoints/{endpoint_id}/kontrak",
    operation_id = "getEndpointKontrak",
    tag = "downstream",
    params(("endpoint_id" = Uuid, Path, description = "Endpoint yang ditugaskan kepada client pemanggil")),
    responses(
        (status = 200, description = "Kontrak endpoint aktif", body = EndpointKontrak, headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        )
        ),
        (status = 401, response = Problem401Down),
        (status = 404, description = "Endpoint tidak dikenal atau bukan milik client (tidak dibedakan)", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
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

/// Pengiriman sinkron: hasil provider dikembalikan pada respons yang sama.
/// `Idempotency-Key` tidak boleh dikirim; bila ada, jawabannya 400
/// `idempotency_key_tidak_didukung`.
#[utoipa::path(
    post,
    path = "/v1/endpoints/{endpoint_id}/sync",
    operation_id = "submitSync",
    tag = "downstream",
    params(("endpoint_id" = Uuid, Path, description = "Endpoint sinkron; mode ditentukan oleh path")),
    request_body(content = SubmitRequest, content_type = "application/json", description = "Amplop input + metadata dengan additionalProperties false", example = json!({"input": {"NIK": "3201234567890001", "NAMA_LGKP": "Budi Santoso"}, "metadata": {"status_penerima_manfaat": "sesuai"}})),
    responses(
        (status = 200, description = "Jawaban provider lengkap (status done)", body = SyncResult, headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("done" = (summary = "Semua elemen sesuai", value = json!({"request_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "done", "hasil": {"NAMA_LGKP": {"status": "sesuai", "skor": 90}}}))))
        ),
        (status = 400, description = "JSON tidak valid/kunci ganda (validasi_gagal) atau Idempotency-Key pada sync (idempotency_key_tidak_didukung)", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("idempotency_key_tidak_didukung" = (summary = "Header Idempotency-Key dikirim ke path sync", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#idempotency_key_tidak_didukung", "title": "Idempotency key tidak didukung", "status": 400, "kode": "idempotency_key_tidak_didukung"}))), ("validasi_gagal" = (summary = "Body bukan JSON yang sah", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#validasi_gagal", "title": "Validasi gagal", "status": 400, "kode": "validasi_gagal", "detail": "Body bukan JSON yang sah."}))))
        ),
        (status = 401, response = Problem401Down),
        (status = 403, description = "Endpoint nonaktif (penugasan menentukan akses)", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("objek_nonaktif" = (summary = "Endpoint dinonaktifkan", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#objek_nonaktif", "title": "Objek nonaktif", "status": 403, "kode": "objek_nonaktif", "jenis_objek": "endpoint"}))))
        ),
        (status = 404, response = Problem404Down),
        (status = 422, description = "Field input/metadata tidak sesuai aturan; nilai yang ditolak tidak dikembalikan", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("validasi_gagal" = (summary = "Field input tak dikenal (termasuk TRESHOLD, USER_ID, PASSWORD, IP_USER, transactionId)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#validasi_gagal", "title": "Validasi gagal", "status": 422, "kode": "validasi_gagal", "errors": [{"field": "TRESHOLD", "kode": "field_tak_dikenal"}, {"field": "NAMA_LGKAP", "kode": "field_tak_dikenal"}], "jumlah_nama_tidak_cocok": 1}))), ("metadata_tidak_valid" = (summary = "Kunci metadata tidak ada di kontrak endpoint", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#metadata_tidak_valid", "title": "Metadata tidak valid", "status": 422, "kode": "metadata_tidak_valid", "errors": [{"field": "status_penerima_manfaat", "kode": "field_tak_dikenal"}]}))))
        ),
        (status = 429, description = "Kuota periode habis, porsi sync/jam berjalan penuh, atau limiter penolakan aktif; semua dengan Retry-After", body = Problem, content_type = "application/problem+json", headers(
            ("Retry-After" = i64, description = "Detik bulat (minimal 1) hingga percobaan berikutnya. `kuota_habis` mengikuti jendela kuota: sisa waktu sampai ember tertua keluar dari jendela (`bergulir_24j`) atau sampai `reset_kuota` berikutnya (`kalender`); `porsi_sync_habis` dan `penolakan_berlebih` mengikuti token limiter berikutnya"),
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("kuota_habis" = (summary = "Kuota periode upstream pool habis", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#kuota_habis", "title": "Kuota habis", "status": 429, "kode": "kuota_habis"}))), ("porsi_sync_habis" = (summary = "Porsi sync atau rate limit jam berjalan penuh (satu kode)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#porsi_sync_habis", "title": "Porsi sync habis", "status": 429, "kode": "porsi_sync_habis"}))), ("penolakan_berlebih" = (summary = "Limiter penolakan client aktif; body hanya empat field", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#penolakan_berlebih", "title": "Penolakan berlebih", "status": 429, "kode": "penolakan_berlebih"}))))
        ),
        (status = 502, description = "retry_habis atau provider_error_permanen, dengan request_id, tanpa Retry-After", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("retry_habis" = (summary = "Jendela retry habis (saat ini hanya ada satu percobaan efektif)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#retry_habis", "title": "Retry habis", "status": 502, "kode": "retry_habis", "request_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f"}))), ("provider_error_permanen" = (summary = "Kelas kegagalan tidak diulang (auth/kuota provider/kode tak dipetakan/respons invalid/TLS)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#provider_error_permanen", "title": "Kesalahan provider permanen", "status": 502, "kode": "provider_error_permanen", "request_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f"}))))
        ),
        (status = 503, description = "Pool/provider nonaktif (objek_nonaktif, tanpa Retry-After) atau Redis fail-closed (layanan_tidak_tersedia, Retry-After: 5)", body = Problem, content_type = "application/problem+json", headers(
            ("Retry-After" = i64, description = "Hanya ada pada layanan_tidak_tersedia, selalu 5 detik; tidak ada pada objek_nonaktif"),
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("objek_nonaktif" = (summary = "Upstream pool atau provider dinonaktifkan", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#objek_nonaktif", "title": "Objek nonaktif", "status": 503, "kode": "objek_nonaktif", "jenis_objek": "provider"}))), ("layanan_tidak_tersedia" = (summary = "Redis tidak siap (fail-closed)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#layanan_tidak_tersedia", "title": "Layanan tidak tersedia", "status": 503, "kode": "layanan_tidak_tersedia"}))))
        ),
        (status = 504, description = "Panggilan sync tidak selesai dalam batas waktu", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
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

/// Pengiriman asinkron: permintaan masuk antrean dan statusnya diperiksa lewat
/// polling atau webhook. Header `Idempotency-Key` opsional: kunci dan sidik yang
/// sama diulang menghasilkan 200 berisi status terkini; kunci sama dengan sidik
/// berbeda menghasilkan 422 `idempotency_key_konflik`.
#[utoipa::path(
    post,
    path = "/v1/endpoints/{endpoint_id}/async",
    operation_id = "enqueueAsync",
    tag = "downstream",
    params(("endpoint_id" = Uuid, Path, description = "Endpoint asinkron; path menentukan mode"), ("Idempotency-Key" = Option<String>, Header, description = "Opsional; 1–128 karakter ASCII printable (0x20–0x7E). Kunci sama + sidik sama → 200 replay; kunci sama + sidik beda → 422")),
    request_body(content = SubmitRequest, content_type = "application/json", description = "Amplop input + metadata dengan additionalProperties false", example = json!({"input": {"NIK": "3201234567890001", "NAMA_LGKP": "Budi Santoso"}, "metadata": {"status_penerima_manfaat": "sesuai"}})),
    responses(
        (status = 202, description = "Request baru masuk antrean", body = EnqueueResponse, headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("queued" = (summary = "Request baru", value = json!({"queue_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "queued"}))))
        ),
        (status = 200, description = "Replay kunci idempoten yang sudah ada; tidak ada request baru", body = EnqueueResponse, headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("replay" = (summary = "Kunci dan sidik sama", value = json!({"queue_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "done"}))))
        ),
        (status = 400, response = Problem400Down),
        (status = 401, response = Problem401Down),
        (status = 403, response = Problem403ObjekNonaktif),
        (status = 404, response = Problem404Down),
        (status = 422, description = "Field tak dikenal, metadata tidak valid, atau kunci idempoten dengan sidik berbeda", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("validasi_gagal" = (summary = "Field input tak dikenal", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#validasi_gagal", "title": "Validasi gagal", "status": 422, "kode": "validasi_gagal", "errors": [{"field": "TRESHOLD", "kode": "field_tak_dikenal"}, {"field": "NAMA_LGKAP", "kode": "field_tak_dikenal"}], "jumlah_nama_tidak_cocok": 1}))), ("metadata_tidak_valid" = (summary = "Kunci metadata tidak ada di kontrak endpoint", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#metadata_tidak_valid", "title": "Metadata tidak valid", "status": 422, "kode": "metadata_tidak_valid", "errors": [{"field": "status_penerima_manfaat", "kode": "field_tak_dikenal"}]}))), ("idempotency_key_konflik" = (summary = "Kunci sama, isi berbeda", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#idempotency_key_konflik", "title": "Konflik idempotency key", "status": 422, "kode": "idempotency_key_konflik"}))))
        ),
        (status = 429, description = "Kuota periode habis atau limiter penolakan aktif; dengan Retry-After", body = Problem, content_type = "application/problem+json", headers(
            ("Retry-After" = i64, description = "Detik bulat (minimal 1) hingga percobaan berikutnya. `kuota_habis` mengikuti jendela kuota: sisa waktu sampai ember tertua keluar dari jendela (`bergulir_24j`) atau sampai `reset_kuota` berikutnya (`kalender`); `porsi_sync_habis` dan `penolakan_berlebih` mengikuti token limiter berikutnya"),
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("kuota_habis" = (summary = "Kuota periode upstream pool habis", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#kuota_habis", "title": "Kuota habis", "status": 429, "kode": "kuota_habis"}))), ("penolakan_berlebih" = (summary = "Limiter penolakan client aktif; body hanya empat field", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#penolakan_berlebih", "title": "Penolakan berlebih", "status": 429, "kode": "penolakan_berlebih"}))))
        ),
        (status = 503, description = "Pool/provider nonaktif (objek_nonaktif, tanpa Retry-After) atau Redis fail-closed (layanan_tidak_tersedia, Retry-After: 5)", body = Problem, content_type = "application/problem+json", headers(
            ("Retry-After" = i64, description = "Hanya ada pada layanan_tidak_tersedia, selalu 5 detik; tidak ada pada objek_nonaktif"),
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
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

/// Memeriksa status sebuah permintaan. Tidak memotong kuota; hasil `done` tetap
/// bisa dibaca sedikitnya selama TTL pool downstream sejak `completed_at`.
#[utoipa::path(
    get,
    path = "/v1/requests/{queue_id}",
    operation_id = "getRequest",
    tag = "downstream",
    params(("queue_id" = Uuid, Path, description = "queue_id dari enqueueAsync")),
    responses(
        (status = 200, description = "Status terkini; tidak pernah mundur", body = RequestStatus, headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(
                ("queued" = (summary = "Masih menunggu di antrean", value = json!({"queue_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "queued"}))),
                ("processing" = (summary = "Sedang dikirim ke provider", value = json!({"queue_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "processing"}))),
                ("done" = (summary = "Request selesai; memuat hasil", value = json!({"queue_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "done", "hasil": {"NAMA_LGKP": {"status": "tidak_sesuai", "skor": 40}}}))),
                ("failed" = (summary = "Gagal setelah provider menjawab; memuat failure_reason", value = json!({"queue_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "failed", "failure_reason": "respons_tidak_valid"}))),
                ("cancelled" = (summary = "Dibatalkan client; memuat cancel_reason", value = json!({"queue_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "cancelled", "cancel_reason": "client"}))),
                ("expired" = (summary = "Kedaluwarsa sebelum diproses", value = json!({"queue_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "expired"})))
            )
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

/// Membatalkan permintaan yang masih `queued`. Permintaan berstatus `processing`
/// atau status akhir dijawab 409 `tidak_bisa_dibatalkan` dengan status terkini di
/// body.
#[utoipa::path(
    post,
    path = "/v1/requests/{queue_id}/cancel",
    operation_id = "cancelRequest",
    tag = "downstream",
    params(("queue_id" = Uuid, Path, description = "queue_id dari enqueueAsync")),
    responses(
        (status = 200, description = "Request dibatalkan", body = RequestStatus, headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("cancelled" = (summary = "Dibatalkan sebelum diproses", value = json!({"queue_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "cancelled", "cancel_reason": "dibatalkan_client"}))))
        ),
        (status = 401, response = Problem401Down),
        (status = 404, response = Problem404Down),
        (status = 409, description = "Request sedang `processing` atau sudah terminal (`done`/`failed`/`expired`/`cancelled`); body memuat status API saat ini", body = Problem, content_type = "application/problem+json", headers(
            ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
        ),
            examples(("tidak_bisa_dibatalkan" = (summary = "Status processing atau sudah done/failed/expired/cancelled", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#tidak_bisa_dibatalkan", "title": "Tidak bisa dibatalkan", "status": 409, "kode": "tidak_bisa_dibatalkan", "status_api": "done"}))))
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

/// Handler tujuan webhook yang didaftarkan klien.
/// Body memakai bentuk yang sama dengan `RequestStatus`, jadi `status`, `hasil`,
/// `failure_reason`, dan `cancel_reason` dibaca dengan cara yang sama.
#[utoipa::path(
    post,
    path = "requestTerminal",
    operation_id = "requestTerminal",
    tag = "downstream",
    params(
        ("webhook-id" = String, Header, description = "request_id (= queue_id); tetap sama saat retry di F1", example = json!("0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f")),
        ("webhook-timestamp" = String, Header, description = "Detik Unix; client menolak selisih > 5 menit", example = json!("1762387200")),
        ("webhook-signature" = String, Header, description = "v1,<base64(HMAC-SHA256(kunci, \"{webhook-id}.{webhook-timestamp}.{body}\"))>; kunci = byte hasil decode secret whsec_ (Standard Webhooks)", example = json!("v1,K5x1s0Y2b3Q4c5D6e7F8g9H0i1J2k3L4m5N6o7P8q9R=")),
        ("poolrouter-secret-versi" = i64, Header, description = "secret_versi yang dipakai; informatif, tidak ikut ditandatangani", example = json!(3))
    ),
    request_body(content = RequestStatus, content_type = "application/json", description = "Status terminal. Untuk `done` memuat `hasil` (bentuk sama dengan polling); tanpa data input dan tanpa NIK", examples(
        ("done" = (summary = "Request selesai; memuat hasil", value = json!({"queue_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "done", "hasil": {"NAMA_LGKP": {"status": "sesuai", "skor": 90}}}))),
        ("cancelled" = (summary = "Dibatalkan client; memuat cancel_reason", value = json!({"queue_id": "0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f", "status": "cancelled", "cancel_reason": "client"})))
    )),
    responses(
        (status = 200, description = "2xx apa pun dari client berarti terkirim; selain itu gagal dan tidak diulang", headers(
            ("Cache-Control" = String, description = "Respons client; contoh no-store")
        )
        )
    ),
    security(())
)]
fn _request_terminal_webhook() {}

// ---------------------------------------------------------------------------
// Reusable problem responses (satu contoh per kode)
// ---------------------------------------------------------------------------

/// 401 on authenticated paths: `WWW-Authenticate: Bearer`, no request row,
/// no quota consumed.
#[derive(utoipa::ToResponse)]
#[response(description = "Token atau API key tidak valid atau kedaluwarsa; tidak disimpan", content_type = "application/problem+json", headers(
    ("WWW-Authenticate" = String, description = "Bearer untuk endpoint resource"),
    ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
), examples(
    ("tidak_terautentikasi" = (summary = "Token kedaluwarsa atau salah", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#tidak_terautentikasi", "title": "Tidak terautentikasi", "status": 401, "kode": "tidak_terautentikasi"})))
))]
pub struct Problem401Down(pub Problem);

/// 400 on downstream paths (malformed JSON).
#[derive(utoipa::ToResponse)]
#[response(description = "Body bukan JSON yang sah", content_type = "application/problem+json", headers(
    ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
), examples(
    ("validasi_gagal" = (summary = "JSON tidak dapat diurai", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#validasi_gagal", "title": "Validasi gagal", "status": 400, "kode": "validasi_gagal"})))
))]
pub struct Problem400Down(pub Problem);

/// 404 on downstream paths: unknown or other client's resource, not
/// distinguished.
#[derive(utoipa::ToResponse)]
#[response(description = "Tidak ditemukan atau bukan milik client (tidak dibedakan)", content_type = "application/problem+json", headers(
    ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
), examples(
    ("tidak_ditemukan" = (summary = "queue_id/endpoint_id tidak dikenal", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#tidak_ditemukan", "title": "Tidak ditemukan", "status": 404, "kode": "tidak_ditemukan"})))
))]
pub struct Problem404Down(pub Problem);

/// 403 `objek_nonaktif` for the endpoint itself (before enqueue).
#[derive(utoipa::ToResponse)]
#[response(description = "Endpoint nonaktif", content_type = "application/problem+json", headers(
    ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
), examples(
    ("objek_nonaktif" = (summary = "Endpoint dinonaktifkan", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#objek_nonaktif", "title": "Objek nonaktif", "status": 403, "kode": "objek_nonaktif", "jenis_objek": "endpoint"})))
))]
pub struct Problem403ObjekNonaktif(pub Problem);

/// 503 `layanan_tidak_tersedia`: Redis fail-closed; always `Retry-After: 5`.
#[derive(utoipa::ToResponse)]
#[response(description = "Redis fail-closed atau komponen tidak siap", content_type = "application/problem+json", headers(
    ("Retry-After" = i64, description = "Selalu 5 detik"),
    ("Cache-Control" = String, description = "Selalu `no-store`; respons tidak boleh disimpan cache")
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
                "API key downstream; token oauth2 adalah pilihan pertama",
            ))),
        );
        rapi_retry_after(openapi);
        super::header::rapi_idempotency_key(openapi);
        super::header::rapi_objek_tertutup(openapi);
    }
}

/// `Retry-After` is integer seconds, rounded up, at least 1. The `headers(..)`
/// attribute can only set type and description, so the floor is applied here,
/// after both documents are assembled.
pub fn rapi_retry_after(openapi: &mut utoipa::openapi::OpenApi) {
    let skema = || {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::Integer)
            .format(Some(utoipa::openapi::schema::SchemaFormat::KnownFormat(
                utoipa::openapi::schema::KnownFormat::Int64,
            )))
            .minimum(Some(1))
            .build()
    };

    let terapkan = |headers: &mut utoipa::openapi::response::Response| {
        if let Some(utoipa::openapi::RefOr::T(header)) = headers.headers.get_mut("Retry-After") {
            header.schema = Some(skema().into());
        }
    };

    for item in openapi.paths.paths.values_mut() {
        for operation in [
            &mut item.get,
            &mut item.put,
            &mut item.post,
            &mut item.delete,
            &mut item.options,
            &mut item.head,
            &mut item.patch,
            &mut item.trace,
        ]
        .into_iter()
        .flatten()
        {
            for response in operation.responses.responses.values_mut() {
                if let utoipa::openapi::RefOr::T(response) = response {
                    terapkan(response);
                }
            }
        }
    }
    if let Some(components) = openapi.components.as_mut() {
        for response in components.responses.values_mut() {
            if let utoipa::openapi::RefOr::T(response) = response {
                terapkan(response);
            }
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    info(title = "PoolRouter Downstream API", description = "API publik PoolRouter untuk klien BGN. Error memakai RFC 9457 problem+json dengan daftar `kode` tertutup; rincian tiap kode, termasuk `batas`, ada di docs/masalah.md. Semua respons selalu `no-store`; respons tidak boleh disimpan cache."),
    paths(issue_token, get_endpoint_kontrak, submit_sync, enqueue_async, get_request, cancel_request),
    components(schemas(
        TokenRequest, TokenResponse, EndpointKontrak, FieldInput, FieldMetadata,
        SubmitRequest, HasilElemen, SyncResult, EnqueueResponse, RequestStatus,
        super::enums::ApiStatus, super::enums::FailureReason, super::enums::CancelReason,
        super::enums::StatusHasil, super::problem::Problem,
        super::problem::ProblemKodeDown, super::problem::ValidasiErrorDown,
        super::problem::JumlahNamaTidakCocok, super::enums::JenisObjek
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
