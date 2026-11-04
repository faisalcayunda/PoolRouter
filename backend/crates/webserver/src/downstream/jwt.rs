//! Client-credentials access tokens (ADR-0001 rev 15 section 5a.5, SHAA-198 U-14).
//!
//! - Signing key: PEM PKCS#8. Ed25519 gives EdDSA (default), P-256 gives ES256;
//!   any other key type refuses to start.
//! - `kid` is the JWK thumbprint (RFC 7638, SHA-256, base64url) of the public key.
//! - At most two verification keys: the signing key and an optional second
//!   public key (rotation: install for verification first, then sign with it).
//! - The algorithm comes from the key found by `kid`, never from the header; a
//!   header that disagrees, an unknown `kid` or a missing `kid` is "invalid".
//! - Claims `iss`, `aud`, `sub`, `exp`, `iat`, `nbf`, `jti` are all required;
//!   leeway is 60 s. `jti` is for log correlation only (no denylist in F0).
//!
//! Error values carry no library message and no claim content: the caller maps
//! them to the closed `alasan` label set only.

use aws_lc_rs::digest;
use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, Ed25519KeyPair, KeyPair,
};
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use poolrouter_core::kredensial::{base64url_decode, base64url_encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const AUDIENCE: &str = "poolrouter-downstream-api";
/// Token lifetime: 15 minutes, no refresh token (RFC 6749 section 4.4.3).
pub const TTL_SECS: u64 = 15 * 60;
const LEEWAY_SECS: u64 = 60;

/// Why a key file was refused. Never includes key material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("not a PEM block of the expected kind")]
    Pem,
    #[error("unsupported key type (only Ed25519 and P-256 are allowed)")]
    UnsupportedKey,
    #[error("at most two verification keys are allowed and they must differ")]
    Duplicate,
}

/// Verification outcome for the metric label `alasan` (MT-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenError {
    /// Signature, `kid`, `alg`, `iss`, `aud`, `nbf`, or a missing claim.
    Invalid,
    Expired,
}

impl TokenError {
    /// Closed label value for `poolrouter_token_gagal_total{alasan}`.
    pub fn alasan(self) -> &'static str {
        match self {
            Self::Invalid => "token_tidak_valid",
            Self::Expired => "token_kedaluwarsa",
        }
    }
}

/// Signing failed. A server fault, never a bad credential: the token endpoint
/// answers 503, not 401.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("token signing failed")]
pub struct SignError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    pub iss: String,
    pub aud: String,
    pub sub: Uuid,
    /// Credential that issued the token. Revoking it revokes this token too.
    pub cid: Uuid,
    pub exp: u64,
    pub iat: u64,
    pub nbf: u64,
    pub jti: Uuid,
}

struct VerifyKey {
    kid: String,
    alg: Algorithm,
    key: DecodingKey,
}

struct SigningKey {
    kid: String,
    alg: Algorithm,
    key: EncodingKey,
}

pub struct JwtKeys {
    issuer: String,
    signing: SigningKey,
    verify: Vec<VerifyKey>,
}

impl std::fmt::Debug for JwtKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwtKeys")
            .field("signing_kid", &self.signing.kid)
            .field(
                "verify_kids",
                &self.verify.iter().map(|k| &k.kid).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

/// Public half of a key in JWK terms, enough for `kid` and verification.
enum PublicJwk {
    Ed25519 { x: [u8; 32] },
    P256 { x: [u8; 32], y: [u8; 32] },
}

impl PublicJwk {
    /// RFC 7638 thumbprint: members in lexicographic order, no whitespace.
    fn thumbprint(&self) -> String {
        let json = match self {
            Self::Ed25519 { x } => format!(
                r#"{{"crv":"Ed25519","kty":"OKP","x":"{}"}}"#,
                base64url_encode(x)
            ),
            Self::P256 { x, y } => format!(
                r#"{{"crv":"P-256","kty":"EC","x":"{}","y":"{}"}}"#,
                base64url_encode(x),
                base64url_encode(y)
            ),
        };
        base64url_encode(digest::digest(&digest::SHA256, json.as_bytes()).as_ref())
    }

    fn algorithm(&self) -> Algorithm {
        match self {
            Self::Ed25519 { .. } => Algorithm::EdDSA,
            Self::P256 { .. } => Algorithm::ES256,
        }
    }

    fn decoding_key(&self) -> Result<DecodingKey, KeyError> {
        match self {
            Self::Ed25519 { x } => DecodingKey::from_ed_components(&base64url_encode(x)),
            Self::P256 { x, y } => {
                DecodingKey::from_ec_components(&base64url_encode(x), &base64url_encode(y))
            }
        }
        .map_err(|_| KeyError::UnsupportedKey)
    }
}

/// DER prefix of a SubjectPublicKeyInfo holding an Ed25519 key (followed by 32 bytes).
const SPKI_ED25519: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];
/// DER prefix of a SubjectPublicKeyInfo holding a P-256 key (followed by 65 bytes, 0x04 || x || y).
const SPKI_P256: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// Body of the first PEM block labelled `label`, decoded from base64.
fn pem_der(pem: &str, label: &str) -> Result<Vec<u8>, KeyError> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let start = pem.find(&begin).ok_or(KeyError::Pem)? + begin.len();
    let stop = pem[start..].find(&end).ok_or(KeyError::Pem)? + start;
    // Standard alphabet to URL alphabet, so one decoder serves both.
    let body: String = pem[start..stop]
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '=')
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            other => other,
        })
        .collect();
    base64url_decode(&body).ok_or(KeyError::Pem)
}

