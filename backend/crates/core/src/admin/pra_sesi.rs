//! Pre-session tokens for the two-step admin login (ADR-0006 §9, AC-9..AC-13).
//!
//! After a correct password an MFA-required user gets no session. They get a
//! short-lived opaque token in the `__Host-pr_mfa` cookie instead, and
//! [`PraSesiStore`] is the only place it exists server-side.
//!
//! - **Keyed by hash, never by the token.** Redis holds `sha256(token)`, the
//!   same principle as `admin_session.token_sha256`: a Redis dump yields no
//!   usable credential.
//! - **One token, one success.** [`PraSesiStore::pakai`] is a single `GETDEL`
//!   through a Lua script, so two concurrent `POST /auth/mfa` calls with the
//!   same token cannot both mint a session (AC-10, AC-13 "konfirmasi ulang →
//!   401").
//! - **Failures are counted against the same bucket as a wrong password.**
//!   [`PraSesiStore::catat_gagal`] increments the per-token failure counter; at
//!   [`BATAS_GAGAL`] the token is destroyed so it cannot be retried at all
//!   (AC-11 "kegagalan kelima menghanguskan token pra-sesi").
//! - **TTL matches the cookie** ([`UMUR_DETIK`]): 300 seconds, so a token that
//!   outlived the browser cookie is already gone.
//!
//! The typed code never reaches Redis, the audit trail or a log line; only the
//! failure count does.

use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use redis::Script;
use redis::aio::ConnectionManager;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// How long a pre-session token lives. Matches the cookie `Max-Age` (AC-9).
pub const UMUR_DETIK: u64 = 300;

/// Failures allowed against one pre-session token before it is destroyed.
pub const BATAS_GAGAL: i64 = 5;

/// What the second step is allowed to do with this token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JenisPraSesi {
    /// Password checked, TOTP already active: `POST /auth/mfa` verifies a code.
    Mfa,
    /// Password checked, no active MFA: `POST /auth/mfa/daftar` enrolls.
    DaftarMfa,
}

impl JenisPraSesi {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mfa => "mfa",
            Self::DaftarMfa => "daftar_mfa",
        }
    }

    fn dari_str(teks: &str) -> Option<Self> {
        match teks {
            "mfa" => Some(Self::Mfa),
            "daftar_mfa" => Some(Self::DaftarMfa),
            _ => None,
        }
    }
}

impl std::fmt::Display for JenisPraSesi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A resolved pre-session: `pengguna_id` plus what the token may still do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PraSesi {
    pub pengguna_id: Uuid,
    pub jenis: JenisPraSesi,
}

/// Failure modes that the handler must distinguish.
#[derive(Debug)]
pub enum GalatPraSesi {
    /// Redis could not be reached: fail closed, no second step.
    Penyimpanan(redis::RedisError),
    /// The stored value did not parse; treated as no token at all.
    DataRusak,
}

impl std::fmt::Display for GalatPraSesi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Penyimpanan(e) => write!(f, "penyimpanan pra-sesi gagal: {e}"),
            Self::DataRusak => f.write_str("data pra-sesi rusak"),
        }
    }
}

impl std::error::Error for GalatPraSesi {}

impl From<redis::RedisError> for GalatPraSesi {
    fn from(e: redis::RedisError) -> Self {
        Self::Penyimpanan(e)
    }
}

/// `GETDEL` plus the failure counter stored beside it. Kept as one script so a
/// token is consumed exactly once even under concurrent calls.
const SKRIP_PAKAI: &str = r"
local v = redis.call('GET', KEYS[1])
if v then
  redis.call('DEL', KEYS[1])
  redis.call('DEL', KEYS[2])
end
return v
";

/// Count a failure and destroy the token once the limit is reached.
const SKRIP_GAGAL: &str = r"
local n = redis.call('INCR', KEYS[2])
if n == 1 then
  redis.call('PEXPIRE', KEYS[2], ARGV[1])
end
if n >= tonumber(ARGV[2]) then
  redis.call('DEL', KEYS[1])
  redis.call('DEL', KEYS[2])
  return -1
end
return n
";

/// Redis-backed pre-session store for the two-step login.
///
/// The client is held, not a connection: `pembatas` already takes that shape,
/// so a Redis restart between the two login steps heals without a restart.
#[derive(Clone)]
pub struct PraSesiStore {
    klien: redis::Client,
    pakai: Script,
    gagal: Script,
}

impl PraSesiStore {
    #[must_use]
    pub fn baru(klien: redis::Client) -> Self {
        Self {
            klien,
            pakai: Script::new(SKRIP_PAKAI),
            gagal: Script::new(SKRIP_GAGAL),
        }
    }

