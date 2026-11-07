//! Shared HTTP pieces of the downstream API: RFC 9457 error responses with
//! `Retry-After` (R-10) and the root `Cache-Control: no-store` layer (R-16).

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::api::problem::{JumlahNamaTidakCocok, Problem, ProblemKodeDown, ValidasiErrorDown};

const PROBLEM_JSON: &str = "application/problem+json";

/// Error that renders as `application/problem+json`.
///
/// `detail` never carries input values, NIK or secrets: callers pass fixed text only.
#[derive(Debug, Clone)]
pub struct ApiError {
    // Boxed: `Problem` is large and ApiError travels in every `Result` (clippy::result_large_err).
    problem: Box<Problem>,
    retry_after: Option<u64>,
    www_authenticate: Option<&'static str>,
}

impl ApiError {
    pub fn new(status: StatusCode, kode: ProblemKodeDown) -> Self {
        Self {
            problem: Box::new(Problem::new(kode, status.as_u16())),
            retry_after: None,
            www_authenticate: None,
        }
    }

    pub fn detail(mut self, detail: &'static str) -> Self {
        self.problem.detail = Some(detail.to_owned());
        self
    }

    /// RFC 6749 `error` member, token endpoint only.
    pub fn oauth_error(mut self, error: &'static str) -> Self {
        self.problem.error = Some(error.to_owned());
        self
    }

    /// `Retry-After` in whole seconds, at least 1.
    pub fn retry_after(mut self, secs: u64) -> Self {
        self.retry_after = Some(secs.max(1));
        self
    }

    pub fn www_authenticate(mut self, challenge: &'static str) -> Self {
        self.www_authenticate = Some(challenge);
        self
    }

    /// 401 `tidak_terautentikasi`. The cause is never distinguished to the client.
    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ProblemKodeDown::TidakTerautentikasi,
        )
        .www_authenticate("Bearer")
    }

    /// 503 `layanan_tidak_tersedia` with the fixed `Retry-After: 5` (ADR-0001 5a.7).
    pub fn unavailable() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ProblemKodeDown::LayananTidakTersedia,
        )
        .retry_after(5)
    }

    pub fn status(&self) -> StatusCode {
        StatusCode::from_u16(self.problem.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }

    /// `Retry-After` this error will carry, if any.
    pub fn retry_after_secs(&self) -> Option<u64> {
        self.retry_after
    }

    /// Field-level validation detail for the two 422 validation codes. Both are
    /// omitted when empty, so a bare 400 never grows an `errors` member.
    pub fn errors(
        mut self,
        errors: Vec<ValidasiErrorDown>,
        jumlah: Option<JumlahNamaTidakCocok>,
    ) -> Self {
        if !errors.is_empty() {
            self.problem.errors = Some(errors);
        }
        self.problem.jumlah_nama_tidak_cocok = jumlah;
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        // Serializing a plain struct of strings cannot fail; fall back to an empty
        // body rather than panic in the error path.
        let body = serde_json::to_vec(&self.problem).unwrap_or_default();
        let mut res = (status, body).into_response();
        let h = res.headers_mut();
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static(PROBLEM_JSON));
        if let Some(secs) = self.retry_after {
            h.insert(header::RETRY_AFTER, HeaderValue::from(secs));
        }
        if let Some(challenge) = self.www_authenticate {
            h.insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static(challenge),
            );
        }
        res
    }
}

/// `Cache-Control: no-store` on every response, installed once at the router root.
pub fn no_store() -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::to_bytes;
    use axum::routing::get;
    use tower::ServiceExt;

    async fn body_json(res: Response) -> serde_json::Value {
        let bytes = to_bytes(res.into_body(), 64 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn problem_json_shape_and_headers() {
        let res = ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            ProblemKodeDown::TokenDibatasi,
        )
        .retry_after(0)
        .into_response();
        assert_eq!(res.status(), 429);
        assert_eq!(res.headers()[header::CONTENT_TYPE], PROBLEM_JSON);
        assert_eq!(
            res.headers()[header::RETRY_AFTER],
            "1",
            "floor of one second"
        );
        let v = body_json(res).await;
        assert_eq!(v["kode"], "token_dibatasi");
        assert_eq!(v["status"], 429);
        assert_eq!(
            v["type"],
            "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#token_dibatasi"
        );
        assert!(
            v.get("error").is_none(),
            "429 never carries an RFC 6749 error"
        );
    }

    #[tokio::test]
    async fn unauthorized_challenges_and_hides_cause() {
        let res = ApiError::unauthorized().into_response();
        assert_eq!(res.status(), 401);
        assert_eq!(res.headers()[header::WWW_AUTHENTICATE], "Bearer");
        assert!(res.headers().get(header::RETRY_AFTER).is_none());
        assert_eq!(body_json(res).await["kode"], "tidak_terautentikasi");
    }

    #[tokio::test]
    async fn unavailable_has_fixed_retry_after_five() {
        let res = ApiError::unavailable().into_response();
        assert_eq!(res.status(), 503);
        assert_eq!(res.headers()[header::RETRY_AFTER], "5");
    }

    #[tokio::test]
    async fn no_store_covers_ok_and_error_responses() {
        let app = Router::new()
            .route("/ok", get(|| async { "x" }))
            .route("/err", get(|| async { ApiError::unauthorized() }))
            .layer(no_store());
        for path in ["/ok", "/err", "/missing"] {
            let res = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store", "{path}");
        }
    }
}
