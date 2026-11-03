//! RFC 9457 `application/problem+json` (R-10, ADR-0001 section 5a.7).
//!
//! `kode` is a closed enum (`ProblemKode`); `type` is the stable anchor URL
//! into `docs/masalah.md`. The anchor set of that document must equal
//! `ProblemKode` — the equality is asserted by `api::tests::masalah_anchors`.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::enums::{ApiStatus, JenisObjek};

/// Closed machine-readable code. One anchor in `docs/masalah.md` per value;
/// `type` and `kode` are 1:1 (ADR-0001 section 5a.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKode {
    // Downstream (all) and admin (session-based 401).
    TidakTerautentikasi,
    /// Downstream 429: token bucket empty (hourly limit or 200 ms sync token).
    TokenDibatasi,
    /// 400 (malformed JSON / unsupported grant) and 422 (field validation).
    ValidasiGagal,
    /// 422: metadata fields do not match the endpoint contract.
    MetadataTidakValid,
    /// 400: `Idempotency-Key` sent on the sync path.
    IdempotencyKeyTidakDidukung,
    /// 422: same key, different fingerprint.
    IdempotencyKeyKonflik,
    /// 429: period quota exhausted (`kuota_habis` only for the period window).
    KuotaHabis,
    /// 429: sync portion or hourly limiter exhausted on the sync path.
    PorsiSyncHabis,
    /// 429: rejection limiter (burst/isolation) engaged.
    PenolakanBerlebih,
    /// 403 (endpoint) or 503 (pool/provider after enqueue).
    ObjekNonaktif,
    /// 409: cancel of a terminal request.
    TidakBisaDibatalkan,
    /// 503: Redis fail-closed or component down; always `Retry-After: 5`.
    LayananTidakTersedia,
    /// 502: retry window exhausted.
    RetryHabis,
    /// 502: non-retryable provider failure class.
    ProviderErrorPermanen,
    /// 504: sync call did not finish in time.
    TimeoutSync,
    /// 404: unknown resource.
    TidakDitemukan,
    // Admin.
    /// 401 on `POST /admin/v1/login`: wrong credentials.
    KredensialSalah,
    /// 429: login limiter engaged.
    LoginDibatasi,
    /// 403: write request from an unknown origin.
    OriginTidakValid,
    /// 403: role outside the operation's allowed set.
    AksesDitolak,
    /// 403: NIK or NIK fragment in log search parameters.
    PencarianNikDilarang,
    /// 409: webhook secret still used by queued deliveries.
    SecretDipakai,
    /// 409: credential referenced by an active endpoint.
    KredensialDicabut,
    /// 409: credential limit per client reached.
    BatasKredensial,
    /// 422: webhook domain outside the allowlist.
    WebhookDomainTidakDiizinkan,
    /// 422: webhook URL is not https.
    WebhookTidakHttps,
    /// 422: required metadata field missing from the provider config.
    MetadataWajibHilang,
    /// 422: auditor export above the 10,000-row limit.
    EksporTerlaluBesar,
}

