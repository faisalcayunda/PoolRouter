//! Client authentication for the downstream API (ADR-0001 rev 15 5a.5, SHAA-198).
//!
//! Three entry points: client secret (token endpoint), Bearer token and API key
//! (every other endpoint). The client always sees the same 401; the cause only
//! reaches the closed `alasan` metric label (MT-03).
//!
//! Input that cannot be a credential (non-UUID `client_id`, malformed API key)
//! is refused before any store or argon2 work.

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use chrono::Utc;
use poolrouter_core::kredensial::{
    PARAMETER_SECRET, cocok_secret, cocok_secret_klien, hash_palsu, urai_api_key,
};
use tokio::sync::Semaphore;
use uuid::Uuid;

use super::credential::{CredentialStore, Jenis, StoreError};
use super::jwt::{JwtKeys, TokenError};
use super::layers::ApiError;
use super::server::AppState;

/// Concurrent argon2 verifications. Each one holds about 19 MiB.
const MAX_PARALLEL_HASHING: usize = 4;
/// Header carrying an API key.
pub const API_KEY_HEADER: &str = "x-api-key";

/// Why authentication failed. `alasan` is the closed label set of
/// `poolrouter_token_gagal_total` (ADR-0001 5a.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFail {
    KredensialTidakDikenal,
    SecretSalah,
    KredensialNonaktif,
    KredensialKedaluwarsa,
    TokenTidakValid,
    TokenKedaluwarsa,
    ApiKeyTidakValid,
}

impl AuthFail {
    pub fn alasan(self) -> &'static str {
        match self {
            Self::KredensialTidakDikenal => "kredensial_tidak_dikenal",
            Self::SecretSalah => "secret_salah",
            Self::KredensialNonaktif => "kredensial_nonaktif",
            Self::KredensialKedaluwarsa => "kredensial_kedaluwarsa",
            Self::TokenTidakValid => "token_tidak_valid",
            Self::TokenKedaluwarsa => "token_kedaluwarsa",
            Self::ApiKeyTidakValid => "api_key_tidak_valid",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    /// Refused: answer 401 and count `alasan`.
    Fail(AuthFail),
    /// The store did not answer: fail closed with 503.
    Unavailable,
}

impl From<StoreError> for AuthError {
    fn from(_: StoreError) -> Self {
        Self::Unavailable
    }
}

impl From<AuthFail> for AuthError {
    fn from(f: AuthFail) -> Self {
        Self::Fail(f)
    }
}

impl From<TokenError> for AuthFail {
    fn from(e: TokenError) -> Self {
        match e {
            TokenError::Invalid => Self::TokenTidakValid,
            TokenError::Expired => Self::TokenKedaluwarsa,
        }
    }
}

impl AuthError {
    /// Count the failure and build the response. Only refusals are counted;
    /// an unavailable store is not an authentication failure.
    pub fn into_api_error(self) -> ApiError {
        match self {
            Self::Fail(f) => {
                metrics::counter!("poolrouter_token_gagal_total", "alasan" => f.alasan())
                    .increment(1);
                ApiError::unauthorized()
            }
            Self::Unavailable => ApiError::unavailable(),
        }
    }
}

pub struct Authenticator {
    store: Arc<dyn CredentialStore>,
    keys: Arc<JwtKeys>,
    hashing: Arc<Semaphore>,
    /// One dummy PHC string, computed once: unknown clients pay the same cost.
    dummy: Arc<[u8]>,
}

impl Authenticator {
    pub fn new(store: Arc<dyn CredentialStore>, keys: Arc<JwtKeys>) -> Self {
        Self {
            store,
            keys,
            hashing: Arc::new(Semaphore::new(MAX_PARALLEL_HASHING)),
            dummy: Arc::from(hash_palsu(PARAMETER_SECRET).as_bytes()),
        }
    }

    pub fn keys(&self) -> &JwtKeys {
        &self.keys
    }

