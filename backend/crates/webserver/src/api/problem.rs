//! RFC 9457 `application/problem+json` (R-10, ADR-0001 section 5a.7).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Error body. `detail` never contains NIK, payload or secret values (R-16).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Problem {
    /// URI reference identifying the problem type.
    #[serde(rename = "type")]
    pub type_: String,
    pub title: String,
    pub status: u16,
    pub detail: Option<String>,
    /// Closed machine-readable code (`ProblemKode` once transcribed).
    pub kode: String,
}

impl Problem {
    pub fn not_implemented() -> Self {
        Self {
            type_: "about:blank".into(),
            title: "Not Implemented".into(),
            status: 501,
            detail: Some("Contract stub; handler lands after contract review.".into()),
            kode: "belum_diimplementasi".into(),
        }
    }
}
