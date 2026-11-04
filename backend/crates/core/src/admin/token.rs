//! The opaque session token and its stored form (ADR-0003 rev 5 section 3.6,
//! option A: "sesi PoolRouter opaque di cookie `HttpOnly`").
//!
//! The token is 32 bytes from the OS CSPRNG, sent to the browser as base64url
//! without padding. The database stores **only** its SHA-256 —
//! `admin_session.token_hash`. So a dump of `admin_session` is not a set of
//! usable cookies, and a token found in a log cannot be replayed.
//!
//! Why SHA-256 and not argon2id here: the token is 256 bits of uniform random
//! data, so there is nothing to brute-force and no benefit to a slow hash; the
//! expensive-hash argument applies to low-entropy secrets like passwords. What
//! the hash must guarantee is that the stored value cannot be turned back into
//! the cookie, which a fast preimage-resistant hash already does.
//!
//! The token's [`Debug`] prints a fixed placeholder. A `{:?}` in a log line or a
//! test failure is the most common way a credential escapes, so the type makes
//! it impossible rather than relying on reviewers.

use std::fmt;

use base64::Engine as _;
use base64::alphabet;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
/// The one engine this module uses: URL-safe alphabet, no padding, and
/// **strict** about trailing bits.
///
/// The stock `URL_SAFE_NO_PAD` tolerates non-zero bits in the unused space of
/// the final character (WHATWG "forgiving" base64), which means several
/// different 43-character strings decode to the same 32 bytes. For a session
/// token that is not acceptable: each spelling would hash differently, so one
/// session could be presented under several cookie values, and the token would
/// stop being a single opaque identifier. Making the decoder strict is what
/// lets [`TokenSesi::dari_b64`] treat the text as canonical by construction.
const URL_SAFE_KETAT: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(false)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone)
        .with_encode_padding(false),
);
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Token length in bytes. 256 bits: far beyond guessing, and the same width as
/// the SHA-256 that stores it.
pub const PANJANG_TOKEN: usize = 32;

/// Length of the base64url form sent in the cookie (43 characters: 32 bytes
/// without padding).
pub const PANJANG_TOKEN_B64: usize = 43;

/// A live session token. Dropped zeroed.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct TokenSesi {
    /// Kept as the base64url text rather than the raw bytes, because that is
    /// the exact thing in the cookie and there is then only one representation
    /// to get wrong.
    nilai: String,
}

impl TokenSesi {
    /// Wrap a token received from a client. Rejects anything that is not
    /// exactly 43 base64url characters: a wrong-length cookie is a 401, not a
    /// case for the hash function to handle.
    pub fn dari_b64(nilai: impl Into<String>) -> Result<Self, TokenTidakValid> {
        let nilai = nilai.into();
        if nilai.len() != PANJANG_TOKEN_B64 {
            return Err(TokenTidakValid::Panjang { dapat: nilai.len() });
        }
        if !nilai
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(TokenTidakValid::Karakter);
        }
        // The strict engine rejects non-zero trailing bits, so a string that
        // decodes is canonical: 32 bytes have exactly one 43-character
        // spelling, which is what makes the hash a single identifier.
        let mentah = URL_SAFE_KETAT
            .decode(&nilai)
            .map_err(|_| TokenTidakValid::Karakter)?;
        if mentah.len() != PANJANG_TOKEN {
            return Err(TokenTidakValid::Karakter);
        }
        Ok(Self { nilai })
    }

    /// Mint a fresh token from the OS CSPRNG.
    pub fn baru() -> Self {
        let mut bytes = [0u8; PANJANG_TOKEN];
        // getrandom is infallible in practice on the platforms F0 targets; if
        // it ever is not, panicking is correct — issuing a weak session is
        // worse than refusing to issue one.
        getrandom::fill(&mut bytes).expect("CSPRNG tidak tersedia");
        let nilai = URL_SAFE_KETAT.encode(bytes);
        bytes.zeroize();
        Self { nilai }
    }

    /// The value to put in `Set-Cookie`. Only the webserver's cookie layer
    /// should call this.
    pub fn sebagai_b64(&self) -> &str {
        &self.nilai
    }

    /// What goes in `admin_session.token_hash`: lowercase hex SHA-256 of the
    /// token text.
    ///
    /// The hash covers the base64url *text*, not the raw bytes, so the same
    /// cookie string always hashes to the same row and the stored form is a
    /// plain function of what a client sends.
    pub fn hash(&self) -> HashToken {
        let mut h = Sha256::new();
        h.update(self.nilai.as_bytes());
        HashToken(hex_lower(&h.finalize()))
    }
}