fn public_from_spki(der: &[u8]) -> Result<PublicJwk, KeyError> {
    if let Some(rest) = der.strip_prefix(&SPKI_ED25519[..])
        && let Ok(x) = <[u8; 32]>::try_from(rest)
    {
        return Ok(PublicJwk::Ed25519 { x });
    }
    if let Some(rest) = der.strip_prefix(&SPKI_P256[..])
        && rest.len() == 65
        && rest[0] == 0x04
    {
        let x = <[u8; 32]>::try_from(&rest[1..33]).map_err(|_| KeyError::UnsupportedKey)?;
        let y = <[u8; 32]>::try_from(&rest[33..65]).map_err(|_| KeyError::UnsupportedKey)?;
        return Ok(PublicJwk::P256 { x, y });
    }
    Err(KeyError::UnsupportedKey)
}

impl JwtKeys {
    /// Build from the signing key PEM (PKCS#8) and an optional second public
    /// key PEM (SPKI). `issuer` is `${POOLROUTER_TOKEN_ISSUER}`.
    pub fn from_pem(
        issuer: &str,
        signing_pem: &str,
        verify_pem: Option<&str>,
    ) -> Result<Self, KeyError> {
        let der = pem_der(signing_pem, "PRIVATE KEY")?;
        let (public, enc) = if let Ok(kp) = Ed25519KeyPair::from_pkcs8(&der) {
            let x = <[u8; 32]>::try_from(kp.public_key().as_ref())
                .map_err(|_| KeyError::UnsupportedKey)?;
            (PublicJwk::Ed25519 { x }, EncodingKey::from_ed_der(&der))
        } else if let Ok(kp) = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der) {
            let p = kp.public_key().as_ref();
            if p.len() != 65 || p[0] != 0x04 {
                return Err(KeyError::UnsupportedKey);
            }
            let x = <[u8; 32]>::try_from(&p[1..33]).map_err(|_| KeyError::UnsupportedKey)?;
            let y = <[u8; 32]>::try_from(&p[33..65]).map_err(|_| KeyError::UnsupportedKey)?;
            (PublicJwk::P256 { x, y }, EncodingKey::from_ec_der(&der))
        } else {
            return Err(KeyError::UnsupportedKey);
        };

