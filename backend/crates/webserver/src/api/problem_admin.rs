//! RFC 9457 problem body for the admin API. Separate from the downstream
//! `Problem` so an admin-only code never leaks into the frozen external
//! contract (D-2): the admin body carries its own code set, its own
//! `ValidasiError` (with `posisi`), and admin-only fields.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::enums::JenisObjek;
use super::problem::{JumlahNamaTidakCocok, ProblemKodeAdmin, ValidasiErrorAdmin};

/// RFC 9457 problem body for every admin 4xx/5xx. `detail` never contains
/// input values, NIK or secrets (R-16).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProblemAdmin {
    /// Stable anchor `https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#<kode>`.
    #[serde(rename = "type")]
    pub type_: String,
    pub title: String,
    /// HTTP status, 400..=599.
    #[schema(minimum = 400, maximum = 599)]
    pub status: u16,
    pub kode: ProblemKodeAdmin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Failing fields; only on 422 `validasi_gagal` and `metadata_tidak_valid`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<ValidasiErrorAdmin>>,
    /// Withheld-name count (D6); same rule as the downstream body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jumlah_nama_tidak_cocok: Option<JumlahNamaTidakCocok>,
    /// Kind of the inactive object; only on `objek_nonaktif`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jenis_objek: Option<JenisObjek>,
    /// `request_id` the failure belongs to, when the operation names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<Uuid>,
}

impl ProblemAdmin {
    pub fn new(kode: ProblemKodeAdmin, status: u16) -> Self {
        Self {
            type_: kode.type_url(),
            title: kode.title().into(),
            status,
            kode,
            detail: None,
            errors: None,
            jumlah_nama_tidak_cocok: None,
            jenis_objek: None,
            request_id: None,
        }
    }

    /// Placeholder body of the 501 handler stubs.
    pub fn stub() -> Self {
        Self::new(ProblemKodeAdmin::ValidasiGagal, 501)
    }
}

impl Default for ProblemAdmin {
    fn default() -> Self {
        Self::stub()
    }
}
