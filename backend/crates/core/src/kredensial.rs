//! Client credential formats shared by the downstream API (verification) and
//! the admin API (issuing, SHAA-196). One generator and one parser, so the two
//! sides cannot drift (Shu decision on SHAA-198, ADR-0001 rev 15 section 5a.5).
//!
//! API key wire format: `pr_<credential_id>_<secret>`
//!
//! - `credential_id`: 32 lowercase hex characters, a UUID without hyphens.
//! - `secret`: 256 random bits, base64url without padding (43 characters).
//! - `client_credential.kunci_hash` stores SHA-256 of the *secret part only*.
//!   The prefix and the id are not hashed: the id is only a lookup handle.
//!
//! Nothing here logs or formats a secret into an error.

use aws_lc_rs::{constant_time, digest, rand};
use uuid::Uuid;

/// Fixed prefix of every API key.
pub const API_KEY_PREFIX: &str = "pr_";
/// Random bytes in a secret (256 bits, ADR-0001 section 5a.5).
pub const SECRET_BYTES: usize = 32;
/// Length of the base64url (no padding) form of [`SECRET_BYTES`].
pub const SECRET_CHARS: usize = 43;
/// Length of SHA-256 output, the size of `kunci_hash` for API keys.
pub const HASH_BYTES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KredensialError {
    /// The system random source failed. Never fall back to a weaker source.
    #[error("random source unavailable")]
    Random,
}

/// A freshly generated API key. `kunci` is shown to the operator once;
/// `kunci_hash` is what the database stores.
pub struct ApiKeyBaru {
    pub kunci: String,
    pub kunci_hash: [u8; HASH_BYTES],
}

impl std::fmt::Debug for ApiKeyBaru {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiKeyBaru").finish_non_exhaustive()
    }
}

/// The parts of a well-formed API key. `secret` borrows from the input.
#[derive(Clone, Copy)]
pub struct ApiKeyTerurai<'a> {
    pub credential_id: Uuid,
    pub secret: &'a str,
}

impl std::fmt::Debug for ApiKeyTerurai<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiKeyTerurai")
            .field("credential_id", &self.credential_id)
            .finish_non_exhaustive()
    }
}

/// 256 random bits as base64url without padding. Used for API key secrets and
/// for client secrets (ADR-0003 section 3.6a).
pub fn secret_acak() -> Result<String, KredensialError> {
    let mut buf = [0u8; SECRET_BYTES];
    rand::fill(&mut buf).map_err(|_| KredensialError::Random)?;
    Ok(base64url_encode(&buf))
}

/// Build a new API key for `credential_id`.
pub fn buat_api_key(credential_id: Uuid) -> Result<ApiKeyBaru, KredensialError> {
    let secret = secret_acak()?;
    let kunci_hash = hash_secret(&secret);
    let kunci = format!("{API_KEY_PREFIX}{}_{secret}", credential_id.simple());
    Ok(ApiKeyBaru { kunci, kunci_hash })
}

/// Split an API key. `None` for anything that is not exactly the canonical
/// format; the caller answers 401 without touching the database.
pub fn urai_api_key(kunci: &str) -> Option<ApiKeyTerurai<'_>> {
    let rest = kunci.strip_prefix(API_KEY_PREFIX)?;
    let (id, secret) = rest.split_once('_')?;
    if id.len() != 32 || !id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    if secret.len() != SECRET_CHARS {
        return None;
    }
    // Canonical form only: decode must give 32 bytes and encode back to the
    // same text, which rejects stray trailing bits.
    let bytes = base64url_decode(secret)?;
    if bytes.len() != SECRET_BYTES || base64url_encode(&bytes) != secret {
        return None;
    }
    Some(ApiKeyTerurai {
        credential_id: Uuid::parse_str(id).ok()?,
        secret,
    })
}

/// SHA-256 of the secret part.
pub fn hash_secret(secret: &str) -> [u8; HASH_BYTES] {
    let d = digest::digest(&digest::SHA256, secret.as_bytes());
    let mut out = [0u8; HASH_BYTES];
    out.copy_from_slice(d.as_ref());
    out
}