        let signing = SigningKey {
            kid: public.thumbprint(),
            alg: public.algorithm(),
            key: enc,
        };
        let mut verify = vec![VerifyKey {
            kid: signing.kid.clone(),
            alg: signing.alg,
            key: public.decoding_key()?,
        }];
        if let Some(pem) = verify_pem {
            let other = public_from_spki(&pem_der(pem, "PUBLIC KEY")?)?;
            let kid = other.thumbprint();
            if verify.iter().any(|k| k.kid == kid) {
                return Err(KeyError::Duplicate);
            }
            verify.push(VerifyKey {
                kid,
                alg: other.algorithm(),
                key: other.decoding_key()?,
            });
        }
        Ok(Self {
            issuer: issuer.to_owned(),
            signing,
            verify,
        })
    }

    pub fn signing_kid(&self) -> &str {
        &self.signing.kid
    }

    /// Issue a token for `client_id` under credential `credential_id` at unix
    /// time `now`. Returns the compact token and `expires_in` seconds.
    pub fn issue(
        &self,
        client_id: Uuid,
        credential_id: Uuid,
        now: u64,
    ) -> Result<(String, u64), SignError> {
        let mut header = Header::new(self.signing.alg);
        header.kid = Some(self.signing.kid.clone());
        let claims = Claims {
            iss: self.issuer.clone(),
            aud: AUDIENCE.to_owned(),
            sub: client_id,
            cid: credential_id,
            exp: now + TTL_SECS,
            iat: now,
            nbf: now,
            jti: Uuid::now_v7(),
        };
        let token =
            jsonwebtoken::encode(&header, &claims, &self.signing.key).map_err(|_| SignError)?;
        Ok((token, TTL_SECS))
    }

    /// Verify signature and claims. The key (and so the algorithm) is chosen by
    /// `kid`; the header `alg` must match it.
    pub fn verify(&self, token: &str) -> Result<Claims, TokenError> {
        let header = jsonwebtoken::decode_header(token).map_err(|_| TokenError::Invalid)?;
        let kid = header.kid.as_deref().ok_or(TokenError::Invalid)?;
        let key = self
            .verify
            .iter()
            .find(|k| k.kid == kid)
            .ok_or(TokenError::Invalid)?;
        if header.alg != key.alg {
            return Err(TokenError::Invalid);
        }
        let mut v = Validation::new(key.alg);
        v.leeway = LEEWAY_SECS;
        v.validate_nbf = true;
        v.set_audience(&[AUDIENCE]);
        v.set_issuer(&[self.issuer.as_str()]);
        v.set_required_spec_claims(&["exp", "nbf", "iss", "aud", "sub"]);
        match jsonwebtoken::decode::<Claims>(token, &key.key, &v) {
            Ok(data) => Ok(data.claims),
            Err(e) if matches!(e.kind(), ErrorKind::ExpiredSignature) => Err(TokenError::Expired),
            Err(_) => Err(TokenError::Invalid),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::rand::SystemRandom;

    const ISS: &str = "https://poolrouter.example.test";

    fn pem(label: &str, der: &[u8]) -> String {
        // Standard base64 from the URL alphabet, 64-column lines.
        let b = base64url_encode(der).replace('-', "+").replace('_', "/");
        let pad = (4 - b.len() % 4) % 4;
        let b = format!("{b}{}", "=".repeat(pad));
        let lines: Vec<&str> = b
            .as_bytes()
            .chunks(64)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect();
        format!(
            "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
            lines.join("\n")
        )
    }

    fn ed_pair() -> (String, String) {
        let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let kp = Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap();
        let mut spki = SPKI_ED25519.to_vec();
        spki.extend_from_slice(kp.public_key().as_ref());
        (pem("PRIVATE KEY", doc.as_ref()), pem("PUBLIC KEY", &spki))
    }

    fn ec_pair() -> (String, String) {
        let doc =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
                .unwrap();
        let kp = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, doc.as_ref()).unwrap();
        let mut spki = SPKI_P256.to_vec();
        spki.extend_from_slice(kp.public_key().as_ref());
        (pem("PRIVATE KEY", doc.as_ref()), pem("PUBLIC KEY", &spki))
    }

    fn now() -> u64 {
        jsonwebtoken::get_current_timestamp()
    }

    #[test]
    fn thumbprint_matches_rfc8037_appendix_a3() {
        let x = base64url_decode("11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo").unwrap();
        let jwk = PublicJwk::Ed25519 {
            x: <[u8; 32]>::try_from(x.as_slice()).unwrap(),
        };
        assert_eq!(
            jwk.thumbprint(),
            "kPrK_qmxVWaYVA9wwBF6Iuo3vVzz7TxHCTwXBygrS4k"
        );
    }

    #[test]
    fn eddsa_round_trip_with_required_claims() {
        let (sk, _) = ed_pair();
        let keys = JwtKeys::from_pem(ISS, &sk, None).unwrap();
        let client = Uuid::now_v7();
        let (tok, ttl) = keys.issue(client, Uuid::now_v7(), now()).unwrap();
        assert_eq!(ttl, 900);
        let h = jsonwebtoken::decode_header(&tok).unwrap();
        assert_eq!(h.alg, Algorithm::EdDSA);
        assert_eq!(h.kid.as_deref(), Some(keys.signing_kid()));
        let c = keys.verify(&tok).unwrap();
        assert_eq!(c.sub, client);
        assert_eq!(c.iss, ISS);
        assert_eq!(c.aud, AUDIENCE);
        assert_eq!(c.exp - c.iat, 900);
        assert_eq!(c.nbf, c.iat);
    }

    #[test]
    fn es256_round_trip() {
        let (sk, _) = ec_pair();
        let keys = JwtKeys::from_pem(ISS, &sk, None).unwrap();
        let (tok, _) = keys.issue(Uuid::now_v7(), Uuid::now_v7(), now()).unwrap();
        assert_eq!(
            jsonwebtoken::decode_header(&tok).unwrap().alg,
            Algorithm::ES256
        );
        assert!(keys.verify(&tok).is_ok());
    }

    #[test]
    fn expired_token_is_distinguished_only_beyond_leeway() {
        let (sk, _) = ed_pair();
        let keys = JwtKeys::from_pem(ISS, &sk, None).unwrap();
        // Issued 15 min + 30 s ago: inside the 60 s leeway, still valid.
        let (tok, _) = keys
            .issue(Uuid::now_v7(), Uuid::now_v7(), now() - 900 - 30)
            .unwrap();
        assert!(keys.verify(&tok).is_ok());
        // Issued 15 min + 120 s ago: expired.
        let (tok, _) = keys
            .issue(Uuid::now_v7(), Uuid::now_v7(), now() - 900 - 120)
            .unwrap();
        assert_eq!(keys.verify(&tok), Err(TokenError::Expired));
    }

    #[test]
    fn not_yet_valid_token_is_invalid_beyond_leeway() {
        let (sk, _) = ed_pair();
        let keys = JwtKeys::from_pem(ISS, &sk, None).unwrap();
        let (tok, _) = keys
            .issue(Uuid::now_v7(), Uuid::now_v7(), now() + 600)
            .unwrap();
        assert_eq!(keys.verify(&tok), Err(TokenError::Invalid));
    }

    #[test]
    fn wrong_issuer_and_foreign_key_are_invalid() {
        let (sk, _) = ed_pair();
        let a = JwtKeys::from_pem(ISS, &sk, None).unwrap();
        let (tok, _) = a.issue(Uuid::now_v7(), Uuid::now_v7(), now()).unwrap();
        let other_iss = JwtKeys::from_pem("https://other.example.test", &sk, None).unwrap();
        assert_eq!(other_iss.verify(&tok), Err(TokenError::Invalid));
        let (sk2, _) = ed_pair();
        let b = JwtKeys::from_pem(ISS, &sk2, None).unwrap();
        assert_eq!(b.verify(&tok), Err(TokenError::Invalid), "unknown kid");
    }

    #[test]
    fn tampered_payload_and_garbage_are_invalid() {
        let (sk, _) = ed_pair();
        let keys = JwtKeys::from_pem(ISS, &sk, None).unwrap();
        let (tok, _) = keys.issue(Uuid::now_v7(), Uuid::now_v7(), now()).unwrap();
        let parts: Vec<&str> = tok.split('.').collect();
        let forged = base64url_encode(
            br#"{"iss":"x","aud":"poolrouter-downstream-api","sub":"00000000-0000-0000-0000-000000000000","exp":9999999999,"iat":1,"nbf":1,"jti":"00000000-0000-0000-0000-000000000000"}"#,
        );
        let t = format!("{}.{}.{}", parts[0], forged, parts[2]);
        assert_eq!(keys.verify(&t), Err(TokenError::Invalid));
        for g in [
            "",
            "a.b.c",
            "not-a-jwt",
            "....",
            &format!("{}.{}", parts[0], parts[1]),
        ] {
            assert_eq!(keys.verify(g), Err(TokenError::Invalid), "{g:?}");
        }
    }

    #[test]
    fn header_alg_none_hs256_and_missing_kid_are_invalid() {
        let (sk, _) = ed_pair();
        let keys = JwtKeys::from_pem(ISS, &sk, None).unwrap();
        let (tok, _) = keys.issue(Uuid::now_v7(), Uuid::now_v7(), now()).unwrap();
        let parts: Vec<&str> = tok.split('.').collect();
        let kid = keys.signing_kid();
        for hdr in [
            format!(r#"{{"alg":"none","typ":"JWT","kid":"{kid}"}}"#),
            format!(r#"{{"alg":"HS256","typ":"JWT","kid":"{kid}"}}"#),
            format!(r#"{{"alg":"ES256","typ":"JWT","kid":"{kid}"}}"#),
            r#"{"alg":"EdDSA","typ":"JWT"}"#.to_owned(),
        ] {
            let t = format!(
                "{}.{}.{}",
                base64url_encode(hdr.as_bytes()),
                parts[1],
                parts[2]
            );
            assert_eq!(keys.verify(&t), Err(TokenError::Invalid), "{hdr}");
        }
    }

    #[test]
    fn rotation_accepts_tokens_of_the_second_verification_key() {
        let (old_sk, _) = ed_pair();
        let (new_sk, new_pub) = ec_pair();
        // Phase 1: still signing with the old key, new public key installed.
        let phase1 = JwtKeys::from_pem(ISS, &old_sk, Some(&new_pub)).unwrap();
        // Phase 2: signing with the new key (as another replica would).
        let phase2 = JwtKeys::from_pem(ISS, &new_sk, None).unwrap();
        let (tok, _) = phase2.issue(Uuid::now_v7(), Uuid::now_v7(), now()).unwrap();
        assert!(
            phase1.verify(&tok).is_ok(),
            "installed for verification first"
        );
        assert_ne!(phase1.signing_kid(), phase2.signing_kid());
    }

    #[test]
    fn key_files_are_validated() {
        let (sk, pk) = ed_pair();
        // A public key is not a signing key.
        assert_eq!(
            JwtKeys::from_pem(ISS, &pk, None).unwrap_err(),
            KeyError::Pem
        );
        // Same key twice.
        assert_eq!(
            JwtKeys::from_pem(ISS, &sk, Some(&pk)).unwrap_err(),
            KeyError::Duplicate
        );
        // Unsupported key type: garbage PKCS#8 body.
        let bogus = pem("PRIVATE KEY", &[0x30, 0x03, 0x02, 0x01, 0x00]);
        assert_eq!(
            JwtKeys::from_pem(ISS, &bogus, None).unwrap_err(),
            KeyError::UnsupportedKey
        );
        assert_eq!(JwtKeys::from_pem(ISS, "", None).unwrap_err(), KeyError::Pem);
        // Verification key that is not Ed25519/P-256 SPKI.
        let bad_pub = pem("PUBLIC KEY", &[1, 2, 3]);
        assert_eq!(
            JwtKeys::from_pem(ISS, &sk, Some(&bad_pub)).unwrap_err(),
            KeyError::UnsupportedKey
        );
    }

    #[test]
    fn debug_and_errors_never_show_key_material() {
        let (sk, _) = ed_pair();
        let keys = JwtKeys::from_pem(ISS, &sk, None).unwrap();
        let shown = format!("{keys:?}");
        assert!(shown.contains(keys.signing_kid()));
        let body: String = sk.lines().filter(|l| !l.starts_with("-----")).collect();
        assert!(!shown.contains(&body));
    }

    /// Sign arbitrary claims with the key's own signing key and `kid`.
    fn sign_raw(keys: &JwtKeys, claims: &serde_json::Value) -> String {
        let mut h = Header::new(keys.signing.alg);
        h.kid = Some(keys.signing.kid.clone());
        jsonwebtoken::encode(&h, claims, &keys.signing.key).unwrap()
    }

    #[test]
    fn token_missing_any_required_claim_is_rejected() {
        let (sk, _) = ed_pair();
        let keys = JwtKeys::from_pem(ISS, &sk, None).unwrap();
        let t = now();
        let full = serde_json::json!({
            "iss": ISS, "aud": AUDIENCE, "sub": Uuid::now_v7(), "cid": Uuid::now_v7(),
            "exp": t + 900, "iat": t, "nbf": t, "jti": Uuid::now_v7(),
        });
        assert!(
            keys.verify(&sign_raw(&keys, &full)).is_ok(),
            "control: complete claims pass"
        );
        for claim in ["iss", "aud", "sub", "cid", "exp", "iat", "nbf", "jti"] {
            let mut c = full.clone();
            c.as_object_mut().unwrap().remove(claim);
            assert_eq!(
                keys.verify(&sign_raw(&keys, &c)),
                Err(TokenError::Invalid),
                "token without `{claim}` must be refused"
            );
        }
    }

    #[test]
    fn pkcs8_v1_ed25519_from_openssl_is_accepted() {
        // `openssl genpkey -algorithm ed25519` writes PKCS#8 v1: fixed 16-byte
        // prefix followed by the 32-byte seed. Built here from a random seed so
        // that no key file is committed.
        let mut seed = [0u8; 32];
        aws_lc_rs::rand::fill(&mut seed).unwrap();
        let mut der = vec![
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        der.extend_from_slice(&seed);
        let keys = JwtKeys::from_pem(ISS, &pem("PRIVATE KEY", &der), None).unwrap();
        let (tok, _) = keys.issue(Uuid::now_v7(), Uuid::now_v7(), now()).unwrap();
        assert!(keys.verify(&tok).is_ok());
    }

    #[test]
    fn alasan_labels_are_from_the_closed_set() {
        assert_eq!(TokenError::Invalid.alasan(), "token_tidak_valid");
        assert_eq!(TokenError::Expired.alasan(), "token_kedaluwarsa");
    }
}