    /// Verify `secret` against an argon2id PHC string off the async threads.
    async fn argon2_cocok(&self, secret: &str, phc: Vec<u8>) -> bool {
        let Ok(_permit) = self.hashing.clone().acquire_owned().await else {
            return false;
        };
        let secret = secret.to_owned();
        tokio::task::spawn_blocking(move || cocok_secret_klien(&secret, &phc))
            .await
            .unwrap_or(false)
    }

    /// Same cost as one verification, for unknown clients, so response time does
    /// not reveal whether a `client_id` exists.
    async fn argon2_palsu(&self, secret: &str) {
        let Ok(_permit) = self.hashing.clone().acquire_owned().await else {
            return;
        };
        let secret = secret.to_owned();
        let dummy = self.dummy.clone();
        let _ = tokio::task::spawn_blocking(move || cocok_secret_klien(&secret, &dummy)).await;
    }

    /// Token endpoint: `client_id` (UUID text) and client secret. Returns the
    /// client and the credential that matched, which becomes the token's `cid`.
    pub async fn client_secret(
        &self,
        client_id: &str,
        secret: &str,
    ) -> Result<(Uuid, Uuid), AuthError> {
        let client_id = Uuid::parse_str(client_id).map_err(|_| AuthFail::KredensialTidakDikenal)?;
        let now = Utc::now();
        let semua = self.store.kredensial_client(client_id).await?;
        let rahasia: Vec<_> = semua
            .iter()
            .filter(|k| k.jenis == Jenis::ClientSecret)
            .collect();
        let aktif: Vec<_> = rahasia.iter().filter(|k| k.aktif).collect();
        let layak: Vec<_> = aktif.iter().filter(|k| k.belum_kedaluwarsa(now)).collect();

        if layak.is_empty() {
            self.argon2_palsu(secret).await;
            return Err(if rahasia.is_empty() {
                AuthFail::KredensialTidakDikenal
            } else if aktif.is_empty() {
                AuthFail::KredensialNonaktif
            } else {
                AuthFail::KredensialKedaluwarsa
            }
            .into());
        }
        let mut cocok = None;
        for k in layak.iter().take(2) {
            if self.argon2_cocok(secret, k.kunci_hash.clone()).await {
                cocok = Some(k.credential_id);
                break;
            }
        }
        let Some(credential_id) = cocok else {
            return Err(AuthFail::SecretSalah.into());
        };
        // Status is checked after the secret so that it cannot be probed without it.
        match self.store.client_aktif(client_id).await? {
            Some(true) => Ok((client_id, credential_id)),
            _ => Err(AuthFail::KredensialNonaktif.into()),
        }
    }

    /// Bearer token: signature and claims, then the client must still be active
    /// and the credential that issued the token (`cid`) must belong to it, still
    /// be an active client secret and not expired. Revoking one secret therefore
    /// revokes the tokens issued from it (ADR-0001 rev 15 5a.5).
    pub async fn bearer(&self, token: &str) -> Result<Uuid, AuthError> {
        let claims = self.keys.verify(token).map_err(AuthFail::from)?;
        let client_id = claims.sub;
        match self.store.client_aktif(client_id).await? {
            Some(true) => {}
            Some(false) => return Err(AuthFail::KredensialNonaktif.into()),
            None => return Err(AuthFail::TokenTidakValid.into()),
        }
        let k = self
            .store
            .kredensial(claims.cid)
            .await?
            .filter(|k| k.client_id == client_id)
            .ok_or(AuthFail::TokenTidakValid)?;
        if k.jenis != Jenis::ClientSecret {
            return Err(AuthFail::TokenTidakValid.into());
        }
        if !k.aktif {
            return Err(AuthFail::KredensialNonaktif.into());
        }
        if !k.belum_kedaluwarsa(Utc::now()) {
            return Err(AuthFail::KredensialKedaluwarsa.into());
        }
        Ok(client_id)
    }

