//! RFC 9457 `application/problem+json` (R-10, ADR-0001 section 5a.7).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::problem_kode::ProblemKode;

/// Base URI for problem `type` (ADR-0001 rev 15 section 5a.7).
const TYPE_BASE: &str = "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#";

/// Error body. `detail` never contains NIK, payload or secret values (R-16).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Problem {
    /// URI reference identifying the problem type (anchor in docs/masalah.md).
    #[serde(rename = "type")]
    pub type_: String,
    pub title: String,
    pub status: u16,
    pub detail: Option<String>,
    /// Closed machine-readable code, one of `ProblemKode`.
    pub kode: String,
}

impl Problem {
    /// Stable `type` URI: one anchor per `kode` in `docs/masalah.md` (5a.7).
    pub fn uri(kode: ProblemKode) -> String {
        format!("{TYPE_BASE}{}", kode.as_str())
    }

    pub fn not_implemented() -> Self {
        Self {
            // Contract stub only; the anchor pair applies once handlers use
            // real `ProblemKode` values (5a.7 pairs `type` and `kode` 1:1).
            type_: "about:blank".into(),
            title: "Not Implemented".into(),
            status: 501,
            detail: Some("Contract stub; handler lands after contract review.".into()),
            kode: "belum_diimplementasi".into(),
        }
    }
}
