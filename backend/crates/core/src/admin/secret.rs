//! Server-generated secrets for credentials and webhook signing (D-1).
//!
//! The value is returned to the caller once and the type deliberately keeps
//! its debug representation opaque. Persist callers should store only `hash`.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const PANJANG_SECRET: usize = 32;

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretSekali {
    nilai: String,
}

impl SecretSekali {
    pub fn baru() -> Self {
        let mut bytes = [0u8; PANJANG_SECRET];
        getrandom::fill(&mut bytes).expect("CSPRNG tidak tersedia");
        let nilai = URL_SAFE_NO_PAD.encode(bytes);
        bytes.zeroize();
        Self { nilai }
    }

    pub fn expose(&self) -> &str {
        &self.nilai
    }

    pub fn hash(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(self.nilai.as_bytes());
        digest
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

impl fmt::Debug for SecretSekali {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretSekali([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_256_bit_url_secret_and_hash() {
        let secret = SecretSekali::baru();
        assert_eq!(secret.expose().len(), 43);
        assert!(
            secret
                .expose()
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_eq!(secret.hash().len(), 64);
        assert!(!format!("{secret:?}").contains(secret.expose()));
    }

    #[test]
    fn each_secret_is_fresh() {
        assert_ne!(SecretSekali::baru().expose(), SecretSekali::baru().expose());
    }
}
