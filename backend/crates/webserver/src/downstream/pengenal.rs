//! Credential resolution for the downstream operations (ARC-158, AC-3).
//!
//! Bearer OAuth2 tokens and API keys arrive at the same five operations, and
//! both end in the same pair `(client_id, credential_id)`; the store then scopes
//! every read and write by `client_id`. The resolver only wraps the existing
//! [`Authenticator`], so the 30 s credential cache and the hashing budget stay
//! where they already live.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;
use uuid::Uuid;

use super::auth::{API_KEY_HEADER, AuthError, Authenticator};
use super::downstream_sesi::SesiKlien;

/// Why the caller could not be identified. Both arms turn into a documented
/// problem and neither carries a cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GalatPengenal {
    /// No credential, or one that is refused.
    Ditolak,
    /// The credential store did not answer: fail closed.
    Layanan,
}

impl From<AuthError> for GalatPengenal {
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::Fail(_) => Self::Ditolak,
            AuthError::Unavailable => Self::Layanan,
        }
    }
}

pub type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Identifies the client behind a downstream request.
pub trait PengenalDownstream: Send + Sync + 'static {
    fn kenali<'a>(&'a self, headers: &'a HeaderMap) -> Fut<'a, Result<SesiKlien, GalatPengenal>>;
}

/// Production resolver: `Authorization: Bearer <token>` first, then
/// `X-Api-Key: pr_<credential_id>_<secret>`.
pub struct PengenalNyata {
    auth: Arc<Authenticator>,
}

impl PengenalNyata {
    pub fn baru(auth: Arc<Authenticator>) -> Arc<Self> {
        Arc::new(Self { auth })
    }
}

/// The token endpoint's claims carry the issuing credential; the API key path
/// keeps the credential id in the key itself, so both give a credential id.
impl PengenalDownstream for PengenalNyata {
    fn kenali<'a>(&'a self, headers: &'a HeaderMap) -> Fut<'a, Result<SesiKlien, GalatPengenal>> {
        Box::pin(async move { self.kenali_dalam(headers).await })
    }
}

impl PengenalNyata {
    async fn kenali_dalam(&self, headers: &HeaderMap) -> Result<SesiKlien, GalatPengenal> {
        let bearer = headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|v| !v.is_empty());
        if let Some(token) = bearer {
            let (client_id, credential_id) = self.auth.bearer_detail(token).await?;
            return Ok(SesiKlien {
                client_id,
                credential_id,
            });
        }
        let kunci = headers
            .get(API_KEY_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|v| !v.is_empty());
        match kunci {
            None => Err(GalatPengenal::Ditolak),
            Some(k) => {
                let (client_id, credential_id) = self.auth.api_key_detail(k).await?;
                Ok(SesiKlien {
                    client_id,
                    credential_id,
                })
            }
        }
    }
}

/// `Uuid` parsing helper shared by the handlers' path extractors.
pub fn urai_uuid(mentah: &str) -> Option<Uuid> {
    Uuid::parse_str(mentah).ok()
}
