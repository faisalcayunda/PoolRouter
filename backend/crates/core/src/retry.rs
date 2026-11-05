//! Retry policy controlled by PoolRouter (ADR-0001 rev 15 section 5a.3, R-03).
//!
//! Three pieces, all pure so the worker and the contract tests share one code
//! path:
//!
//! - [`ErrorClass`] and [`ErrorClass::retryable`]: which failures are retried.
//!   Unknown provider codes are `lainnya` and are never retried.
//! - [`SumberAcakBackoff`]: the single injection point for the random draw.
//!   Production uses [`AcakAwsLc`]; contract tests use [`AcakBatasAtas`].
//! - [`KebijakanBackoff::not_before`]: full jitter,
//!   `now + random(0, min(cap, base * 2^(n-1)))`, compared with
//!   `ttl_expires_at` by the same code in every environment.
//!
//! The retry budget itself (rolling 1 hour, per pool) is enforced atomically in
//! the dispatch script (`kuota::store`); [`budget_retry_tersedia`] mirrors the
//! predicate for unit tests and metrics decisions outside Redis.

use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use chrono::{DateTime, Duration, Utc};

/// `error_class` values of `transaction_log` (migration `request_log`, R-08).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorClass {
    Timeout,
    Koneksi,
    Tls,
    ResponsTidakValid,
    AuthDitolak,
    KuotaProviderHabis,
    RateLimitProvider,
    ProviderSementara,
    Lainnya,
}

impl ErrorClass {
    /// Database / metric label value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Koneksi => "koneksi",
            Self::Tls => "tls",
            Self::ResponsTidakValid => "respons_tidak_valid",
            Self::AuthDitolak => "auth_ditolak",
            Self::KuotaProviderHabis => "kuota_provider_habis",
            Self::RateLimitProvider => "rate_limit_provider",
            Self::ProviderSementara => "provider_sementara",
            Self::Lainnya => "lainnya",
        }
    }

    /// Parses a stored value. Anything outside the set is `None`; the caller
    /// decides (a stored row can never hold such a value, the CHECK forbids it).
    pub fn dari_str(s: &str) -> Option<Self> {
        Some(match s {
            "timeout" => Self::Timeout,
            "koneksi" => Self::Koneksi,
            "tls" => Self::Tls,
            "respons_tidak_valid" => Self::ResponsTidakValid,
            "auth_ditolak" => Self::AuthDitolak,
            "kuota_provider_habis" => Self::KuotaProviderHabis,
            "rate_limit_provider" => Self::RateLimitProvider,
            "provider_sementara" => Self::ProviderSementara,
            "lainnya" => Self::Lainnya,
            _ => return None,
        })
    }

    /// R-03 table. Everything not listed as retryable is final, so a new
    /// variant defaults to "no retry" only if it is added to the `false` arm
    /// on purpose; the match is exhaustive and the compiler forces the choice.
    pub fn retryable(self) -> bool {
        match self {
            Self::Timeout | Self::Koneksi | Self::RateLimitProvider | Self::ProviderSementara => {
                true
            }
            Self::Tls
            | Self::ResponsTidakValid
            | Self::AuthDitolak
            | Self::KuotaProviderHabis
            | Self::Lainnya => false,
        }
    }
}

/// Random source for the backoff draw (contract KP-05).
pub trait SumberAcakBackoff: Send + Sync {
    /// A value in `0..=batas_ms`. `batas_ms` is the full-jitter upper bound.
    fn acak_hingga(&self, batas_ms: u64) -> u64;
}

/// Production source, backed by `aws-lc-rs` (ADR-0004 section 4.4).
#[derive(Default)]
pub struct AcakAwsLc(SystemRandom);

impl AcakAwsLc {
    pub fn baru() -> Self {
        Self(SystemRandom::new())
    }
}

impl SumberAcakBackoff for AcakAwsLc {
    fn acak_hingga(&self, batas_ms: u64) -> u64 {
        if batas_ms == 0 {
            return 0;
        }
        // Rejection sampling keeps the draw uniform on 0..=batas_ms.
        let rentang = u128::from(batas_ms) + 1;
        let zona = (u128::from(u64::MAX) + 1) / rentang * rentang;
        loop {
            let mut buf = [0u8; 8];
            // A failing system RNG is not recoverable here: fall back to the
            // upper bound, which only delays the retry (never retries early).
            if self.0.fill(&mut buf).is_err() {
                return batas_ms;
            }
            let v = u128::from(u64::from_le_bytes(buf));
            if v < zona {
                return (v % rentang) as u64;
            }
        }
    }
}

