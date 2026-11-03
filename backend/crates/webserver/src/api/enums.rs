//! Closed value sets shared by downstream and admin APIs. Names and values
//! follow data contract v0.7.10 (see `backend/openapi/DESIGN.md`).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Status exposed by polling and webhook (ADR-0001 section 4.1). Never goes backwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApiStatus {
    Queued,
    Processing,
    Done,
    Failed,
    Expired,
    Cancelled,
}

/// `failure_reason` of a `failed` request (contract section 3.2.5). Closed set,
/// shown to clients; the `kode` of a sync 502/504 body equals this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    /// Last attempt failed with a transient class and no attempt is left, or the
    /// retry window ran out. Sync too since v0.7.10 (effective attempt limit 1).
    RetryHabis,
    /// Attempt failed with a class that is not retried (auth, provider quota,
    /// unmapped code, invalid response, TLS).
    ProviderErrorPermanen,
    /// Sync provider call did not finish in time.
    TimeoutSync,
    /// K-20 hit a request that was waiting for retry (`attempt_count > 0`).
    ObjekNonaktif,
}

/// `cancel_reason` of a `cancelled` request (contract section 3.2.3). One value
/// per entity kind; both pool kinds share `pool_nonaktif`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    DibatalkanClient,
    ClientNonaktif,
    KredensialNonaktif,
    EndpointNonaktif,
    PoolNonaktif,
    ProviderNonaktif,
}

/// `kode` of a 429 response (ADR-0001 section 5a.7). Each comes with `Retry-After`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitKode {
    KuotaHabis,
    PorsiSyncHabis,
    PenolakanBerlebih,
}

/// Interface language (ADR-0003 Addendum A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BahasaUi {
    Id,
    En,
}

/// Jenis objek yang bisa dinonaktifkan (ADR-0001 section 4.1). Dikirim di
/// `jenis_objek` pada 403/503 `objek_nonaktif`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JenisObjek {
    Endpoint,
    DownstreamPool,
    UpstreamPool,
    Provider,
}

/// Jenis rule validasi pada konfigurasi provider (BRD US-03). F0 tidak
/// menyediakan rule kustom; nilai di luar enum ini ditolak 422.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuleJenis {
    Regex,
    Enum,
    Panjang,
    FormatTanggal,
}

/// Jenis kredensial downstream (contract section 3.1b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JenisKredensial {
    Oauth2,
    ApiKey,
}

/// Periode kuota upstream pool (ADR-0001 section 3a.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JendelaKuota {
    Bergulir24j,
    Kalender,
}

/// Status per elemen pada `hasil` (contract section 3.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatusHasil {
    Sesuai,
    TidakSesuai,
}
