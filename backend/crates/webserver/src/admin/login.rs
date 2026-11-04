//! `POST /admin/v1/auth/login` (ADR-0003 rev 5 §3.6).
//!
//! What the handler guarantees:
//! - The `Origin` check runs before anything else, including the body.
//! - Unknown user, inactive user and wrong password are the same 401
//!   `kredensial_salah`, and cost the same: a missing user is verified against
//!   a fixed dummy hash, so response time does not name the account.
//! - 5 failures per username in 15 minutes lock that username for 15 minutes
//!   (429 `login_dibatasi` with `Retry-After`); a locked request does no
//!   password work at all.
//! - Every attempt writes `audit_login` (no password, no typed username). A
//!   login that cannot be audited is undone: the session is deleted and the
//!   answer is 503, never a cookie for an unrecorded session.
//! - Success is 204 with `__Host-pr_session`; the token appears only there.

use std::net::{IpAddr, SocketAddr};
use std::sync::OnceLock;

use axum::extract::rejection::JsonRejection;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use poolrouter_core::admin::TokenSesi;
use poolrouter_core::admin::password::{
    KebijakanPassword, PANJANG_MAKSIMUM, ParameterArgon2, hash_password, verifikasi_password,
};
use poolrouter_core::admin::pengguna::cari_login;
use poolrouter_core::audit::{AksiLogin, HasilLogin, LoginBaru, catat_login};
use uuid::Uuid;

use super::sesi::{AdminState, COOKIE_SESI, Galat, periksa_origin};
use crate::api::admin::LoginRequest;
use crate::api::problem::ProblemKodeAdmin;

const PANJANG_USERNAME: std::ops::RangeInclusive<usize> = 3..=100;

/// A valid argon2id hash of a throwaway string, made once. Unknown users are
/// verified against it so they cost what a real user costs.
fn hash_palsu() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        hash_password(
            "hash-palsu-untuk-menyamakan-waktu",
            KebijakanPassword::f0(),
            ParameterArgon2::OWASP,
        )
        .map(|h| h.to_string())
        .unwrap_or_default()
    })
}

/// The address recorded in `audit_login`. Behind the reverse proxy the peer is
/// the proxy, so `X-Real-IP` wins when it is a well-formed address; the admin
/// listener is internal-only, so only the proxy can set it.
fn sumber_ip(headers: &HeaderMap, peer: SocketAddr) -> String {
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<IpAddr>().ok())
        .unwrap_or_else(|| peer.ip())
        .to_string()
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.chars().take(512).collect())
}

fn cookie_sesi(token: &TokenSesi) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{COOKIE_SESI}={}; Path=/; HttpOnly; Secure; SameSite=Strict",
        token.sebagai_b64()
    ))
    .expect("token base64url selalu ASCII")
}

/// Verify on a blocking thread: argon2id is deliberately slow and must not
/// stall the async workers.
async fn cocok(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || verifikasi_password(&password, &hash))
        .await
        .unwrap_or(false)
}

pub(super) async fn login(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    body: Result<axum::Json<LoginRequest>, JsonRejection>,
) -> Result<Response, Galat> {
    periksa_origin(&method, &headers, &s.asal)?;
    let axum::Json(req) = body.map_err(|_| {
        Galat::baru(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemKodeAdmin::ValidasiGagal,
        )
    })?;
    let sekarang = (s.jam)();
    let username = req.username.trim().to_lowercase();
    let ip = sumber_ip(&headers, peer);
    let ua = user_agent(&headers);

    if let Some(detik) = s.pembatas.terkunci(&username, sekarang) {
        audit(&s, None, HasilLogin::Gagal, None, &ip, &ua).await;
        return Err(Galat::dibatasi(detik));
    }

    let pengguna = if PANJANG_USERNAME.contains(&username.chars().count()) {
        cari_login(&s.pool, &username).await.map_err(|e| {
            tracing::error!(error = %e, "admin: pencarian pengguna gagal");
            Galat::layanan()
        })?
    } else {
        None
    };

    let hash = pengguna
        .as_ref()
        .map_or_else(|| hash_palsu().to_owned(), |p| p.password_hash.clone());
    let password_wajar = req.password.len() <= PANJANG_MAKSIMUM;
    let benar = password_wajar && cocok(req.password, hash).await;

    match pengguna {
        Some(p) if benar && p.aktif => {
            let token = TokenSesi::baru();
            let sesi = s
                .sesi
                .terbitkan(p.pengguna_id, &token.hash(), ua.as_deref(), sekarang)
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "admin: penerbitan sesi gagal");
                    Galat::layanan()
                })?;
            let dicatat = audit(
                &s,
                Some(p.pengguna_id),
                HasilLogin::Berhasil,
                Some(sesi.sesi_id),
                &ip,
                &ua,
            )
            .await;
            if !dicatat {
                // No session without a trail.
                let _ = s.sesi.cabut(&token.hash()).await;
                return Err(Galat::layanan());
            }
            s.pembatas.berhasil(&username);
            let mut res = StatusCode::NO_CONTENT.into_response();
            res.headers_mut()
                .insert(header::SET_COOKIE, cookie_sesi(&token));
            Ok(res)
        }
        gagal => {
            s.pembatas.gagal(&username, sekarang);
            audit(
                &s,
                gagal.map(|p| p.pengguna_id),
                HasilLogin::Gagal,
                None,
                &ip,
                &ua,
            )
            .await;
            Err(Galat::baru(
                StatusCode::UNAUTHORIZED,
                ProblemKodeAdmin::KredensialSalah,
            ))
        }
    }
}

/// Write one `audit_login` row. Returns whether it was written.
async fn audit(
    s: &AdminState,
    pengguna_id: Option<Uuid>,
    hasil: HasilLogin,
    sesi_id: Option<Uuid>,
    ip: &str,
    ua: &Option<String>,
) -> bool {
    let kejadian = LoginBaru {
        pengguna_id,
        aksi: AksiLogin::Login,
        hasil,
        sesi_id: sesi_id.map(|u| u.to_string()),
        sumber_ip: Some(ip.to_owned()),
        user_agent: ua.clone(),
    };
    let ok = async {
        let mut conn = s.pool.acquire().await.map_err(|e| e.to_string())?;
        catat_login(&mut conn, &kejadian)
            .await
            .map_err(|e| e.to_string())
    }
    .await;
    match ok {
        Ok(()) => true,
        Err(e) => {
            tracing::error!(error = %e, "admin: audit_login gagal ditulis");
            false
        }
    }
}
