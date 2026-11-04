//! `POST /v1/oauth/token` (OAuth2 client credentials, ADR-0001 rev 15 5a.5).
//!
//! Order of work: per-IP throttle, form parsing, grant check, credential
//! extraction, per-client throttle, authentication, token. The throttle runs
//! before authentication so secret guessing is rate limited, and it is a
//! throttle, not a lockout. Every refusal is RFC 9457 `problem+json`; 401
//! always carries `error: invalid_client` and never says why.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use poolrouter_core::kredensial::base64url_decode;
use uuid::Uuid;

use super::gcra::{Gcra, retry_after_secs};
use super::layers::ApiError;
use super::server::AppState;
use crate::api::downstream::{TipeToken, TokenRequest, TokenResponse};
use crate::api::problem::ProblemKodeDown;

/// Throttle of the token endpoint: per source IP and per `client_id`.
pub trait TokenThrottle: Send + Sync + 'static {
    fn ip(&self, ip: IpAddr) -> Result<(), Duration>;
    fn client(&self, client_id: Uuid) -> Result<(), Duration>;
}

/// In-process GCRA. Transitional: the F0 decision (Shu, SHAA-198) is a Redis
/// throttle shared by all replicas, delivered with part 2.
pub struct LocalThrottle {
    ip: Gcra,
    client: Gcra,
}

impl LocalThrottle {
    /// 120/min per IP and 60/min per client, both with a burst of 10.
    pub fn new() -> Self {
        Self {
            ip: Gcra::per_minute(120, 10),
            client: Gcra::per_minute(60, 10),
        }
    }
}

impl Default for LocalThrottle {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenThrottle for LocalThrottle {
    fn ip(&self, ip: IpAddr) -> Result<(), Duration> {
        self.ip.check(&ip.to_string())
    }
    fn client(&self, client_id: Uuid) -> Result<(), Duration> {
        self.client.check(&client_id.to_string())
    }
}

fn count(alasan: &'static str) {
    metrics::counter!("poolrouter_token_gagal_total", "alasan" => alasan).increment(1);
}

fn bad_request(alasan: &'static str, error: &'static str) -> ApiError {
    count(alasan);
    ApiError::new(StatusCode::BAD_REQUEST, ProblemKodeDown::ValidasiGagal).oauth_error(error)
}

fn throttled(wait: Duration) -> ApiError {
    count("dibatasi");
    ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        ProblemKodeDown::TokenDibatasi,
    )
    .retry_after(retry_after_secs(wait))
}

/// `Authorization: Basic base64(client_id:client_secret)`.
fn basic_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let b64 = value.strip_prefix("Basic ")?.trim();
    // Standard alphabet and padding to the URL alphabet the decoder knows.
    let url: String = b64
        .chars()
        .filter(|c| *c != '=')
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            o => o,
        })
        .collect();
    let raw = String::from_utf8(base64url_decode(&url)?).ok()?;
    let (id, secret) = raw.split_once(':')?;
    Some((id.to_owned(), secret.to_owned()))
}

pub async fn issue_token(
    State(app): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<TokenResponse>, ApiError> {
    app.throttle.ip(peer.ip()).map_err(throttled)?;

    let is_form = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"));
    if !is_form {
        return Err(bad_request("permintaan_tidak_valid", "invalid_request"));
    }
    let form: TokenRequest = serde_urlencoded::from_bytes(&body)
        .map_err(|_| bad_request("permintaan_tidak_valid", "invalid_request"))?;
    if form.grant_type != "client_credentials" {
        return Err(bad_request(
            "grant_tidak_didukung",
            "unsupported_grant_type",
        ));
    }

    let (client_id, secret) = match (
        basic_credentials(&headers),
        form.client_id,
        form.client_secret,
    ) {
        (Some(basic), None, None) => basic,
        (None, Some(id), Some(secret)) => (id, secret),
        _ => return Err(bad_request("permintaan_tidak_valid", "invalid_request")),
    };

    // Unparsable ids skip the per-client bucket: the IP bucket already applies
    // and the id cannot be a real client.
    if let Ok(id) = Uuid::parse_str(&client_id) {
        app.throttle.client(id).map_err(throttled)?;
    }

    let client = app
        .auth
        .client_secret(&client_id, &secret)
        .await
        .map_err(|e| {
            let err = e.into_api_error();
            if err.status() == StatusCode::UNAUTHORIZED {
                err.oauth_error("invalid_client").www_authenticate("Basic")
            } else {
                err
            }
        })?;

    let now = jsonwebtoken::get_current_timestamp();
    // A signing failure is a server fault, not a bad credential.
    let (access_token, expires_in) = app
        .auth
        .keys()
        .issue(client, now)
        .map_err(|_| ApiError::unavailable())?;
    Ok(Json(TokenResponse {
        access_token,
        token_type: TipeToken::Bearer,
        expires_in: i64::try_from(expires_in).unwrap_or(900),
    }))
}
