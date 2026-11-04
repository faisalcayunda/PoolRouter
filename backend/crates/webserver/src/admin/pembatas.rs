//! Login limiter in Redis: 5 failures per username in 15 minutes lock that
//! username until the window ends (ADR-0003 rev 5 §3.6, Shu SHAA-196
//! 2026-11-04).
//!
//! - **Key:** `pr:login:gagal:<hex HMAC-SHA256(key, "login-limit" 0x00 username)>`.
//!   The HMAC key is one the webserver already holds; the label separates this
//!   use from the others. The typed username never reaches Redis, the same
//!   principle as `audit_login`.
//! - **Atomic:** one Lua script counts (`INCR`, and `PEXPIRE` 15 min on the
//!   first failure of the window); another checks. `Retry-After` comes from
//!   `PTTL`.
//! - **Fail-closed:** if Redis cannot be reached the caller gets an error and
//!   login answers 503 before any password work or session. Failing open would
//!   allow unlimited guessing during an outage.
//! - Known and unknown usernames take exactly the same path, so the limiter
//!   cannot tell an account from a missing one.

use std::time::Duration;

use aws_lc_rs::hmac;
use redis::Script;

pub const MAKS_GAGAL: u32 = 5;
pub const JENDELA_MS: u64 = 15 * 60 * 1000;
const PREFIKS: &str = "pr:login:gagal:";
const LABEL: &[u8] = b"login-limit\0";
/// Redis budget for one limiter call, connection included.
const BATAS_WAKTU: Duration = Duration::from_millis(500);

/// Locked when the count has reached the limit: remaining ms of the window,
/// else -1. A counter that somehow has no TTL is deleted instead of locking
/// forever.
const SKRIP_PERIKSA: &str = r#"
local n = tonumber(redis.call('GET', KEYS[1]) or '0')
if n >= tonumber(ARGV[1]) then
  local t = redis.call('PTTL', KEYS[1])
  if t < 0 then redis.call('DEL', KEYS[1]) return -1 end
  return t
end
return -1
"#;

/// Count one failure; the first of a window starts the 15-minute expiry.
const SKRIP_GAGAL: &str = r#"
local n = redis.call('INCR', KEYS[1])
if n == 1 or redis.call('PTTL', KEYS[1]) < 0 then
  redis.call('PEXPIRE', KEYS[1], ARGV[1])
end
return n
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HasilPeriksa {
    Bebas,
    /// Seconds until the window ends, at least 1.
    Terkunci {
        coba_lagi_detik: u64,
    },
}

/// Redis could not be reached or answered in time. Carries no detail on
/// purpose: driver text can include addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedisTidakTersedia;

pub struct PembatasLogin {
    klien: redis::Client,
    kunci: hmac::Key,
    periksa: Script,
    gagal: Script,
}

impl PembatasLogin {
    pub fn baru(klien: redis::Client, kunci_hmac: &[u8]) -> Self {
        Self {
            klien,
            kunci: hmac::Key::new(hmac::HMAC_SHA256, kunci_hmac),
            periksa: Script::new(SKRIP_PERIKSA),
            gagal: Script::new(SKRIP_GAGAL),
        }
    }

    /// The Redis key for a normalised username.
    pub fn kunci_redis(&self, username: &str) -> String {
        let mut pesan = Vec::with_capacity(LABEL.len() + username.len());
        pesan.extend_from_slice(LABEL);
        pesan.extend_from_slice(username.as_bytes());
        let tag = hmac::sign(&self.kunci, &pesan);
        let mut k = String::with_capacity(PREFIKS.len() + 64);
        k.push_str(PREFIKS);
        for b in tag.as_ref() {
            k.push_str(&format!("{b:02x}"));
        }
        k
    }

    async fn jalankan<T, F, Fut>(&self, f: F) -> Result<T, RedisTidakTersedia>
    where
        F: FnOnce(redis::aio::MultiplexedConnection) -> Fut,
        Fut: std::future::Future<Output = redis::RedisResult<T>>,
    {
        let kerja = async {
            let conn = self.klien.get_multiplexed_async_connection().await?;
            f(conn).await
        };
        match tokio::time::timeout(BATAS_WAKTU, kerja).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => {
                tracing::error!(kind = ?e.kind(), "admin: pembatas login: Redis gagal");
                Err(RedisTidakTersedia)
            }
            Err(_) => {
                tracing::error!("admin: pembatas login: Redis melewati batas waktu");
                Err(RedisTidakTersedia)
            }
        }
    }

    pub async fn periksa(&self, username: &str) -> Result<HasilPeriksa, RedisTidakTersedia> {
        let kunci = self.kunci_redis(username);
        let sisa_ms: i64 = self
            .jalankan(|mut c| async move {
                self.periksa
                    .key(&kunci)
                    .arg(MAKS_GAGAL)
                    .invoke_async(&mut c)
                    .await
            })
            .await?;
        Ok(if sisa_ms < 0 {
            HasilPeriksa::Bebas
        } else {
            HasilPeriksa::Terkunci {
                coba_lagi_detik: (sisa_ms as u64).div_ceil(1000).max(1),
            }
        })
    }

    pub async fn catat_gagal(&self, username: &str) -> Result<(), RedisTidakTersedia> {
        let kunci = self.kunci_redis(username);
        let _n: i64 = self
            .jalankan(|mut c| async move {
                self.gagal
                    .key(&kunci)
                    .arg(JENDELA_MS)
                    .invoke_async(&mut c)
                    .await
            })
            .await?;
        Ok(())
    }

    /// A successful login clears the count.
    pub async fn berhasil(&self, username: &str) -> Result<(), RedisTidakTersedia> {
        let kunci = self.kunci_redis(username);
        let _n: i64 = self
            .jalankan(
                |mut c| async move { redis::cmd("DEL").arg(&kunci).query_async(&mut c).await },
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pembatas(kunci: &[u8]) -> PembatasLogin {
        // Never connects: only key derivation is exercised here.
        PembatasLogin::baru(redis::Client::open("redis://127.0.0.1:1/").unwrap(), kunci)
    }

    #[test]
    fn kunci_stabil_berbeda_per_username_dan_tidak_memuat_username() {
        let p = pembatas(b"kunci-uji");
        let a = p.kunci_redis("auditor_01");
        assert_eq!(a, p.kunci_redis("auditor_01"));
        assert_ne!(a, p.kunci_redis("auditor_02"));
        assert!(a.starts_with(PREFIKS));
        assert!(!a.contains("auditor"));
        let hex = &a[PREFIKS.len()..];
        assert_eq!(hex.len(), 64);
        assert!(
            hex.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
    }

    #[test]
    fn kunci_bergantung_pada_kunci_hmac() {
        assert_ne!(
            pembatas(b"kunci-satu").kunci_redis("x"),
            pembatas(b"kunci-dua").kunci_redis("x")
        );
    }
}
