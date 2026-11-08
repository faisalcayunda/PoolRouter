//! Handler bodies for the downstream operations (ARC-187).
//!
//! The wire shape is fixed by `downstream-api.yaml`, generated from the
//! annotations in [`crate::api::downstream`]; this module only fills the bodies.
//! `getEndpointKontrak` is wired here; the other four are still stubs, each
//! dispatched by its own slice.
//!
//! Every answer passes through [`jawab`] or [`problem`], so `Cache-Control:
//! no-store` sits on every 2xx and every problem, 4xx and 5xx alike (AC-2).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use super::downstream::{EndpointKontrak, FieldMetadata};
use super::problem::{Problem, ProblemKodeDown};
use crate::downstream::downstream_sesi::{GalatDownstream, SesiKlien, SessionDownstream};
use crate::downstream::pengenal::{GalatPengenal, PengenalDownstream};

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

/// `Idempotency-Key` is optional; when present it must satisfy the
/// `request.idempotency_key` column: at most 128 bytes and printable ASCII only
/// (`^[\x21-\x7e]+$`), which also rules out spaces and control characters.
#[allow(clippy::result_large_err)]
fn idempotency_key(headers: &HeaderMap) -> Result<Option<String>, Response> {
    let Some(v) = headers.get("idempotency-key") else {
        return Ok(None);
    };
    let s = v.to_str().map_err(|_| {
        problem(
            ProblemKodeDown::ValidasiGagal,
            StatusCode::UNPROCESSABLE_ENTITY,
        )
    })?;
    if s.is_empty() || s.len() > 128 || !s.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(problem(
            ProblemKodeDown::ValidasiGagal,
            StatusCode::UNPROCESSABLE_ENTITY,
        ));
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
    let s = v.to_str().map_err(|_| {
        problem(
            ProblemKodeDown::ValidasiGagal,
            StatusCode::UNPROCESSABLE_ENTITY,
        )
    })?;
    if s.len() > 128 {
        return Err(problem(
            ProblemKodeDown::ValidasiGagal,
            StatusCode::UNPROCESSABLE_ENTITY,
        ));
    }
    Ok(Some(s.to_owned()))
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
// submitSync / enqueueAsync / getRequest / cancelRequest
// ---------------------------------------------------------------------------

/// Checks the chain status the stub owns: unknown endpoint is 404, an endpoint
/// (or its pool/provider) switched off is 403, and anything else is the 503 the
/// next slice replaces.
#[allow(clippy::result_large_err)]
async fn stub_status(
    state: &Arc<DownstreamState>,
    klien: SesiKlien,
    endpoint: Uuid,
) -> Result<(), Response> {
    match state.sesi.status_rantai(klien, endpoint).await {
        Err(e) => Err(galat(e)),
        Ok(None) => Err(galat(GalatDownstream::EndpointTidakDikenal)),
        Ok(Some(false)) => Err(galat(GalatDownstream::ObjekNonaktif)),
        Ok(Some(true)) => Ok(()),
    }
}

/// Stub until the dispatch slice wires the provider round trip. Validates the
/// headers this slice owns and answers the documented 503 instead of inventing
/// a result.
#[allow(clippy::result_large_err)]
pub(crate) async fn submit_sync(
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
    // Sync rejects Idempotency-Key (AC-1: OpenAPI spec).
    if headers.contains_key("idempotency-key") {
        return problem(
            ProblemKodeDown::IdempotencyKeyTidakDidukung,
            StatusCode::BAD_REQUEST,
        );
    }
    if let Err(r) = stub_status(&state, klien, endpoint).await {
        return r;
    }
    let mut res = problem(
        ProblemKodeDown::LayananTidakTersedia,
        StatusCode::SERVICE_UNAVAILABLE,
    );
    res.headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
    res
}

#[allow(clippy::result_large_err)]
pub(crate) async fn enqueue_async(
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
    if let Err(r) = idempotency_key(&headers) {
        return r;
    }
    if let Err(r) = referensi_client(&headers) {
        return r;
    }
    if let Err(r) = stub_status(&state, klien, endpoint).await {
        return r;
    }
    problem(
        ProblemKodeDown::LayananTidakTersedia,
        StatusCode::SERVICE_UNAVAILABLE,
    )
}

#[allow(clippy::result_large_err)]
pub(crate) async fn get_request(
    State(state): State<Arc<DownstreamState>>,
    headers: HeaderMap,
    Path(mentah): Path<String>,
) -> Response {
    let _klien = match masuk(&state, &headers).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    let _request = match path_id(&mentah) {
        Ok(id) => id,
        Err(r) => return r,
    };
    // Stub: polling queries wired in the polling slice.
    problem(ProblemKodeDown::TidakDitemukan, StatusCode::NOT_FOUND)
}

#[allow(clippy::result_large_err)]
pub(crate) async fn cancel_request(
    State(state): State<Arc<DownstreamState>>,
    headers: HeaderMap,
    Path(mentah): Path<String>,
) -> Response {
    let _klien = match masuk(&state, &headers).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    let _request = match path_id(&mentah) {
        Ok(id) => id,
        Err(r) => return r,
    };
    // Stub: cancellation logic wired in the cancel slice.
    problem(ProblemKodeDown::TidakDitemukan, StatusCode::NOT_FOUND)
}

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
