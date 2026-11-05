//! `GET /v1/endpoints/{endpoint_id}/kontrak` (contract `getEndpointKontrak`,
//! AC-9, GW3). The endpoint is answered from the client's assignment, so an
//! endpoint owned by another client and a missing one are the same 404.
//!
//! An assigned endpoint is served even while it is inactive (GW3): the client
//! learns about the deactivation on the submit call (`403 objek_nonaktif`), not
//! by the contract disappearing.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use super::auth::AuthClient;
use super::layers::ApiError;
use super::server::AppState;
use crate::api::downstream::EndpointKontrak;
use crate::api::problem::ProblemKodeDown;

pub async fn get_endpoint_kontrak(
    State(state): State<AppState>,
    AuthClient(client_id): AuthClient,
    Path(endpoint_id): Path<Uuid>,
) -> Result<Json<EndpointKontrak>, ApiError> {
    match state
        .konfig
        .muat(client_id, endpoint_id, (state.jam)())
        .await
    {
        Ok(entri) => match entri.kontrak {
            Some(kontrak) => Ok(Json(kontrak)),
            None => Err(ApiError::new(
                StatusCode::NOT_FOUND,
                ProblemKodeDown::TidakDitemukan,
            )),
        },
        // The store maps every driver failure to one opaque error; the client
        // gets the ordinary fail-closed answer with `Retry-After`.
        Err(_) => Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ProblemKodeDown::LayananTidakTersedia,
        )
        .retry_after(5)),
    }
}