impl fmt::Debug for TokenSesi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TokenSesi(<disembunyikan>)")
    }
}

/// A token as received that is not a well-formed session token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenTidakValid {
    Panjang { dapat: usize },
    Karakter,
}

impl fmt::Display for TokenTidakValid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Panjang { dapat } => write!(
                f,
                "token sesi {dapat} karakter, seharusnya {PANJANG_TOKEN_B64}"
            ),
            Self::Karakter => f.write_str("token sesi bukan base64url kanonik"),
        }
    }
}

impl std::error::Error for TokenTidakValid {}

/// SHA-256 of a token, as stored. 64 lowercase hex, matching the column's
/// `CHECK (token_hash ~ '^[0-9a-f]{64}$')`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct HashToken(String);

impl HashToken {
    pub fn parse(hex: impl Into<String>) -> Result<Self, HashTokenTidakValid> {
        let s = hex.into();
        if s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Ok(Self(s))
        } else {
            Err(HashTokenTidakValid)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Compare against another stored hash without early exit. Lookups are by
    /// SQL equality on an indexed column, so this is for the explicit
    /// comparisons (e.g. "is this the session we think it is"), where a timing
    /// difference would otherwise leak how much of the hash matched.
    pub fn sama_dengan(&self, lain: &Self) -> bool {
        self.0.as_bytes().ct_eq(lain.0.as_bytes()).into()
    }
}

impl fmt::Debug for HashToken {
    /// Not secret (it is the stored form), but never useful in a log and easy
    /// to confuse with the token, so it prints truncated.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HashToken({}…)", &self.0[..8.min(self.0.len())])
    }
}

/// A stored hash that is not 64 lowercase hex.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashTokenTidakValid;

impl fmt::Display for HashTokenTidakValid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("token_hash harus 64 karakter hex huruf kecil")
    }
}

impl std::error::Error for HashTokenTidakValid {}

