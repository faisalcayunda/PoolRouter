//! Handler bodies for the five downstream operations (ARC-158, AC-1/2/3).
//!
//! The wire shape is fixed by `downstream-api.yaml`, generated from the
//! annotations in [`crate::api::downstream`]; this module only fills the bodies.
//!
//! Every answer passes through [`jawab`] or [`problem`], so `Cache-Control:
//! no-store` sits on every 2xx and every problem, 4xx and 5xx alike (AC-2).

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::downstream::{
    EndpointKontrak, EnqueueResponse, FieldMetadata, RequestStatus, SubmitRequest,
};
use super::enums::ApiStatus;
use super::problem::{Problem, ProblemKodeDown};
use crate::downstream::downstream_sesi::{
    AntrePermintaan, BarisRequest, GalatDownstream, SesiKlien, SessionDownstream,
};
use crate::downstream::pengenal::PengenalDownstream;

/// State of the downstream API router: the credential resolver and the request
/// store, nothing else. The admin pool never reaches this router (ADR-0001 7.2).
pub struct DownstreamState {
    pub pengenal: Arc<dyn PengenalDownstream>,
    pub sesi: Arc<dyn SessionDownstream>,
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
    let mut p = Problem::new(kode, status.as_u16());
    problem_dengan(&mut p)
}

fn problem_dengan(p: &mut Problem) -> Response {
    let status = StatusCode::from_u16(p.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = serde_json::to_vec(p).unwrap_or_else(|_| b"{}".to_vec());
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
        GalatDownstream::Layanan => problem(
            ProblemKodeDown::LayananTidakTersedia,
            StatusCode::SERVICE_UNAVAILABLE,
        ),
    }
}