/// Constant-time comparison of `secret` against a stored `kunci_hash`.
/// A stored value of the wrong length never matches.
pub fn cocok_secret(secret: &str, kunci_hash: &[u8]) -> bool {
    constant_time::verify_slices_are_equal(&hash_secret(secret), kunci_hash).is_ok()
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// base64url (RFC 4648 section 5) without padding.
pub fn base64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// Decode base64url without padding. `None` for any other character or an
/// impossible length.
pub fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if bytes.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n: u32 = 0;
        for (i, &c) in chunk.iter().enumerate() {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'-' => 62,
                b'_' => 63,
                _ => return None,
            };
            n |= u32::from(v) << (18 - 6 * i);
        }
        let produced = chunk.len() - 1;
        for i in 0..produced {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_matches_rfc4648_vectors() {
        // RFC 4648 section 10, adapted to the URL alphabet without padding.
        for (plain, enc) in [
            ("", ""),
            ("f", "Zg"),
            ("fo", "Zm8"),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg"),
            ("fooba", "Zm9vYmE"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64url_encode(plain.as_bytes()), enc);
            assert_eq!(base64url_decode(enc).unwrap(), plain.as_bytes());
        }
        // The two URL-specific characters.
        assert_eq!(base64url_encode(&[0xfb, 0xff]), "-_8");
        assert_eq!(base64url_decode("-_8").unwrap(), [0xfb, 0xff]);
    }

    #[test]
    fn base64url_rejects_padding_standard_alphabet_and_bad_length() {
        assert!(base64url_decode("Zg==").is_none());
        assert!(base64url_decode("a+b/").is_none());
        assert!(base64url_decode("A").is_none());
        assert!(base64url_decode("Zm9v ").is_none());
    }

    #[test]
    fn generated_key_round_trips() {
        let id = Uuid::parse_str("0193b8a2-7c4e-7d10-8a3f-5e6b7c8d9e0f").unwrap();
        let k = buat_api_key(id).unwrap();
        assert!(k.kunci.starts_with("pr_0193b8a27c4e7d108a3f5e6b7c8d9e0f_"));
        assert_eq!(k.kunci.len(), 3 + 32 + 1 + SECRET_CHARS);
        let p = urai_api_key(&k.kunci).expect("well formed");
        assert_eq!(p.credential_id, id);
        assert!(cocok_secret(p.secret, &k.kunci_hash));
    }

    #[test]
    fn two_keys_differ() {
        let id = Uuid::now_v7();
        assert_ne!(
            buat_api_key(id).unwrap().kunci,
            buat_api_key(id).unwrap().kunci
        );
    }

    #[test]
    fn wrong_secret_or_hash_length_never_matches() {
        let k = buat_api_key(Uuid::now_v7()).unwrap();
        let other = secret_acak().unwrap();
        assert!(!cocok_secret(&other, &k.kunci_hash));
        assert!(!cocok_secret(&other, &[]));
        let p = urai_api_key(&k.kunci).unwrap();
        assert!(!cocok_secret(p.secret, &k.kunci_hash[..31]));
    }

    #[test]
    fn parser_rejects_every_malformed_shape() {
        let id = "0193b8a27c4e7d108a3f5e6b7c8d9e0f";
        let good = secret_acak().unwrap();
        assert!(urai_api_key(&format!("pr_{id}_{good}")).is_some());
        let cases = [
            String::new(),
            "pr_".into(),
            format!("PR_{id}_{good}"),
            format!("pr{id}_{good}"),
            format!("pr_{id}{good}"),
            format!("pr_{}_{good}", &id[..31]),
            format!("pr_{}0_{good}", id),
            format!("pr_{}_{good}", id.to_uppercase()),
            format!("pr_{}-{}_{good}", &id[..8], &id[9..]),
            format!("pr_{id}_{}", &good[..42]),
            format!("pr_{id}_{good}A"),
            format!("pr_{id}_{}+", &good[..42]),
            format!("pr_{id}_{good}_extra"),
            format!(" pr_{id}_{good}"),
            format!("pr_{id}_{good}\n"),
        ];
        for c in cases {
            assert!(urai_api_key(&c).is_none(), "accepted: {c:?}");
        }
    }

    #[test]
    fn parser_rejects_non_canonical_trailing_bits() {
        // 43 chars carry 258 bits; the last char's two low bits must be zero.
        let id = "0193b8a27c4e7d108a3f5e6b7c8d9e0f";
        let canonical = base64url_encode(&[0u8; SECRET_BYTES]);
        assert!(canonical.ends_with('A'));
        let mut tampered = canonical[..42].to_string();
        tampered.push('B');
        assert!(urai_api_key(&format!("pr_{id}_{canonical}")).is_some());
        assert!(urai_api_key(&format!("pr_{id}_{tampered}")).is_none());
    }

    #[test]
    fn hash_is_sha256_of_the_secret_part_only() {
        // SHA-256("abc")
        let h = hash_secret("abc");
        let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn debug_never_prints_secrets() {
        let k = buat_api_key(Uuid::now_v7()).unwrap();
        let p = urai_api_key(&k.kunci).unwrap();
        let shown = format!("{k:?} {p:?}");
        assert!(!shown.contains(p.secret));
    }
}