/// Contract-test source: always the upper bound, so `not_before` is
/// deterministic.
pub struct AcakBatasAtas;

impl SumberAcakBackoff for AcakBatasAtas {
    fn acak_hingga(&self, batas_ms: u64) -> u64 {
        batas_ms
    }
}

/// Outcome of scheduling a retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeputusanRetry {
    /// Return the request to `queued` with this `not_before`.
    Jadwalkan(DateTime<Utc>),
    /// `failed` / `retry_habis` (a): attempt limit reached.
    BatasPercobaanHabis,
    /// `failed` / `retry_habis` (b): `not_before` would pass `ttl_expires_at`.
    MelewatiTtl,
    /// The error class is not retried (R-03).
    TidakDiretry,
}

/// Per pool backoff parameters (KP-01 ranges are validated at the write path).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KebijakanBackoff {
    pub base_detik: u32,
    pub cap_detik: u32,
    pub batas_percobaan: u32,
}

impl Default for KebijakanBackoff {
    fn default() -> Self {
        Self {
            base_detik: 5,
            cap_detik: 300,
            batas_percobaan: 3,
        }
    }
}

impl KebijakanBackoff {
    /// Upper bound of the jitter window after `attempt_count` failed attempts
    /// (`n >= 1`): `min(cap, base * 2^(n-1))`, saturating, in milliseconds.
    pub fn batas_atas_ms(&self, attempt_count: u32) -> u64 {
        let n = attempt_count.max(1);
        let cap = u64::from(self.cap_detik) * 1000;
        let base = u64::from(self.base_detik) * 1000;
        // 2^(n-1) overflows u64 only above n = 64; the cap is reached far earlier.
        let kali = 1u64.checked_shl(n - 1).unwrap_or(u64::MAX);
        base.saturating_mul(kali).min(cap)
    }

    /// Decides what happens after a failed attempt. `attempt_count` already
    /// includes the attempt that just failed.
    pub fn putuskan(
        &self,
        kelas: ErrorClass,
        attempt_count: u32,
        sekarang: DateTime<Utc>,
        ttl_expires_at: DateTime<Utc>,
        acak: &dyn SumberAcakBackoff,
    ) -> KeputusanRetry {
        if !kelas.retryable() {
            return KeputusanRetry::TidakDiretry;
        }
        if attempt_count >= self.batas_percobaan {
            return KeputusanRetry::BatasPercobaanHabis;
        }
        let jeda_ms = acak.acak_hingga(self.batas_atas_ms(attempt_count));
        // jeda_ms <= cap (<= 3600 s), so the conversion cannot overflow i64.
        let not_before = sekarang + Duration::milliseconds(jeda_ms as i64);
        if not_before >= ttl_expires_at {
            return KeputusanRetry::MelewatiTtl;
        }
        KeputusanRetry::Jadwalkan(not_before)
    }
}