    /// API key `pr_<credential_id>_<secret>`.
    pub async fn api_key(&self, key: &str) -> Result<Uuid, AuthError> {
        let parsed = urai_api_key(key).ok_or(AuthFail::ApiKeyTidakValid)?;
        let k = self
            .store
            .kredensial(parsed.credential_id)
            .await?
            .filter(|k| k.jenis == Jenis::ApiKey)
            .ok_or(AuthFail::ApiKeyTidakValid)?;
        // Secret first: status and expiry are only revealed to the key holder.
        if !cocok_secret(parsed.secret, &k.kunci_hash) {
            return Err(AuthFail::ApiKeyTidakValid.into());
        }
        if !k.aktif {
            return Err(AuthFail::KredensialNonaktif.into());
        }
        if !k.belum_kedaluwarsa(Utc::now()) {
            return Err(AuthFail::KredensialKedaluwarsa.into());
        }
        match self.store.client_aktif(k.client_id).await? {
            Some(true) => Ok(k.client_id),
            _ => Err(AuthFail::KredensialNonaktif.into()),
        }
    }
}

/// The authenticated client of a request. Use as a handler argument on every
/// endpoint except the token endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthClient(pub Uuid);

impl FromRequestParts<AppState> for AuthClient {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let bearer = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        let result = if let Some(token) = bearer {
            state.auth.bearer(token.trim()).await
        } else if let Some(key) = parts
            .headers
            .get(API_KEY_HEADER)
            .and_then(|v| v.to_str().ok())
        {
            state.auth.api_key(key.trim()).await
        } else {
            return Err(ApiError::unauthorized());
        };
        result.map(AuthClient).map_err(AuthError::into_api_error)
    }
}

#[cfg(test)]
mod tests {
    use super::super::credential::{Kredensial, fake::FakeStore};
    use super::*;
    use argon2::{Algorithm, Argon2, Params, PasswordHasher, Version};
    use poolrouter_core::kredensial::buat_api_key;

    const ISS: &str = "https://poolrouter.example.test";

    /// Cheap parameters so unoptimised test builds stay fast; the PHC text
    /// carries them, so verification needs no matching configuration.
    fn phc(secret: &str) -> Vec<u8> {
        Argon2::new(
            Algorithm::Argon2id,
            Version::V0x13,
            Params::new(8, 1, 1, None).unwrap(),
        )
        .hash_password_with_salt(secret.as_bytes(), b"0123456789abcdef")
        .unwrap()
        .to_string()
        .into_bytes()
    }

    fn keys() -> Arc<JwtKeys> {
        use aws_lc_rs::rand::SystemRandom;
        use aws_lc_rs::signature::Ed25519KeyPair;
        let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let b = poolrouter_core::kredensial::base64url_encode(doc.as_ref())
            .replace('-', "+")
            .replace('_', "/");
        let pem = format!("-----BEGIN PRIVATE KEY-----\n{b}\n-----END PRIVATE KEY-----\n");
        Arc::new(JwtKeys::from_pem(ISS, &pem, None).unwrap())
    }

    fn secret_cred(client: Uuid, secret: &str) -> Kredensial {
        Kredensial {
            credential_id: Uuid::now_v7(),
            client_id: client,
            jenis: Jenis::ClientSecret,
            kunci_hash: phc(secret),
            aktif: true,
            kedaluwarsa_at: None,
        }
    }

    struct Env {
        fake: Arc<FakeStore>,
        auth: Authenticator,
        client: Uuid,
    }

    fn env() -> Env {
        let fake = Arc::new(FakeStore::default());
        let client = Uuid::now_v7();
        fake.clients.lock().unwrap().insert(client, true);
        let auth = Authenticator::new(fake.clone(), keys());
        Env { fake, auth, client }
    }

