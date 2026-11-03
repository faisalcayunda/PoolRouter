//! Downstream API (consumed by BGN applications outside this repo). After
//! SHAA-175 closes, breaking changes need a new API version.

use axum::{Json, http::StatusCode};
use serde::Serialize;
use utoipa::{OpenApi, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::{enums::ApiStatus, problem::Problem};

#[derive(Debug, Serialize, ToSchema)]
pub struct EnqueueResponse {
    /// Equals `request_id` (ADR-0001 section 4.1).
    pub queue_id: Uuid,
    pub status: ApiStatus,
}

#[derive(OpenApi)]
#[openapi(
    info(title = "PoolRouter downstream API", version = "0.1.0"),
    components(schemas(Problem, ApiStatus, EnqueueResponse))
)]
pub struct DownstreamDoc;

/// Enqueue an async validation request.
#[utoipa::path(
    post,
    path = "/v1/async",
    operation_id = "enqueueAsync",
    responses((status = 501, description = "Not implemented", body = Problem, content_type = "application/problem+json"))
)]
async fn enqueue_async() -> (StatusCode, Json<Problem>) {
    (StatusCode::NOT_IMPLEMENTED, Json(Problem::not_implemented()))
}

pub fn router() -> OpenApiRouter {
    OpenApiRouter::with_openapi(DownstreamDoc::openapi()).routes(routes!(enqueue_async))
}

/// OpenAPI document as YAML, used by `openapi-gen`.
pub fn yaml() -> Result<String, utoipa::Error> {
    let (_, api) = router().split_for_parts();
    api.to_yaml()
}
