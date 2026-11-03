//! Closed value sets shared by downstream and admin APIs. Names follow data
//! contract v0.7.10; values not yet transcribed from the contract are listed in
//! the TODO notes and must be completed before review.

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

/// `failure_reason` of a `failed` request (contract section 3.2.5).
// TODO(SHAA-175): complete from contract v0.7.10 section 3.2.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    RetryHabis,
    ObjekNonaktif,
    TimeoutSync,
}

/// `cancel_reason` of a `cancelled` request (contract section 3.2.3).
// TODO(SHAA-175): complete from contract v0.7.10 section 3.2.3 (one value per entity kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    KredensialNonaktif,
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