    #[tokio::test]
    async fn non_uuid_client_id_is_refused_without_touching_the_store() {
        let e = env();
        for bad in [
            "",
            "bgn-mpm-prod",
            "0193b8a2",
            "not-a-uuid-at-all-not-a-uuid-at-all!",
            " ",
        ] {
            assert_eq!(
                e.auth.client_secret(bad, "x").await,
                Err(AuthError::Fail(AuthFail::KredensialTidakDikenal)),
                "{bad:?}"
            );
        }
        assert_eq!(
            e.fake.calls(),
            0,
            "no query for input that cannot be a client id"
        );
    }

    #[tokio::test]
    async fn client_secret_accepts_the_right_secret_and_refuses_the_rest() {
        let e = env();
        let cred = secret_cred(e.client, "s3cret");
        let cred_id = cred.credential_id;
        e.fake.kredensial.lock().unwrap().push(cred);
        let id = e.client.to_string();
        assert_eq!(
            e.auth.client_secret(&id, "s3cret").await,
            Ok((e.client, cred_id))
        );
        assert_eq!(
            e.auth.client_secret(&id, "wrong").await,
            Err(AuthError::Fail(AuthFail::SecretSalah))
        );
        assert_eq!(
            e.auth
                .client_secret(&Uuid::now_v7().to_string(), "s3cret")
                .await,
            Err(AuthError::Fail(AuthFail::KredensialTidakDikenal))
        );
    }

    #[tokio::test]
    async fn only_the_two_active_credentials_are_tried_and_either_works() {
        let e = env();
        {
            let mut k = e.fake.kredensial.lock().unwrap();
            k.push(secret_cred(e.client, "old"));
            k.push(secret_cred(e.client, "new"));
        }
        let id = e.client.to_string();
        let (old_id, new_id) = {
            let k = e.fake.kredensial.lock().unwrap();
            (k[0].credential_id, k[1].credential_id)
        };
        assert_eq!(
            e.auth.client_secret(&id, "old").await,
            Ok((e.client, old_id))
        );
        assert_eq!(
            e.auth.client_secret(&id, "new").await,
            Ok((e.client, new_id))
        );
    }

    #[tokio::test]
    async fn revoked_expired_and_inactive_are_distinguished_only_in_the_label() {
        let e = env();
        let id = e.client.to_string();
        let mut revoked = secret_cred(e.client, "s");
        revoked.aktif = false;
        e.fake.kredensial.lock().unwrap().push(revoked);
        assert_eq!(
            e.auth.client_secret(&id, "s").await,
            Err(AuthError::Fail(AuthFail::KredensialNonaktif))
        );

        let e = env();
        let mut expired = secret_cred(e.client, "s");
        expired.kedaluwarsa_at = Some(Utc::now() - chrono::Duration::seconds(5));
        e.fake.kredensial.lock().unwrap().push(expired);
        assert_eq!(
            e.auth.client_secret(&e.client.to_string(), "s").await,
            Err(AuthError::Fail(AuthFail::KredensialKedaluwarsa))
        );

        let e = env();
        e.fake
            .kredensial
            .lock()
            .unwrap()
            .push(secret_cred(e.client, "s"));
        e.fake.clients.lock().unwrap().insert(e.client, false);
        assert_eq!(
            e.auth.client_secret(&e.client.to_string(), "s").await,
            Err(AuthError::Fail(AuthFail::KredensialNonaktif)),
            "inactive client"
        );
    }

    #[tokio::test]
    async fn store_failure_fails_closed() {
        let e = env();
        e.fake
            .kredensial
            .lock()
            .unwrap()
            .push(secret_cred(e.client, "s"));
        *e.fake.fail.lock().unwrap() = true;
        assert_eq!(
            e.auth.client_secret(&e.client.to_string(), "s").await,
            Err(AuthError::Unavailable)
        );
        assert_eq!(
            e.auth.api_key("pr_x").await,
            Err(AuthError::Fail(AuthFail::ApiKeyTidakValid))
        );
    }