    async fn koneksi(&self) -> Result<ConnectionManager, GalatPraSesi> {
        self.klien
            .get_connection_manager()
            .await
            .map_err(GalatPraSesi::Penyimpanan)
    }

    /// Store a new pre-session and return the token to put in the cookie.
    ///
    /// The token is 256 bits from the OS CSPRNG and only its SHA-256 is stored.
    pub async fn terbitkan(
        &self,
        pengguna_id: Uuid,
        jenis: JenisPraSesi,
        sekarang: DateTime<Utc>,
    ) -> Result<String, GalatPraSesi> {
        let mut raw = [0u8; 32];
        getrandom::fill(&mut raw).map_err(|_| GalatPraSesi::DataRusak)?;
        let token = base64url(&raw);
        let hash = hash_token(&token);
        let nilai = format!("{pengguna_id}:{}:{}", jenis.as_str(), sekarang.timestamp());
        let mut koneksi = self.koneksi().await?;
        let _: () = koneksi
            .set_ex(kunci(&hash), nilai, UMUR_DETIK)
            .await
            .map_err(GalatPraSesi::Penyimpanan)?;
        Ok(token)
    }

    /// Consume the token and return what it carried. `None` means unknown,
    /// expired, already used, or destroyed by too many failures.
    pub async fn pakai(&self, token: &str) -> Result<Option<PraSesi>, GalatPraSesi> {
        let hash = hash_token(token);
        let mut koneksi = self.koneksi().await?;
        let nilai: Option<String> = self
            .pakai
            .key(kunci(&hash))
            .key(kunci_gagal(&hash))
            .invoke_async(&mut koneksi)
            .await?;
        let Some(nilai) = nilai else {
            return Ok(None);
        };
        urai_nilai(&nilai).map(Some).ok_or(GalatPraSesi::DataRusak)
    }

    /// Record a failed code against this token. Returns `true` if the token is
    /// now destroyed, which the caller reports as the generic 401.
    pub async fn catat_gagal(&self, token: &str) -> Result<bool, GalatPraSesi> {
        let hash = hash_token(token);
        let mut koneksi = self.koneksi().await?;
        let sisa: i64 = self
            .gagal
            .key(kunci(&hash))
            .key(kunci_gagal(&hash))
            .arg(UMUR_DETIK * 1000)
            .arg(BATAS_GAGAL)
            .invoke_async(&mut koneksi)
            .await?;
        Ok(sisa == -1)
    }

    /// Drop the token without counting a failure (used after a success that
    /// already consumed it, and on explicit logout of the second step).
    pub async fn buang(&self, token: &str) -> Result<(), GalatPraSesi> {
        let hash = hash_token(token);
        let mut koneksi = self.koneksi().await?;
        let _: () = koneksi
            .del(&[kunci(&hash), kunci_gagal(&hash)])
            .await
            .map_err(GalatPraSesi::Penyimpanan)?;
        Ok(())
    }
}

fn urai_nilai(nilai: &str) -> Option<PraSesi> {
    let mut bagian = nilai.split(':');
    let pengguna_id = Uuid::parse_str(bagian.next()?).ok()?;
    let jenis = JenisPraSesi::dari_str(bagian.next()?)?;
    Some(PraSesi { pengguna_id, jenis })
}

fn kunci(hash: &str) -> String {
    format!("pr:pra_sesi:{hash}")
}

fn kunci_gagal(hash: &str) -> String {
    format!("pr:pra_sesi_gagal:{hash}")
}

/// SHA-256 of the token as lowercase hex; the token itself is never a key.
fn hash_token(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    h.finalize().iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn base64url(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for potong in data.chunks(3) {
        let b = [
            potong[0],
            *potong.get(1).unwrap_or(&0),
            *potong.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let sisa = potong.len();
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if sisa > 1 {
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        }
        if sisa > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jenis_bolak_balik_lewat_string() {
        for j in [JenisPraSesi::Mfa, JenisPraSesi::DaftarMfa] {
            assert_eq!(JenisPraSesi::dari_str(j.as_str()), Some(j));
        }
        assert_eq!(JenisPraSesi::dari_str("lain"), None);
    }

    #[test]
    fn nilai_uraian_memuat_pengguna_dan_jenis() {
        let id = Uuid::now_v7();
        let nilai = format!("{id}:daftar_mfa:1700000000");
        assert_eq!(
            urai_nilai(&nilai),
            Some(PraSesi {
                pengguna_id: id,
                jenis: JenisPraSesi::DaftarMfa
            })
        );
        assert_eq!(urai_nilai("rusak"), None);
    }

    #[test]
    fn hash_bukan_token_dan_stabil() {
        let a = hash_token("token-uji");
        assert_eq!(a, hash_token("token-uji"));
        assert_ne!(a, "token-uji");
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn base64url_tanpa_padding() {
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
    }
}