impl ProblemKode {
    /// Every value, used by `docs/masalah.md` anchor checks and tests.
    pub const ALL: &'static [ProblemKode] = &[
        ProblemKode::TidakTerautentikasi,
        ProblemKode::TokenDibatasi,
        ProblemKode::ValidasiGagal,
        ProblemKode::MetadataTidakValid,
        ProblemKode::IdempotencyKeyTidakDidukung,
        ProblemKode::IdempotencyKeyKonflik,
        ProblemKode::KuotaHabis,
        ProblemKode::PorsiSyncHabis,
        ProblemKode::PenolakanBerlebih,
        ProblemKode::ObjekNonaktif,
        ProblemKode::TidakBisaDibatalkan,
        ProblemKode::LayananTidakTersedia,
        ProblemKode::RetryHabis,
        ProblemKode::ProviderErrorPermanen,
        ProblemKode::TimeoutSync,
        ProblemKode::TidakDitemukan,
        ProblemKode::KredensialSalah,
        ProblemKode::LoginDibatasi,
        ProblemKode::OriginTidakValid,
        ProblemKode::AksesDitolak,
        ProblemKode::PencarianNikDilarang,
        ProblemKode::SecretDipakai,
        ProblemKode::KredensialDicabut,
        ProblemKode::BatasKredensial,
        ProblemKode::WebhookDomainTidakDiizinkan,
        ProblemKode::WebhookTidakHttps,
        ProblemKode::MetadataWajibHilang,
        ProblemKode::EksporTerlaluBesar,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            ProblemKode::TidakTerautentikasi => "tidak_terautentikasi",
            ProblemKode::TokenDibatasi => "token_dibatasi",
            ProblemKode::ValidasiGagal => "validasi_gagal",
            ProblemKode::MetadataTidakValid => "metadata_tidak_valid",
            ProblemKode::IdempotencyKeyTidakDidukung => "idempotency_key_tidak_didukung",
            ProblemKode::IdempotencyKeyKonflik => "idempotency_key_konflik",
            ProblemKode::KuotaHabis => "kuota_habis",
            ProblemKode::PorsiSyncHabis => "porsi_sync_habis",
            ProblemKode::PenolakanBerlebih => "penolakan_berlebih",
            ProblemKode::ObjekNonaktif => "objek_nonaktif",
            ProblemKode::TidakBisaDibatalkan => "tidak_bisa_dibatalkan",
            ProblemKode::LayananTidakTersedia => "layanan_tidak_tersedia",
            ProblemKode::RetryHabis => "retry_habis",
            ProblemKode::ProviderErrorPermanen => "provider_error_permanen",
            ProblemKode::TimeoutSync => "timeout_sync",
            ProblemKode::TidakDitemukan => "tidak_ditemukan",
            ProblemKode::KredensialSalah => "kredensial_salah",
            ProblemKode::LoginDibatasi => "login_dibatasi",
            ProblemKode::OriginTidakValid => "origin_tidak_valid",
            ProblemKode::AksesDitolak => "akses_ditolak",
            ProblemKode::PencarianNikDilarang => "pencarian_nik_dilarang",
            ProblemKode::SecretDipakai => "secret_dipakai",
            ProblemKode::KredensialDicabut => "kredensial_dicabut",
            ProblemKode::BatasKredensial => "batas_kredensial",
            ProblemKode::WebhookDomainTidakDiizinkan => "webhook_domain_tidak_diizinkan",
            ProblemKode::WebhookTidakHttps => "webhook_tidak_https",
            ProblemKode::MetadataWajibHilang => "metadata_wajib_hilang",
            ProblemKode::EksporTerlaluBesar => "ekspor_terlalu_besar",
        }
    }

    /// RFC 9457 `title`. Human summary; details live in `docs/masalah.md`.
    pub fn title(&self) -> &'static str {
        match self {
            ProblemKode::TidakTerautentikasi => "Tidak terautentikasi",
            ProblemKode::TokenDibatasi => "Token dibatasi",
            ProblemKode::ValidasiGagal => "Validasi gagal",
            ProblemKode::MetadataTidakValid => "Metadata tidak valid",
            ProblemKode::IdempotencyKeyTidakDidukung => "Idempotency key tidak didukung",
            ProblemKode::IdempotencyKeyKonflik => "Konflik idempotency key",
            ProblemKode::KuotaHabis => "Kuota habis",
            ProblemKode::PorsiSyncHabis => "Porsi sync habis",
            ProblemKode::PenolakanBerlebih => "Penolakan berlebih",
            ProblemKode::ObjekNonaktif => "Objek nonaktif",
            ProblemKode::TidakBisaDibatalkan => "Tidak bisa dibatalkan",
            ProblemKode::LayananTidakTersedia => "Layanan tidak tersedia",
            ProblemKode::RetryHabis => "Retry habis",
            ProblemKode::ProviderErrorPermanen => "Kesalahan provider permanen",
            ProblemKode::TimeoutSync => "Timeout sync",
            ProblemKode::TidakDitemukan => "Tidak ditemukan",
            ProblemKode::KredensialSalah => "Kredensial salah",
            ProblemKode::LoginDibatasi => "Login dibatasi",
            ProblemKode::OriginTidakValid => "Origin tidak valid",
            ProblemKode::AksesDitolak => "Akses ditolak",
            ProblemKode::PencarianNikDilarang => "Pencarian NIK dilarang",
            ProblemKode::SecretDipakai => "Secret masih dipakai",
            ProblemKode::KredensialDicabut => "Kredensial masih dipakai",
            ProblemKode::BatasKredensial => "Batas kredensial tercapai",
            ProblemKode::WebhookDomainTidakDiizinkan => "Domain webhook tidak diizinkan",
            ProblemKode::WebhookTidakHttps => "Webhook bukan https",
            ProblemKode::MetadataWajibHilang => "Metadata wajib hilang",
            ProblemKode::EksporTerlaluBesar => "Ekspor terlalu besar",
        }
    }

    /// Stable `type` anchor (ADR-0001 section 5a.7).
    pub fn type_url(&self) -> String {
        format!(
            "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#{}",
            self.as_str()
        )
    }
}