    #[tokio::test]
    async fn bearer_follows_client_and_credential_status() {
        let e = env();
        let cred = secret_cred(e.client, "s");
        let cred_id = cred.credential_id;
        e.fake.kredensial.lock().unwrap().push(cred);
        let (tok, _) = e
            .auth
            .keys()
            .issue(e.client, cred_id, jsonwebtoken::get_current_timestamp())
            .unwrap();
        assert_eq!(e.auth.bearer(&tok).await, Ok(e.client));
        // Revoking the issuing credential refuses the token already issued from it.
        e.fake.kredensial.lock().unwrap()[0].aktif = false;
        assert_eq!(
            e.auth.bearer(&tok).await,
            Err(AuthError::Fail(AuthFail::KredensialNonaktif))
        );
        e.fake.kredensial.lock().unwrap()[0].aktif = true;
        // A second active credential does not keep the revoked one's token alive.
        e.fake
            .kredensial
            .lock()
            .unwrap()
            .push(secret_cred(e.client, "lain"));
        e.fake.kredensial.lock().unwrap()[0].aktif = false;
        assert_eq!(
            e.auth.bearer(&tok).await,
            Err(AuthError::Fail(AuthFail::KredensialNonaktif)),
            "another active credential must not resurrect this token"
        );
        e.fake.kredensial.lock().unwrap()[0].aktif = true;
        // Expiry of the issuing credential does block the issued token (Shu, 09.18).
        e.fake.kredensial.lock().unwrap()[0].kedaluwarsa_at =
            Some(Utc::now() - chrono::Duration::seconds(5));
        assert_eq!(
            e.auth.bearer(&tok).await,
            Err(AuthError::Fail(AuthFail::KredensialKedaluwarsa))
        );
        e.fake.kredensial.lock().unwrap()[0].kedaluwarsa_at = None;
        e.fake.clients.lock().unwrap().insert(e.client, false);
        assert_eq!(
            e.auth.bearer(&tok).await,
            Err(AuthError::Fail(AuthFail::KredensialNonaktif))
        );
        assert_eq!(
            e.auth.bearer("garbage").await,
            Err(AuthError::Fail(AuthFail::TokenTidakValid))
        );
    }

    #[tokio::test]
    async fn bearer_for_an_unknown_client_is_an_invalid_token() {
        let e = env();
        let (tok, _) = e
            .auth
            .keys()
            .issue(
                Uuid::now_v7(),
                Uuid::now_v7(),
                jsonwebtoken::get_current_timestamp(),
            )
            .unwrap();
        assert_eq!(
            e.auth.bearer(&tok).await,
            Err(AuthError::Fail(AuthFail::TokenTidakValid))
        );
    }

    #[tokio::test]
    async fn bearer_with_a_foreign_or_api_key_credential_is_refused() {
        let e = env();
        e.fake
            .kredensial
            .lock()
            .unwrap()
            .push(secret_cred(e.client, "s"));
        let now = jsonwebtoken::get_current_timestamp();

        // `cid` of a credential that belongs to another client.
        let asing = secret_cred(Uuid::now_v7(), "s").credential_id;
        let (tok, _) = e.auth.keys().issue(e.client, asing, now).unwrap();
        assert_eq!(
            e.auth.bearer(&tok).await,
            Err(AuthError::Fail(AuthFail::TokenTidakValid)),
            "credential of another client"
        );

        // `cid` of an unknown credential.
        let (tok, _) = e.auth.keys().issue(e.client, Uuid::now_v7(), now).unwrap();
        assert_eq!(
            e.auth.bearer(&tok).await,
            Err(AuthError::Fail(AuthFail::TokenTidakValid)),
            "unknown credential"
        );

        // `cid` of an API key credential: tokens are only issued to client secrets.
        let key_id = Uuid::now_v7();
        let k = buat_api_key(key_id).unwrap();
        e.fake.kredensial.lock().unwrap().push(Kredensial {
            credential_id: key_id,
            client_id: e.client,
            jenis: Jenis::ApiKey,
            kunci_hash: k.kunci_hash.to_vec(),
            aktif: true,
            kedaluwarsa_at: None,
        });
        let (tok, _) = e.auth.keys().issue(e.client, key_id, now).unwrap();
        assert_eq!(
            e.auth.bearer(&tok).await,
            Err(AuthError::Fail(AuthFail::TokenTidakValid)),
            "api key credential cannot issue a bearer token"
        );
    }

