//! Mapping a failed provider call onto the shared retry table (ARC-4).
//!
//! AC-24: every failed call becomes one [`ErrorClass`] from `core::retry`.
//! No new classes are invented here; an unrecognised failure is `Lainnya`,
//! which `retryable` reports as final.
//!
//! Scope: ADR-0001 §7 puts the *provider-specific* mapping (which Dukcapil
//! body code means which class, including the `kuota_provider_habis` /
//! `rate_limit_provider` split) in the response-schema configuration, not in
//! code. This module therefore only covers what the transport itself
//! observes — timeout, connection, TLS, and HTTP status — and deliberately
//! contains no Dukcapil body-code table.

use poolrouter_core::retry::ErrorClass;

/// A provider call that did not produce a usable success response.
///
/// The variants mirror the transport and protocol failures the adapter can
/// observe. Keeping them separate from `ErrorClass` lets the caller log
/// enough detail while the retry decision stays a single table lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GagalPanggilProvider {
    /// The exchange exceeded its deadline.
    Timeout,
    /// Connection could not be established or was dropped mid-flight.
    Koneksi(String),
    /// TLS handshake or certificate validation failed (D5 trust mode).
    Tls(String),
    /// Provider answered 4xx other than 401/403/429.
    HttpKlien { status: u16 },
    /// Provider answered 5xx.
    HttpServer { status: u16 },
    /// Provider answered 401 or 403.
    AuthDitolak { status: u16 },
    /// Provider answered 429.
    RateLimit { status: u16 },
    /// Provider signalled exhausted quota in its own body.
    KuotaHabis,
    /// The body did not match the configured response schema.
    ResponsTidakValid(String),
}

/// Map one failure onto the retry table.
///
/// A 4xx that is not auth or rate-limit is a client-side configuration
/// problem: retrying the same request cannot succeed, so it is `Lainnya`.
pub fn kelas_error(gagal: &GagalPanggilProvider) -> ErrorClass {
    match gagal {
        GagalPanggilProvider::Timeout => ErrorClass::Timeout,
        GagalPanggilProvider::Koneksi(_) => ErrorClass::Koneksi,
        GagalPanggilProvider::Tls(_) => ErrorClass::Tls,
        GagalPanggilProvider::ResponsTidakValid(_) => ErrorClass::ResponsTidakValid,
        GagalPanggilProvider::AuthDitolak { .. } => ErrorClass::AuthDitolak,
        GagalPanggilProvider::KuotaHabis => ErrorClass::KuotaProviderHabis,
        GagalPanggilProvider::RateLimit { .. } => ErrorClass::RateLimitProvider,
        GagalPanggilProvider::HttpServer { .. } => ErrorClass::ProviderSementara,
        GagalPanggilProvider::HttpKlien { .. } => ErrorClass::Lainnya,
    }
}

/// Map a non-2xx HTTP status onto a failure.
///
/// A 2xx here means the caller reached this with a success response; that is
/// a programming error, not a provider fault, so it is `Lainnya` and final
/// rather than being silently retried as a transient provider problem.
pub fn dari_status_http(status: u16) -> GagalPanggilProvider {
    match status {
        401 | 403 => GagalPanggilProvider::AuthDitolak { status },
        429 => GagalPanggilProvider::RateLimit { status },
        s if (400..500).contains(&s) => GagalPanggilProvider::HttpKlien { status: s },
        s if (500..600).contains(&s) => GagalPanggilProvider::HttpServer { status: s },
        s => GagalPanggilProvider::HttpKlien { status: s },
    }
}
