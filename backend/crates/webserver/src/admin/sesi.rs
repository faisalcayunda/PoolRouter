//! Session endpoints: `getSesi`, `perpanjangSesi`, `logout` (ADR-0003 rev 5
//! §3.6, §3.6f). The time rules and the SQL live in `core::admin`; this module
//! only maps HTTP onto them: cookie in, JSON or problem out.
//!
//! Rules that are visible at this layer:
//! - `getSesi` is a GET and never writes (the store's `status` is a SELECT).
//! - `perpanjangSesi` and `logout` are POST, so the `Origin` check runs first:
//!   a foreign origin gets 403 and the session row is not read, let alone
//!   changed (AC "terakhir_aktif tidak berubah").
//! - The extension answers 200 with the three-field body, no `Set-Cookie`, no
//!   token rotation. `logout` is the only endpoint here that sets a cookie, and
//!   it only expires it.
//! - Every failure to identify a session is the same 401, whatever the reason
//!   (unknown, idle, absolute): the client cannot tell them apart.
//! - `Cache-Control: no-store` comes from the router layer on every response.

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
use poolrouter_core::admin::origin;
use poolrouter_core::admin::sesi_store::{SesiDitolak, SesiStore};
use poolrouter_core::admin::token::HashToken;
use poolrouter_core::admin::{AsalUi, TokenSesi};
use sqlx::PgPool;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::set_header::SetResponseHeaderLayer;

use super::login;
use super::pembatas::PembatasMemori;

use crate::api::problem::ProblemKodeAdmin;
use crate::api::problem_admin::ProblemAdmin;

/// Cookie name (`__Host-` prefix: Secure, Path=/, no Domain).
pub const COOKIE_SESI: &str = "__Host-pr_session";

/// The clock is injected so tests can move time instead of sleeping.
pub type Jam = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

#[derive(Clone)]
pub struct AdminState {
    pub sesi: SesiStore,
    /// The `pr_admin_api` pool. It is the only pool admin handlers can reach;
    /// the downstream router never receives it (ADR-0001 §7.2).
    pub pool: PgPool,
    pub asal: AsalUi,
    pub jam: Jam,
    pub pembatas: Arc<PembatasMemori>,
}

impl AdminState {
    pub fn baru(pool: PgPool, asal: AsalUi) -> Self {
        Self {
            sesi: SesiStore::baru(pool.clone()),
            pool,
            asal,
            jam: Arc::new(Utc::now),
            pembatas: Arc::new(PembatasMemori::baru()),
        }
    }

    pub fn dengan_jam(mut self, jam: Jam) -> Self {
        self.jam = jam;
        self
    }
}

pub fn router(state: AdminState) -> Router {
    Router::new()
        .route("/admin/v1/auth/sesi", get(get_sesi))
        .route("/admin/v1/auth/sesi/perpanjang", post(perpanjang_sesi))
        .route("/admin/v1/auth/logout", post(logout))
        .route("/admin/v1/auth/login", post(login::login))
        .with_state(state)
        .layer(RequestBodyLimitLayer::new(16 * 1024))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
}

/// A problem response; `detail` is never set (R-16). Boxed: `ProblemAdmin` is
/// large and `Galat` travels in every `Result` (clippy::result_large_err).
pub(super) struct Galat(Box<ProblemAdmin>, Option<u64>);

impl Galat {
    pub(super) fn baru(status: StatusCode, kode: ProblemKodeAdmin) -> Self {
        Self(Box::new(ProblemAdmin::new(kode, status.as_u16())), None)
    }

    fn tidak_terautentikasi() -> Self {
        Self::baru(
            StatusCode::UNAUTHORIZED,
            ProblemKodeAdmin::TidakTerautentikasi,
        )
    }

    fn origin() -> Self {
        Self::baru(StatusCode::FORBIDDEN, ProblemKodeAdmin::OriginTidakValid)
    }

    /// 429 with `Retry-After` (whole seconds, at least 1).
    pub(super) fn dibatasi(detik: u64) -> Self {
        let mut g = Self::baru(
            StatusCode::TOO_MANY_REQUESTS,
            ProblemKodeAdmin::LoginDibatasi,
        );
        g.1 = Some(detik.max(1));
        g
    }