/// Position inside the provider body template (U-9). Admin API only,
/// validation of the template only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Posisi {
    /// 1-based line in the body template.
    #[schema(minimum = 1)]
    pub baris: i64,
    /// 1-based column in the body template.
    #[schema(minimum = 1)]
    pub kolom: i64,
}

/// One failing input field. `errors[].kode` is a per-field validation code
/// from Addendum A section 6.1; `batas` carries only the numeric rule
/// parameter (never the rejected value). Template errors carry `posisi`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ValidasiError {
    /// Field name, or a JSON pointer for template errors.
    pub field: String,
    pub kode: String,
    /// Numeric rule parameter of the violated rule, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batas: Option<serde_json::Value>,
    /// Template position (U-9). Admin API and template errors only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posisi: Option<Posisi>,
}

/// RFC 9457 problem body shared by every 4xx/5xx. `detail` never contains
/// input values, NIK or secrets (R-16). Field names follow the contract:
/// Indonesian on the wire except the fixed RFC members.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Problem {
    /// Stable anchor `https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#<kode>`.
    #[serde(rename = "type")]
    pub type_: String,
    pub title: String,
    pub status: u16,
    pub kode: ProblemKode,
    /// Human explanation without sensitive values; absent when unneeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Failing fields; only on 422 `validasi_gagal` bodies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<ValidasiError>>,
    /// Kind of the inactive object; only on `objek_nonaktif`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jenis_objek: Option<JenisObjek>,
    /// Current API status; only on 409 `tidak_bisa_dibatalkan`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_api: Option<ApiStatus>,
    /// `queue_id` of the request the failure belongs to (502/504 bodies).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<Uuid>,
    /// RFC 6749 section 5.2 error for the token endpoint (U-3). Never sent
    /// with a 429.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Problem {
    fn new(status: u16, kode: ProblemKode) -> Self {
        Self {
            type_: kode.type_url(),
            title: kode.title().into(),
            status,
            kode,
            detail: None,
            errors: None,
            jenis_objek: None,
            status_api: None,
            request_id: None,
            error: None,
        }
    }

    /// Body of the temporary 501 handler stubs. The stubs are replaced by the
    /// real implementations without changing annotations (SHAA-175 note), so
    /// this code never appears in the generated YAML.
    pub fn stub() -> Self {
        let mut p = Self::new(501, ProblemKode::LayananTidakTersedia);
        p.title = "Belum diimplementasikan".into();
        p.detail = Some("Handler kontrak akan menggantikan stub ini.".into());
        p
    }
}

#[cfg(test)]
mod problem_tests {
    use super::*;

    /// `type` and `kode` are paired 1:1 and every `type` points at a stable
    /// anchor in `docs/masalah.md` (ADR-0001 section 5a.7).
    #[test]
    fn type_url_matches_kode() {
        for kode in ProblemKode::ALL {
            assert_eq!(
                kode.type_url(),
                format!(
                    "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#{}",
                    kode.as_str()
                )
            );
        }
        assert_eq!(ProblemKode::ALL.len(), 28);
    }
}
