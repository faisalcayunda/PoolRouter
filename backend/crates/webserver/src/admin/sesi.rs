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
use axum::routing::{get, post, put};
use chrono::{DateTime, Utc};
use poolrouter_core::admin::origin;
use poolrouter_core::admin::sesi_store::{SesiDitolak, SesiStore};
use poolrouter_core::admin::token::HashToken;
use poolrouter_core::admin::{AsalUi, TokenSesi};
use poolrouter_core::secret::Brankas;
use sqlx::PgPool;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::set_header::SetResponseHeaderLayer;

use super::pembatas::PembatasLogin;
<<<<<<< HEAD
use super::{allowlist, klien, kredensial, log, login, me, provider, secret, webhook};
=======
use super::{allowlist, klien, kredensial, login, me, provider, secret, webhook};
>>>>>>> 1173337 (fix(admin): register provider routes and send valid log test origin)

use crate::api::problem::{ProblemKodeAdmin, ValidasiErrorAdmin};
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
    pub pembatas: Arc<PembatasLogin>,
    /// The KEK-backed envelope for `provider_secret` (AC-8, AC-11).
    pub brankas: Arc<Brankas>,
}

impl AdminState {
    pub fn baru(pool: PgPool, asal: AsalUi, pembatas: PembatasLogin, brankas: Brankas) -> Self {
        Self {
            sesi: SesiStore::baru(pool.clone()),
            pool,
            asal,
            jam: Arc::new(Utc::now),
            pembatas: Arc::new(pembatas),
            brankas: Arc::new(brankas),
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
        .route("/admin/v1/auth/me", get(me::get_me))
        .route(
            "/admin/v1/auth/me/preferensi",
            axum::routing::patch(me::update_preferensi),
        )
        .route("/admin/v1/secrets", get(secret::list_secrets))
        // T7: request log, auditor search, payload opening, export, audit read.
        .route("/admin/v1/log/cari", post(log::search_log))
        .route("/admin/v1/log/{kode_transaksi}", get(log::get_log_entry))
        .route(
            "/admin/v1/auditor/log/cari-nik",
            post(log::search_log_by_nik),
        )
        .route("/admin/v1/auditor/payload", post(log::open_payload))
        .route(
            "/admin/v1/auditor/log/ekspor",
            post(log::export_log_auditor),
        )
        .route(
            "/admin/v1/auditor/audit-akses/cari",
            post(log::search_audit_akses),
        )
        .route(
            "/admin/v1/secrets/{nama}",
            put(secret::put_secret).delete(secret::delete_secret),
        )
        // An empty name arrives as a path with no segment. It is a validation
        // failure (AC-7), not an unrouted path, so it reaches the same handlers.
        .route(
            "/admin/v1/secrets/",
            put(secret::put_secret_kosong).delete(secret::delete_secret_kosong),
        )
        .route(
            "/admin/v1/clients",
            get(klien::list_clients).post(klien::create_client),
        )
        .route(
            "/admin/v1/providers",
            get(provider::list_providers).post(provider::create_provider),
        )
        .route(
            "/admin/v1/providers/{id}",
            get(provider::get_provider).put(provider::update_provider),
        )
        .route(
            "/admin/v1/clients/{id}",
            get(klien::get_client).put(klien::update_client),
        )
        .route(
            "/admin/v1/clients/{id}/credentials",
            get(kredensial::list_credentials).post(kredensial::create_credential),
        )
        .route(
            "/admin/v1/clients/{id}/webhook",
            get(webhook::get_webhook_config).put(webhook::put_webhook_config),
        )
        .route(
            "/admin/v1/clients/{id}/webhook/rotasi-secret",
            post(webhook::rotate_webhook_secret),
        )
        .route(
            "/admin/v1/clients/{id}/webhook/log",
            get(webhook::list_webhook_log),
        )
        .route(
            "/admin/v1/webhook-allowlist",
            get(allowlist::list_allowlist),
        )
        .route(
            "/admin/v1/webhook-allowlist/{domain}",
            put(allowlist::put_allowlist_entry).delete(allowlist::delete_allowlist_entry),
        )
        .route("/admin/v1/providers", get(provider::list_providers).post(provider::create_provider))
        .route(
            "/admin/v1/providers/{id}",
            get(provider::get_provider).put(provider::update_provider),
        )
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

    pub(super) fn tidak_terautentikasi() -> Self {
        Self::baru(
            StatusCode::UNAUTHORIZED,
            ProblemKodeAdmin::TidakTerautentikasi,
        )
    }

    /// 422 `validasi_gagal` per failing field (AC-7).
    pub(super) fn validasi(errors: Vec<ValidasiErrorAdmin>) -> Self {
        let mut problem = ProblemAdmin::new(ProblemKodeAdmin::ValidasiGagal, 422);
        problem.errors = Some(errors);
        Self(Box::new(problem), None)
    }

    /// 409 with a `detail` that describes the conflict without echoing a value
    /// (the provider names that still use a secret, the credential that is still
    /// in use).
    pub(super) fn konflik(kode: ProblemKodeAdmin, detail: impl Into<String>) -> Self {
        let mut problem = ProblemAdmin::new(kode, 409);
        problem.detail = Some(detail.into());
        Self(Box::new(problem), None)
    }

    pub(super) fn tidak_ditemukan() -> Self {
        Self::baru(StatusCode::NOT_FOUND, ProblemKodeAdmin::TidakDitemukan)
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