    pub(super) fn layanan() -> Self {
        Self::baru(
            StatusCode::SERVICE_UNAVAILABLE,
            ProblemKodeAdmin::LayananTidakTersedia,
        )
    }
}

impl IntoResponse for Galat {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::to_vec(&self.0).unwrap_or_default();
        let mut res = (status, body).into_response();
        res.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        if let Some(detik) = self.1 {
            res.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(detik));
        }
        res
    }
}

/// The session token from the `Cookie` header, if present and well formed.
/// A malformed token is treated as absent: 401 either way.
fn token_dari_cookie(headers: &HeaderMap) -> Option<TokenSesi> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pasang| pasang.trim().split_once('='))
        .find(|(nama, _)| *nama == COOKIE_SESI)
        .and_then(|(_, nilai)| TokenSesi::dari_b64(nilai).ok())
}

pub(super) fn hash_dari_cookie(headers: &HeaderMap) -> Result<HashToken, Galat> {
    token_dari_cookie(headers)
        .map(|t| t.hash())
        .ok_or_else(Galat::tidak_terautentikasi)
}

pub(super) fn periksa_origin(
    method: &Method,
    headers: &HeaderMap,
    asal: &AsalUi,
) -> Result<(), Galat> {
    origin::periksa(method.as_str(), headers, asal).map_err(|alasan| {
        tracing::warn!(alasan = %alasan, "admin: origin ditolak");
        Galat::origin()
    })
}

fn galat_store(e: &poolrouter_core::admin::GalatSesi) -> Galat {
    // The driver text can carry connection details; log the kind only.
    tracing::error!(error = %e, "admin: penyimpanan sesi gagal");
    Galat::layanan()
}

fn ditolak(alasan: SesiDitolak) -> Galat {
    tracing::debug!(?alasan, "admin: sesi ditolak");
    Galat::tidak_terautentikasi()
}

async fn get_sesi(State(s): State<AdminState>, headers: HeaderMap) -> Result<Response, Galat> {
    let hash = hash_dari_cookie(&headers)?;
    let status = s
        .sesi
        .status(&hash, (s.jam)())
        .await
        .map_err(|e| galat_store(&e))?
        .map_err(ditolak)?;
    Ok((StatusCode::OK, axum::Json(status)).into_response())
}

async fn perpanjang_sesi(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, Galat> {
    periksa_origin(&method, &headers, &s.asal)?;
    let hash = hash_dari_cookie(&headers)?;
    let status = s
        .sesi
        .perpanjang(&hash, (s.jam)())
        .await
        .map_err(|e| galat_store(&e))?
        .map_err(ditolak)?;
    Ok((StatusCode::OK, axum::Json(status)).into_response())
}

async fn logout(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, Galat> {
    periksa_origin(&method, &headers, &s.asal)?;
    let hash = hash_dari_cookie(&headers)?;
    let ada = s.sesi.cabut(&hash).await.map_err(|e| galat_store(&e))?;
    if !ada {
        return Err(Galat::tidak_terautentikasi());
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "__Host-pr_session=; Max-Age=0; Path=/; HttpOnly; Secure; SameSite=Strict",
        ),
    );
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dengan_cookie(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_str(v).unwrap());
        h
    }

    #[test]
    fn token_dipilih_dari_antara_cookie_lain() {
        let t = TokenSesi::baru();
        let h = dengan_cookie(&format!("a=1; {}={}; b=2", COOKIE_SESI, t.sebagai_b64()));
        assert!(token_dari_cookie(&h).unwrap() == t);
    }

    #[test]
    fn cookie_salah_bentuk_dianggap_tidak_ada() {
        assert!(token_dari_cookie(&dengan_cookie(&format!("{COOKIE_SESI}=pendek"))).is_none());
        assert!(token_dari_cookie(&dengan_cookie("pr_session=x")).is_none());
        assert!(token_dari_cookie(&HeaderMap::new()).is_none());
    }

    #[test]
    fn nama_cookie_harus_persis() {
        let t = TokenSesi::baru();
        let h = dengan_cookie(&format!("x{}={}", COOKIE_SESI, t.sebagai_b64()));
        assert!(token_dari_cookie(&h).is_none());
    }
}