/// Authenticates the caller. Bearer OAuth2 and API key are both accepted, at
/// every operation, by the same resolver (AC-3).
#[allow(clippy::result_large_err)]
async fn masuk(state: &DownstreamState, headers: &HeaderMap) -> Result<SesiKlien, Response> {
    match state.pengenal.kenali(headers).await {
        Ok(k) => Ok(k),
        Err(_) => {
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

fn tidak_valid() -> Response {
    problem(
        ProblemKodeDown::ValidasiGagal,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
}

fn metadata_tidak_valid() -> Response {
    problem(
        ProblemKodeDown::MetadataTidakValid,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
}

/// Reads and shape-checks the submit envelope. `input` and `metadata` must both
/// be JSON objects; anything else is 422 `validasi_gagal` (AC-1).
#[allow(clippy::result_large_err)]
fn baca_kirim(body: &[u8]) -> Result<SubmitRequest, Response> {
    let kirim: SubmitRequest = serde_json::from_slice(body).map_err(|_| tidak_valid())?;
    if !kirim.input.is_object() || !kirim.metadata.is_object() {
        return Err(tidak_valid());
    }
    Ok(kirim)
}

/// Checks the metadata keys against the endpoint contract before the request
/// reaches the store: an unknown key is 422 `metadata_tidak_valid` (AC-1).
#[allow(clippy::result_large_err)]
async fn validasi_metadata(
    state: &DownstreamState,
    klien: SesiKlien,
    endpoint: Uuid,
    kirim: &SubmitRequest,
) -> Result<(), Response> {
    let kontrak = state.sesi.kontrak(klien, endpoint).await.map_err(galat)?;
    let Some(objek) = kirim.metadata.as_object() else {
        return Err(metadata_tidak_valid());
    };
    let bidang = &kontrak.input;
    for (kunci, nilai) in objek {
        let Some(def) = bidang.iter().find(|b| b.nama == *kunci) else {
            return Err(metadata_tidak_valid());
        };
        // A closed list only accepts its own values; the rejected value is never
        // echoed back in the problem.
        if let Some(daftar) = def.nilai_diizinkan.as_ref().and_then(|v| v.as_array())
            && !daftar.iter().any(|d| d == nilai)
        {
            return Err(metadata_tidak_valid());
        }
        if def.wajib && nilai.is_null() {
            return Err(metadata_tidak_valid());
        }
    }
    for def in kontrak.input.iter().filter(|b| b.wajib) {
        if !objek.contains_key(&def.nama) {
            return Err(metadata_tidak_valid());
        }
    }
    Ok(())
}

/// `Idempotency-Key` is optional; when present it must satisfy the
/// `request.idempotency_key` column: at most 128 bytes and printable ASCII only
/// (`^[\x21-\x7e]+$`), which also rules out spaces and control characters.
#[allow(clippy::result_large_err)]
fn idempotency_key(headers: &HeaderMap) -> Result<Option<String>, Response> {
    let Some(v) = headers.get("idempotency-key") else {
        return Ok(None);
    };
    let s = v.to_str().map_err(|_| tidak_valid())?;
    if s.is_empty() || s.len() > 128 || !s.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(tidak_valid());
    }
    Ok(Some(s.to_owned()))
}

/// `Referensi-Client` is optional and lands in a varchar(128) column; a longer
/// reference is refused rather than truncated, because truncation would silently
/// merge two clients' references in the transaction log.
#[allow(clippy::result_large_err)]
fn referensi_client(headers: &HeaderMap) -> Result<Option<String>, Response> {
    let Some(v) = headers.get("referensi-client") else {
        return Ok(None);
    };
    let s = v.to_str().map_err(|_| tidak_valid())?;
    if s.len() > 128 {
        return Err(tidak_valid());
    }
    Ok(Some(s.to_owned()))
}

/// SHA-256 hex (64 chars, the column width) over the endpoint and the exact
/// body, so two calls with one key but different payloads are a conflict.
fn sidik(endpoint: Uuid, body: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(endpoint.as_bytes());
    h.update(b"\0");
    h.update(body);
    format!("{:x}", h.finalize())
}

fn ke_status(b: BarisRequest) -> RequestStatus {
    let status = b.status.unwrap_or(ApiStatus::Queued);
    RequestStatus {
        queue_id: b.request_id,
        status,
        // `hasil` only travels with `done`; the contract says so.
        hasil: match status {
            ApiStatus::Done => b.hasil.and_then(|v| serde_json::from_value(v).ok()),
            _ => None,
        },
        failure_reason: b
            .failure_reason
            .and_then(|s| serde_json::from_value(serde_json::Value::String(s)).ok()),
        cancel_reason: b
            .cancel_reason
            .and_then(|s| serde_json::from_value(serde_json::Value::String(s)).ok()),
    }
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
// submitSync
// ---------------------------------------------------------------------------

/// Synchronous submit. The provider call itself belongs to the dispatch slice;
/// until it is wired this operation validates exactly like the async one and
/// then answers the documented 503 instead of inventing a result (AC-9 covers
/// the gate, not the provider round trip).
#[allow(clippy::result_large_err)]
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
    let kirim = match baca_kirim(&body) {
        Ok(k) => k,
        Err(r) => return r,
    };
    if let Err(r) = validasi_metadata(&state, klien, endpoint, &kirim).await {
        return r;
    }
    problem(
        ProblemKodeDown::LayananTidakTersedia,
        StatusCode::SERVICE_UNAVAILABLE,
    )
}

// ---------------------------------------------------------------------------
// enqueueAsync
// ---------------------------------------------------------------------------

#[allow(clippy::result_large_err)]
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
    let kirim = match baca_kirim(&body) {
        Ok(k) => k,
        Err(r) => return r,
    };
    if let Err(r) = validasi_metadata(&state, klien, endpoint, &kirim).await {
        return r;
    }
    let kunci = match idempotency_key(&headers) {
        Ok(k) => k,
        Err(r) => return r,
    };
    let referensi = match referensi_client(&headers) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let fingerprint = kunci.as_ref().map(|_| sidik(endpoint, &body));
    match state
        .sesi
        .antre(AntrePermintaan {
            klien,
            endpoint_id: endpoint,
            metadata: kirim.metadata,
            referensi_client: referensi,
            idempotency_key: kunci,
            idempotency_fingerprint: fingerprint,
        })
        .await
    {
        Err(e) => galat(e),
        // A replayed key whose row was rejected or purged is unknown to the
        // client just like any other missing request.
        Ok((baris, _)) if baris.status.is_none() => galat(GalatDownstream::RequestTidakDikenal),
        Ok((baris, dibuat)) => jawab(
            if dibuat {
                StatusCode::ACCEPTED
            } else {
                StatusCode::OK
            },
            Json(EnqueueResponse {
                queue_id: baris.request_id,
                status: baris.status.unwrap_or(ApiStatus::Queued),
            }),
        ),
    }
}

// ---------------------------------------------------------------------------
// getRequest / cancelRequest
// ---------------------------------------------------------------------------

#[allow(clippy::result_large_err)]
pub(crate) async fn get_request(
    State(state): State<Arc<DownstreamState>>,
    headers: HeaderMap,
    Path(mentah): Path<String>,
) -> Response {
    let klien = match masuk(&state, &headers).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    let request = match path_id(&mentah) {
        Ok(id) => id,
        Err(r) => return r,
    };
    match state.sesi.request(klien, request).await {
        Err(e) => galat(e),
        Ok(b) => jawab(StatusCode::OK, Json(ke_status(b))),
    }
}

#[allow(clippy::result_large_err)]
pub(crate) async fn cancel_request(
    State(state): State<Arc<DownstreamState>>,
    headers: HeaderMap,
    Path(mentah): Path<String>,
) -> Response {
    let klien = match masuk(&state, &headers).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    let request = match path_id(&mentah) {
        Ok(id) => id,
        Err(r) => return r,
    };
    match state.sesi.batalkan(klien, request).await {
        Err(e) => galat(e),
        Ok((baris, benar)) if benar => jawab(StatusCode::OK, Json(ke_status(baris))),
        Ok((baris, _)) => {
            // 409 carries the current status so the client stops retrying.
            let mut p = Problem::new(
                ProblemKodeDown::TidakBisaDibatalkan,
                StatusCode::CONFLICT.as_u16(),
            );
            p.status_api = baris.status;
            problem_dengan(&mut p)
        }
    }
}

/// Only the router assembly uses this; kept next to the handlers it wires.
pub fn router(state: Arc<DownstreamState>) -> axum::Router<()> {
    use axum::routing::get;

    axum::Router::new()
        .route(
            "/v1/endpoints/{endpoint_id}/kontrak",
            get(get_endpoint_kontrak),
        )
        .route(
            "/v1/endpoints/{endpoint_id}/sync",
            axum::routing::post(submit_sync),
        )
        .route(
            "/v1/endpoints/{endpoint_id}/async",
            axum::routing::post(enqueue_async),
        )
        .route("/v1/requests/{queue_id}", get(get_request))
        .route(
            "/v1/requests/{queue_id}/cancel",
            axum::routing::post(cancel_request),
        )
        .with_state(state)
}