/// Retry budget predicate (ADR-0001 section 5a.3):
/// `retry_jendela < max(retry_min, rasio * percobaan_jendela)`.
pub fn budget_retry_tersedia(
    retry_jendela: u64,
    percobaan_jendela: u64,
    rasio_retry: f64,
    retry_min: u64,
) -> bool {
    if !rasio_retry.is_finite() || rasio_retry < 0.0 {
        // Fail-closed: a broken ratio never grants a retry.
        return false;
    }
    let proporsional = rasio_retry * percobaan_jendela as f64;
    (retry_jendela as f64) < proporsional.max(retry_min as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(detik: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000 + detik, 0).unwrap()
    }

    const SEMUA: [ErrorClass; 9] = [
        ErrorClass::Timeout,
        ErrorClass::Koneksi,
        ErrorClass::Tls,
        ErrorClass::ResponsTidakValid,
        ErrorClass::AuthDitolak,
        ErrorClass::KuotaProviderHabis,
        ErrorClass::RateLimitProvider,
        ErrorClass::ProviderSementara,
        ErrorClass::Lainnya,
    ];

    #[test]
    fn retry_table_matches_adr() {
        let ya = [
            ErrorClass::Timeout,
            ErrorClass::Koneksi,
            ErrorClass::RateLimitProvider,
            ErrorClass::ProviderSementara,
        ];
        for k in SEMUA {
            assert_eq!(k.retryable(), ya.contains(&k), "{k:?}");
        }
    }

    #[test]
    fn str_roundtrip_and_unknown() {
        for k in SEMUA {
            assert_eq!(ErrorClass::dari_str(k.as_str()), Some(k));
        }
        assert_eq!(ErrorClass::dari_str("ajaib"), None);
    }

    #[test]
    fn upper_bound_doubles_then_caps() {
        let p = KebijakanBackoff::default();
        assert_eq!(p.batas_atas_ms(1), 5_000);
        assert_eq!(p.batas_atas_ms(2), 10_000);
        assert_eq!(p.batas_atas_ms(3), 20_000);
        assert_eq!(p.batas_atas_ms(7), 300_000);
        assert_eq!(p.batas_atas_ms(64), 300_000);
        assert_eq!(p.batas_atas_ms(u32::MAX), 300_000);
        // n = 0 is treated as the first failure, never as a zero window.
        assert_eq!(p.batas_atas_ms(0), 5_000);
    }

    #[test]
    fn schedules_with_stub_at_upper_bound() {
        let p = KebijakanBackoff::default();
        let d = p.putuskan(ErrorClass::Timeout, 2, t(0), t(3600), &AcakBatasAtas);
        assert_eq!(d, KeputusanRetry::Jadwalkan(t(10)));
    }

    #[test]
    fn non_retryable_class_is_final() {
        let p = KebijakanBackoff::default();
        for k in [
            ErrorClass::Tls,
            ErrorClass::ResponsTidakValid,
            ErrorClass::AuthDitolak,
            ErrorClass::KuotaProviderHabis,
            ErrorClass::Lainnya,
        ] {
            assert_eq!(
                p.putuskan(k, 1, t(0), t(3600), &AcakBatasAtas),
                KeputusanRetry::TidakDiretry
            );
        }
    }

    #[test]
    fn attempt_limit_and_ttl() {
        let p = KebijakanBackoff::default();
        assert_eq!(
            p.putuskan(ErrorClass::Koneksi, 3, t(0), t(3600), &AcakBatasAtas),
            KeputusanRetry::BatasPercobaanHabis
        );
        // not_before == ttl_expires_at already counts as past the TTL.
        assert_eq!(
            p.putuskan(ErrorClass::Koneksi, 1, t(0), t(5), &AcakBatasAtas),
            KeputusanRetry::MelewatiTtl
        );
        assert_eq!(
            p.putuskan(ErrorClass::Koneksi, 1, t(0), t(6), &AcakBatasAtas),
            KeputusanRetry::Jadwalkan(t(5))
        );
    }

    #[test]
    fn aws_lc_draw_stays_in_window() {
        let a = AcakAwsLc::baru();
        assert_eq!(a.acak_hingga(0), 0);
        let mut ada_beda = false;
        let mut pertama = None;
        for _ in 0..200 {
            let v = a.acak_hingga(300_000);
            assert!(v <= 300_000);
            match pertama {
                None => pertama = Some(v),
                Some(p) if p != v => ada_beda = true,
                _ => {}
            }
        }
        assert!(ada_beda, "200 draws were all identical");
    }

    #[test]
    fn budget_predicate() {
        // Below the floor: allowed even with no attempts in the window.
        assert!(budget_retry_tersedia(19, 0, 0.10, 20));
        assert!(!budget_retry_tersedia(20, 0, 0.10, 20));
        // Above the floor the ratio governs: 10% of 1000 = 100.
        assert!(budget_retry_tersedia(99, 1000, 0.10, 20));
        assert!(!budget_retry_tersedia(100, 1000, 0.10, 20));
        // Broken ratio is fail-closed.
        assert!(!budget_retry_tersedia(0, 1000, f64::NAN, 20));
        assert!(!budget_retry_tersedia(0, 1000, -0.1, 20));
    }
}