    #[tokio::test]
    async fn api_key_checks_secret_before_status() {
        let e = env();
        let cred_id = Uuid::now_v7();
        let k = buat_api_key(cred_id).unwrap();
        e.fake.kredensial.lock().unwrap().push(Kredensial {
            credential_id: cred_id,
            client_id: e.client,
            jenis: Jenis::ApiKey,
            kunci_hash: k.kunci_hash.to_vec(),
            aktif: true,
            kedaluwarsa_at: None,
        });
        assert_eq!(e.auth.api_key(&k.kunci).await, Ok(e.client));

        // Wrong secret for an existing id: generic refusal, nothing about status.
        let other = buat_api_key(cred_id).unwrap();
        assert_eq!(
            e.auth.api_key(&other.kunci).await,
            Err(AuthError::Fail(AuthFail::ApiKeyTidakValid))
        );
        e.fake.kredensial.lock().unwrap()[0].aktif = false;
        assert_eq!(
            e.auth.api_key(&other.kunci).await,
            Err(AuthError::Fail(AuthFail::ApiKeyTidakValid))
        );
        assert_eq!(
            e.auth.api_key(&k.kunci).await,
            Err(AuthError::Fail(AuthFail::KredensialNonaktif))
        );
        e.fake.kredensial.lock().unwrap()[0].aktif = true;
        e.fake.kredensial.lock().unwrap()[0].kedaluwarsa_at =
            Some(Utc::now() - chrono::Duration::seconds(1));
        assert_eq!(
            e.auth.api_key(&k.kunci).await,
            Err(AuthError::Fail(AuthFail::KredensialKedaluwarsa))
        );
    }

    #[tokio::test]
    async fn malformed_api_key_never_reaches_the_store() {
        let e = env();
        for bad in ["", "pr_", "x", "pr_0193b8a27c4e7d108a3f5e6b7c8d9e0f_short"] {
            assert_eq!(
                e.auth.api_key(bad).await,
                Err(AuthError::Fail(AuthFail::ApiKeyTidakValid)),
                "{bad:?}"
            );
        }
        assert_eq!(e.fake.calls(), 0);
    }

    #[tokio::test]
    async fn api_key_of_the_wrong_kind_is_invalid() {
        let e = env();
        let cred_id = Uuid::now_v7();
        let k = buat_api_key(cred_id).unwrap();
        let mut c = secret_cred(e.client, "s");
        c.credential_id = cred_id; // client-secret credential addressed as an API key
        e.fake.kredensial.lock().unwrap().push(c);
        assert_eq!(
            e.auth.api_key(&k.kunci).await,
            Err(AuthError::Fail(AuthFail::ApiKeyTidakValid))
        );
    }

    #[test]
    fn alasan_values_are_in_the_closed_set() {
        const SET: [&str; 10] = [
            "kredensial_tidak_dikenal",
            "secret_salah",
            "kredensial_nonaktif",
            "kredensial_kedaluwarsa",
            "grant_tidak_didukung",
            "permintaan_tidak_valid",
            "dibatasi",
            "token_tidak_valid",
            "token_kedaluwarsa",
            "api_key_tidak_valid",
        ];
        for f in [
            AuthFail::KredensialTidakDikenal,
            AuthFail::SecretSalah,
            AuthFail::KredensialNonaktif,
            AuthFail::KredensialKedaluwarsa,
            AuthFail::TokenTidakValid,
            AuthFail::TokenKedaluwarsa,
            AuthFail::ApiKeyTidakValid,
        ] {
            assert!(SET.contains(&f.alasan()), "{}", f.alasan());
        }
    }
}