/// Lowercase hex without pulling in a crate for nine lines.
fn hex_lower(bytes: &[u8]) -> String {
    const DIGIT: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGIT[(b >> 4) as usize] as char);
        out.push(DIGIT[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_token_is_the_expected_shape() {
        let t = TokenSesi::baru();
        assert_eq!(t.sebagai_b64().len(), PANJANG_TOKEN_B64);
        assert_eq!(
            URL_SAFE_KETAT.decode(t.sebagai_b64()).unwrap().len(),
            PANJANG_TOKEN
        );
    }

    #[test]
    fn two_tokens_never_match() {
        let mut terlihat = std::collections::HashSet::new();
        for _ in 0..64 {
            assert!(terlihat.insert(TokenSesi::baru().sebagai_b64().to_owned()));
        }
    }

    #[test]
    fn the_hash_is_sha256_hex_of_the_token_text() {
        let t = TokenSesi::baru();
        let h = t.hash();
        assert_eq!(h.as_str().len(), 64);
        assert!(
            h.as_str()
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );

        // Recompute independently: same text in, same digest out, stable across
        // processes (this is what makes a DB lookup by hash work at all).
        let mut d = Sha256::new();
        d.update(t.sebagai_b64().as_bytes());
        assert_eq!(h.as_str(), hex_lower(&d.finalize()));
    }

    #[test]
    fn the_hash_is_stable_for_the_same_token() {
        let t = TokenSesi::baru();
        let ulang = TokenSesi::dari_b64(t.sebagai_b64().to_owned()).unwrap();
        assert!(t.hash().sama_dengan(&ulang.hash()));
    }

    #[test]
    fn different_tokens_hash_differently() {
        assert!(
            !TokenSesi::baru()
                .hash()
                .sama_dengan(&TokenSesi::baru().hash())
        );
    }

    #[test]
    fn hex_encoder_matches_known_vectors() {
        assert_eq!(hex_lower(b""), "");
        assert_eq!(hex_lower(&[0x00]), "00");
        assert_eq!(hex_lower(&[0xff, 0x0a]), "ff0a");
        // SHA-256("abc") is a fixed vector, so a hex-encoder bug would show.
        let mut d = Sha256::new();
        d.update(b"abc");
        assert_eq!(
            hex_lower(&d.finalize()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_wrong_length_cookie_is_rejected() {
        assert_eq!(
            TokenSesi::dari_b64("a".repeat(42)).unwrap_err(),
            TokenTidakValid::Panjang { dapat: 42 }
        );
        assert!(matches!(
            TokenSesi::dari_b64("a".repeat(44)).unwrap_err(),
            TokenTidakValid::Panjang { dapat: 44 }
        ));
        assert!(TokenSesi::dari_b64("").is_err());
    }

    #[test]
    fn a_cookie_outside_the_alphabet_is_rejected() {
        // Standard base64's '+' and '/' are not URL-safe: a client sending them
        // is not sending one of our tokens.
        assert_eq!(
            TokenSesi::dari_b64(format!("{}+", "a".repeat(42))).unwrap_err(),
            TokenTidakValid::Karakter
        );
        assert!(TokenSesi::dari_b64(format!("{}/", "a".repeat(42))).is_err());
        assert!(TokenSesi::dari_b64(format!("{}=", "a".repeat(42))).is_err());
        assert!(TokenSesi::dari_b64(" ".repeat(43)).is_err());
    }

    #[test]
    fn a_non_canonical_cookie_is_rejected() {
        // 32 bytes is 43 base64 characters, so the final character carries only
        // two significant bits and four of its values encode the same bytes.
        // Build one of those sibling spellings with a deliberately *forgiving*
        // decoder (the default config allows non-zero trailing bits), then check
        // that our parser refuses it. Otherwise one session would have several
        // valid cookies and several hashes.
        let t = TokenSesi::baru();
        let kanonik = t.sebagai_b64().to_owned();
        let diizinkan = URL_SAFE_KETAT.decode(&kanonik).unwrap();
        let bytes = kanonik.clone().into_bytes();
        let last = bytes.len() - 1;

        // The forgiving engine: identical to `URL_SAFE_KETAT` except it keeps
        // the default `decode_allow_trailing_bits = true`. It exists only to
        // *construct* the sibling spellings this test refutes.
        let pemaaf = base64::engine::general_purpose::GeneralPurpose::new(
            &alphabet::URL_SAFE,
            GeneralPurposeConfig::new()
                .with_decode_allow_trailing_bits(true)
                .with_decode_padding_mode(DecodePaddingMode::RequireNone),
        );

        let mut kembar = Vec::new();
        for ganti in *b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_" {
            let mut coba = bytes.clone();
            coba[last] = ganti;
            let kandidat = String::from_utf8(coba).unwrap();
            if kandidat == kanonik {
                continue;
            }
            let kembar_dengan = pemaaf.decode(&kandidat).ok();
            if kembar_dengan.as_deref() != Some(diizinkan.as_slice()) {
                continue;
            }
            kembar.push(kandidat.clone());
            assert!(
                TokenSesi::dari_b64(kandidat.clone()).is_err(),
                "{kandidat} kembar dari token yang sama tapi diterima"
            );
            assert_ne!(
                TokenSesi::dari_b64(kandidat).map(|x| x.hash()).ok(),
                Some(t.hash())
            );
        }
        assert_eq!(kembar.len(), 3, "siblings ditemukan: {kembar:?}");

        // The canonical spelling itself still round-trips.
        assert_eq!(
            TokenSesi::dari_b64(kanonik.clone()).unwrap().sebagai_b64(),
            kanonik
        );
    }

    #[test]
    fn a_valid_cookie_round_trips() {
        let t = TokenSesi::baru();
        assert_eq!(TokenSesi::dari_b64(t.sebagai_b64().to_owned()).unwrap(), t);
    }

    #[test]
    fn debug_never_prints_the_token() {
        let t = TokenSesi::baru();
        let tercetak = format!("{t:?}");
        assert!(!tercetak.contains(t.sebagai_b64()), "{tercetak}");
        assert_eq!(tercetak, "TokenSesi(<disembunyikan>)");

        // Also when nested in another struct, the common accidental leak.
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Bungkus {
            token: TokenSesi,
        }
        let b = Bungkus { token: t.clone() };
        assert!(!format!("{b:?}").contains(t.sebagai_b64()));
    }

    #[test]
    fn hash_token_validation() {
        assert!(HashToken::parse("0".repeat(64)).is_ok());
        assert!(HashToken::parse("A".repeat(64)).is_err());
        assert!(HashToken::parse("0".repeat(63)).is_err());
        assert!(HashToken::parse("g".repeat(64)).is_err());
        assert!(HashToken::parse("").is_err());
    }

    #[test]
    fn hash_token_debug_is_truncated() {
        let h = HashToken::parse("0123456789abcdef".repeat(4)).unwrap();
        assert_eq!(format!("{h:?}"), "HashToken(01234567…)");
    }

    #[test]
    fn hash_token_comparison_is_correct() {
        let a = HashToken::parse("a".repeat(64)).unwrap();
        let b = HashToken::parse("a".repeat(64)).unwrap();
        let c = HashToken::parse(format!("{}b", "a".repeat(63))).unwrap();
        assert!(a.sama_dengan(&b));
        assert!(!a.sama_dengan(&c));
    }
}
