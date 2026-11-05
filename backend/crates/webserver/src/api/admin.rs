//! Admin API (Web UI backend). Session cookie auth; role column per contract
//! section 8.1 (`platform_admin`, `auditor`, `operator`, `client_owner`).
//! Roles outside the column get 403 `akses_ditolak` and an `audit_akses` row.
//!
//! Handlers are 501 stubs; annotations are the reviewed contract and must not
//! change when stub bodies are replaced (SHAA-175 note).

use std::collections::BTreeMap;

use axum::{Json, http::StatusCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{OpenApi, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::enums::{
    BahasaUi, HasilWebhook, JendelaKuota, JenisKredensial, KelasErrorWebhook, Peran, RuleJenis,
    StatusKredensial,
};
use super::problem_admin::ProblemAdmin as Problem;

// ---------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------

/// Login body. Two consecutive failures count against the limiter (5 per 15
/// minutes per username).
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    #[schema(example = "auditor_01")]
    pub username: String,
    pub password: String,
}

/// Body of `POST /auth/mfa` (AC-10): exactly one of the two code kinds.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct KodeMfaRequest {
    /// Six-digit TOTP code.
    #[schema(example = "123456")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kode: Option<String>,
    /// One of the 10 recovery codes issued at enrollment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kode_pemulihan: Option<String>,
}

/// Body of `POST /auth/mfa/daftar` (AC-12): the registration code.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DaftarMfaRequest {
    /// The one-time code issued to this user; required, never reusable.
    pub kode_pendaftaran: String,
}

/// Body of `POST /auth/mfa/daftar/konfirmasi` (AC-13).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct KonfirmasiMfaRequest {
    #[schema(example = "123456")]
    pub kode_totp: String,
}

/// `POST /auth/mfa/daftar` response: shown once, never logged (AC-12).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MfaDaftar {
    /// `otpauth://totp/...` for the authenticator app (issuer `PoolRouter`).
    #[schema(
        example = "otpauth://totp/PoolRouter:auditor_01?secret=JBSWY3DPEHPK3PXP&issuer=PoolRouter"
    )]
    pub otpauth_uri: String,
    /// The same secret in base32, for manual entry.
    pub rahasia_base32: String,
}

/// `POST /auth/mfa/daftar/konfirmasi` response: the recovery codes, once (AC-13).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MfaKonfirmasi {
    /// 10 codes; each works once.
    pub kode_pemulihan: Vec<String>,
}

/// Current identity (U-1: "identitas, peran, client_id").
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Me {
    pub pengguna_id: Uuid,
    #[schema(example = "auditor_01")]
    pub username: String,
    /// Role granted to this user (contract section 8.1).
    pub peran: Vec<Peran>,
    /// Set only for `client_owner` sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bahasa_ui: Option<BahasaUi>,
    /// MFA state for this account (AC-14, MF3).
    pub mfa: MeMfa,
}

/// `Me.mfa` (AC-14): whether the step is required, whether it is enrolled, and
/// how many recovery codes remain.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeMfa {
    /// `true` when `POOLROUTER_MFA_WAJIB` is on and the caller's role is one
    /// the policy covers.
    pub wajib: bool,
    /// `true` once a TOTP secret is confirmed (`pengguna_mfa.status = 'aktif'`).
    pub aktif: bool,
    /// Unused recovery codes: 10 right after enrollment, 9 after one login.
    pub sisa_kode_pemulihan: i64,
}

/// Preference update (Addendum A section 7.2). No user id is accepted.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PreferensiUpdate {
    pub bahasa_ui: BahasaUi,
}

/// Session state (rev 5 section 3.6f). All fields required, UTC date-time.
/// Closed object: no extra members.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SesiStatus {
    pub idle_berakhir_pada: DateTime<Utc>,
    pub absolut_berakhir_pada: DateTime<Utc>,
    pub waktu_server: DateTime<Utc>,
}

/// Provider configuration body (`createProvider`/`updateProvider`, US-01/04).
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    /// Provider base URL (https).
    #[schema(example = "https://provider.example.go.id/api")]
    pub base_url: String,
    /// Body template. Placeholders `{{secret.X}}` must reference an existing
    /// secret; `{{endpoint.*}}` must have a literal in the endpoint config.
    #[schema(example = "{\"NIK\":\"{{input.NIK}}\",\"UID\":\"{{endpoint.USER_ID}}\"}")]
    pub body_template: String,
    /// Provider parameters plus their validation rules.
    pub parameters: Vec<ParameterRule>,
    /// Response schema used to map elements (US-05).
    #[schema(value_type = Object)]
    pub respons_skema: serde_json::Value,
    /// TLS trust setting for the provider call.
    #[schema(example = "sistem")]
    pub tls_trust: String,
    #[schema(example = "ringkas")]
    pub log_mode: String,
    /// Fixed 1 in F0; any other value is 422.
    #[schema(minimum = 1, maximum = 1, example = 1)]
    pub config_schema_version: i64,
    /// Reason; required on update (written to the config audit).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(min_length = 10, max_length = 500)]
    pub alasan: Option<String>,
}

/// One provider parameter with an optional validation rule (US-03). Also
/// serialized: the provider entity returns it for the edit form.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ParameterRule {
    /// Parameter name stored verbatim (`USER_ID`, `TRESHOLD`, `NAMA_LGKP`).
    #[schema(example = "USER_ID")]
    pub nama: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nilai: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<RuleValidasi>,
}

/// Validation rule for a provider parameter (BRD US-03; closed `jenis`). Also
/// serialized: the provider entity returns it for the edit form.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuleValidasi {
    pub jenis: RuleJenis,
    /// Regex pattern for `jenis = regex`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pola: Option<String>,
    /// Allowed values for `jenis = enum`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nilai_diizinkan: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panjang_min: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panjang_maks: Option<i64>,
    /// Date format for `jenis = format_tanggal`, e.g. `yyyy-MM-dd`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_tanggal: Option<String>,
}

/// Provider entity (list/get/create/update/activate responses). Every field of
/// [`ProviderConfig`] except `alasan` is present, so the Web UI edit form
/// (US-01/04) can be pre-filled from a read. Secrets never appear here:
/// `body_template` only carries `{{secret.X}}` names, never secret values
/// (R-16).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Provider {
    pub provider_id: Uuid,
    /// Provider base URL (https).
    #[schema(example = "https://provider.example.go.id/api")]
    pub base_url: String,
    /// Body template. Placeholders `{{secret.X}}` reference a secret by name;
    /// the value is never returned.
    #[schema(example = "{\"NIK\":\"{{input.NIK}}\",\"UID\":\"{{endpoint.USER_ID}}\"}")]
    pub body_template: String,
    /// Provider parameters plus their validation rules.
    pub parameters: Vec<ParameterRule>,
    /// Response schema used to map elements (US-05).
    #[schema(value_type = Object)]
    pub respons_skema: serde_json::Value,
    /// TLS trust setting for the provider call.
    #[schema(example = "sistem")]
    pub tls_trust: String,
    #[schema(example = "ringkas")]
    pub log_mode: String,
    /// Fixed 1 in F0.
    #[schema(minimum = 1, maximum = 1, example = 1)]
    pub config_schema_version: i64,
    #[schema(example = 2)]
    pub provider_config_version: i64,
    pub aktif: bool,
}

/// Secret list item. The value itself is never returned.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SecretRingkas {
    #[schema(example = "DUKCAPIL_PASSWORD")]
    pub nama: String,
    /// true: masked in UI.
    pub tersamar: bool,
    pub dipakai_oleh_provider_count: i64,
}

/// `putSecret` body; write-only value.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretNilai {
    /// Secret value; never echoed back, never logged (R-16).
    pub nilai: String,
}

/// Upstream pool configuration body (US-04/05; KP-01/KP-02 ranges enforced
/// server-side with 422, SHAA-280).
///
/// Ranges follow KP-01 (ADR-0001 section 5a "Parameter per upstream pool dan
/// rentangnya") exactly as decided for SHAA-280; the table lives in
/// `backend/openapi/DESIGN.md` section 3.2 and `KP01` in `api/tests.rs` asserts
/// it on the generated document.
///
/// Seven fields are **required with no default**, so the server never silently
/// picks a value the operator did not choose: `provider_id`, `jeda_min`,
/// `jeda_maks`, `ambang_berdampingan`, `kuota_harian`, `rate_limit_per_jam`, and
/// `porsi_sync`. The remaining parameters keep the KP-01 defaults.
///
/// Bounds that `minimum`/`maximum` cannot express are documented here and
/// enforced by [`UpstreamPoolConfig::validate`] (wiring into the handlers
/// follows SHAA-175):
/// - `retry_backoff_cap_detik` must be greater than or equal to
///   `retry_backoff_base_detik`;
/// - `jeda_maks` must be greater than or equal to `jeda_min`;
/// - both `jeda_min` and `jeda_maks` must be at most
///   `floor(3_600_000 / rate_limit_per_jam)` milliseconds. That is the mean
///   inter-request spacing the hourly rate limit allows, so a longer pause
///   would drop the pool below `rate_limit_per_jam` on its own. The `maximum`
///   of 60_000 below is only the static ceiling for `rate_limit_per_jam = 60`
///   and above; for example `rate_limit_per_jam = 36_000` caps both delays at
///   100 ms.
///
/// The `example` values are deliberately values a caller could send, not the
/// defaults.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpstreamPoolConfig {
    pub provider_id: Uuid,
    /// Daily upstream quota, at least 1 with no upper bound (range kept as it
    /// was before SHAA-280). Required: no default, so a create must state the
    /// quota it wants.
    #[schema(minimum = 1, example = 500)]
    pub kuota_harian: i64,
    /// Upstream rate limit per hour, at least 1 with no upper bound (range kept
    /// as it was before SHAA-280). Required: no default. Also caps
    /// `jeda_min`/`jeda_maks` at `floor(3_600_000 / rate_limit_per_jam)`
    /// milliseconds.
    #[schema(minimum = 1, example = 60)]
    pub rate_limit_per_jam: i64,
    /// Sync portion of the pool, 0.0..1.0 (KP-01: `porsi_sync_maks`). Required:
    /// no default.
    #[schema(minimum = 0, maximum = 1, example = 0.1)]
    pub porsi_sync: f64,
    /// Quota reservation margin, 0.001..0.200 (KP-01). Default 0.01.
    #[schema(minimum = 0.001, maximum = 0.2, default = 0.01, example = 0.05)]
    pub margin_kuota: f64,
    /// Attempts per request, 1..=5 (KP-01). Default 3.
    #[schema(minimum = 1, maximum = 5, default = 3, example = 2)]
    pub batas_percobaan: i64,
    /// Backoff base in seconds, 1..=300 (KP-01). Default 5.
    #[schema(minimum = 1, maximum = 300, default = 5, example = 2)]
    pub retry_backoff_base_detik: i64,
    /// Backoff cap in seconds, `retry_backoff_base_detik`..=3600. The lower
    /// bound is per request, not a fixed `minimum`: `retry_backoff_cap_detik`
    /// must be greater than or equal to `retry_backoff_base_detik` (cross-field
    /// 422, see [`UpstreamPoolConfig::validate`]). Default 300.
    #[schema(minimum = 1, maximum = 3600, default = 300, example = 60)]
    pub retry_backoff_cap_detik: i64,
    /// Retry budget as a ratio of `kuota_harian`, 0.0..=0.5 (KP-01).
    /// Default 0.1.
    #[schema(minimum = 0, maximum = 0.5, default = 0.1, example = 0.5)]
    pub retry_rasio: f64,
    /// Retry budget floor per hour, 0..=10_000 (KP-01). Default 20.
    #[schema(minimum = 0, maximum = 10_000, default = 20, example = 10)]
    pub retry_min_per_jam: i64,
    pub jendela_kuota: JendelaKuota,
    /// Hour of the period reset (0..=23), interpreted in `reset_kuota_zona`.
    #[schema(minimum = 0, maximum = 23, default = 0, example = 0)]
    pub reset_kuota_jam: i64,
    /// IANA time zone; the offset is pinned to this zone (KP-02).
    #[schema(default = "Asia/Jakarta", example = "Asia/Jayapura")]
    pub reset_kuota_zona: String,
    /// Minimum inter-request delay in milliseconds, 0..=60_000 and at most
    /// `floor(3_600_000 / rate_limit_per_jam)` (cross-field 422). Required: no
    /// default.
    #[schema(minimum = 0, maximum = 60_000, example = 250)]
    pub jeda_min: i64,
    /// Maximum inter-request delay in milliseconds, 0..=60_000 and at most
    /// `floor(3_600_000 / rate_limit_per_jam)`, and never below `jeda_min`
    /// (cross-field 422). Required: no default.
    #[schema(minimum = 0, maximum = 60_000, example = 1500)]
    pub jeda_maks: i64,
    /// Concurrency threshold (`ambang berdampingan`, US-22), at least 1 with no
    /// upper bound. Required: no default.
    #[schema(minimum = 1, example = 8)]
    pub ambang_berdampingan: i64,
    /// Reason; required on update, and required with `kalender` (source of
    /// certainty of the calendar window).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(min_length = 10, max_length = 500)]
    pub alasan: Option<String>,
}

/// One rejected field of a pure validator, in the shape the admin 422 body needs
/// (Addendum A §6.1, AC-7): the field, the validation code, and the numeric
/// rule parameter when the rule has one. Never the input value.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldError {
    /// Field name exactly as it appears in the JSON body and the OpenAPI schema,
    /// so a client can point at the offending member.
    pub field: &'static str,
    /// Validation code, e.g. `rentang`, `wajib`, `format`.
    pub kode: &'static str,
    /// Numeric parameter of the violated rule, if any (always a number, never
    /// the rejected input).
    pub batas: Option<f64>,
}

impl FieldError {
    fn new(field: &'static str, kode: &'static str) -> Self {
        Self {
            field,
            kode,
            batas: None,
        }
    }

    fn dengan_batas(mut self, batas: f64) -> Self {
        self.batas = Some(batas);
        self
    }
}

impl UpstreamPoolConfig {
    /// Mean inter-request spacing, in milliseconds, that `rate_limit_per_jam`
    /// allows: `floor(3_600_000 / rate_limit_per_jam)`. It is the cross-field
    /// ceiling for `jeda_min` and `jeda_maks` (KP-01, SHAA-280).
    ///
    /// Returns `None` when `rate_limit_per_jam` is not positive, because then the
    /// field's own `minimum` bound already fails and there is no meaningful
    /// ceiling to compare against.
    #[must_use]
    pub fn batas_jeda_dari_rate_limit(&self) -> Option<i64> {
        (self.rate_limit_per_jam > 0).then(|| 3_600_000 / self.rate_limit_per_jam)
    }

    /// Pure validation of every KP-01 bound, including the cross-field rules that
    /// a JSON Schema `minimum`/`maximum` cannot express (SHAA-280 AC 3).
    ///
    /// Returns every violation found, not just the first, so a client can fix the
    /// whole body in one round trip. An empty `Ok(())` means the body is valid.
    /// Field bounds are checked here as well as in the schema, so this function is
    /// self-contained: a caller can rely on it alone.
    ///
    /// No I/O and no environment access, which is what makes it unit-testable and
    /// what later lets the handler call it unchanged (SHAA-175).
    pub fn validate(&self) -> Result<(), Vec<FieldError>> {
        let mut errors = Vec::new();

        // Single-field ranges (KP-01 + Sisipan H for the jeda triad).
        rentang_min(&mut errors, "kuota_harian", self.kuota_harian, 1);
        rentang_min(
            &mut errors,
            "rate_limit_per_jam",
            self.rate_limit_per_jam,
            1,
        );
        rentang(&mut errors, "batas_percobaan", self.batas_percobaan, 1, 5);
        rentang(
            &mut errors,
            "retry_backoff_base_detik",
            self.retry_backoff_base_detik,
            1,
            300,
        );
        rentang(
            &mut errors,
            "retry_backoff_cap_detik",
            self.retry_backoff_cap_detik,
            1,
            3_600,
        );
        rentang_pecahan(&mut errors, "retry_rasio", self.retry_rasio, 0.0, 0.5);
        rentang(
            &mut errors,
            "retry_min_per_jam",
            self.retry_min_per_jam,
            0,
            10_000,
        );
        rentang(&mut errors, "jeda_min", self.jeda_min, 0, 60_000);
        rentang(&mut errors, "jeda_maks", self.jeda_maks, 0, 60_000);
        // `ambang_berdampingan` has no upper bound (Shu, SHAA-280).
        rentang_min(
            &mut errors,
            "ambang_berdampingan",
            self.ambang_berdampingan,
            1,
        );
        rentang_pecahan(&mut errors, "margin_kuota", self.margin_kuota, 0.001, 0.2);
        rentang(&mut errors, "reset_kuota_jam", self.reset_kuota_jam, 0, 23);
        rentang_pecahan(&mut errors, "porsi_sync", self.porsi_sync, 0.0, 1.0);

        // Cross-field: the retry cap never undershoots the base.
        if self.retry_backoff_cap_detik < self.retry_backoff_base_detik {
            errors.push(
                FieldError::new("retry_backoff_cap_detik", "rentang")
                    .dengan_batas(self.retry_backoff_base_detik as f64),
            );
        }

        // Cross-field: the maximum delay never undershoots the minimum delay.
        if self.jeda_maks < self.jeda_min {
            errors.push(FieldError::new("jeda_maks", "rentang").dengan_batas(self.jeda_min as f64));
        }

        // Cross-field: both delays respect the interval the hourly rate limit
        // leaves between requests.
        if let Some(batas) = self.batas_jeda_dari_rate_limit() {
            if self.jeda_min > batas {
                errors.push(FieldError::new("jeda_min", "rentang").dengan_batas(batas as f64));
            }
            if self.jeda_maks > batas {
                errors.push(FieldError::new("jeda_maks", "rentang").dengan_batas(batas as f64));
            }
        }

        // One error per field: static bounds are checked first, so a field that
        // is already out of its own range is not reported a second time by a
        // cross-field rule that the same bad value also breaks.
        let mut dilaporkan: Vec<&'static str> = Vec::new();
        errors.retain(|e| {
            if dilaporkan.contains(&e.field) {
                false
            } else {
                dilaporkan.push(e.field);
                true
            }
        });

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// `batas` carries the bound the value broke, so the 422 says which rule
/// parameter failed without repeating the rejected value (AC-7).
fn rentang(errors: &mut Vec<FieldError>, field: &'static str, nilai: i64, min: i64, max: i64) {
    if nilai < min {
        errors.push(FieldError::new(field, "rentang").dengan_batas(min as f64));
    } else if nilai > max {
        errors.push(FieldError::new(field, "rentang").dengan_batas(max as f64));
    }
}

fn rentang_min(errors: &mut Vec<FieldError>, field: &'static str, nilai: i64, min: i64) {
    if nilai < min {
        errors.push(FieldError::new(field, "rentang").dengan_batas(min as f64));
    }
}

fn rentang_pecahan(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    nilai: f64,
    min: f64,
    max: f64,
) {
    // NaN is not expressible in JSON, but it is in Rust: reject it rather than
    // letting a comparison that is always false slip through.
    if !nilai.is_finite() || nilai < min {
        errors.push(FieldError::new(field, "rentang").dengan_batas(min));
    } else if nilai > max {
        errors.push(FieldError::new(field, "rentang").dengan_batas(max));
    }
}

/// Upstream pool entity (list/get/create/update/activate responses). Every field
/// of [`UpstreamPoolConfig`] except `alasan` is present, so the Web UI edit form
/// (US-04/05) can be pre-filled from a read: the `PUT` is a full replacement and
/// a missing field would be submitted as a default, silently overwriting the
/// current configuration.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UpstreamPool {
    pub upstream_pool_id: Uuid,
    pub provider_id: Uuid,
    // Stored values, so no `default` here: a read never reports what a create
    // would have used (SHAA-280 scopes the KP-01 ranges/defaults to the body).
    #[schema(minimum = 1, example = 500)]
    pub kuota_harian: i64,
    #[schema(minimum = 1, example = 60)]
    pub rate_limit_per_jam: i64,
    /// Sync portion of the pool, 0.0..1.0.
    #[schema(minimum = 0, maximum = 1, example = 0.1)]
    pub porsi_sync: f64,
    /// 0.001..0.200 (KP-01).
    #[schema(minimum = 0.001, maximum = 0.2, example = 0.05)]
    pub margin_kuota: f64,
    /// 1..5 attempts (KP-01).
    #[schema(minimum = 1, maximum = 5, example = 3)]
    pub batas_percobaan: i64,
    #[schema(example = 2)]
    pub retry_backoff_base_detik: i64,
    #[schema(example = 60)]
    pub retry_backoff_cap_detik: i64,
    #[schema(example = 0.5)]
    pub retry_rasio: f64,
    #[schema(example = 10)]
    pub retry_min_per_jam: i64,
    pub jendela_kuota: JendelaKuota,
    /// Hour of the period reset (0..23), interpreted in `reset_kuota_zona`.
    #[schema(minimum = 0, maximum = 23, example = 0)]
    pub reset_kuota_jam: i64,
    /// IANA time zone; the offset is pinned to this zone (KP-02).
    #[schema(example = "Asia/Jakarta")]
    pub reset_kuota_zona: String,
    /// Minimum inter-request delay in milliseconds.
    #[schema(example = 200)]
    pub jeda_min: i64,
    /// Maximum inter-request delay in milliseconds.
    #[schema(example = 800)]
    pub jeda_maks: i64,
    /// Concurrency threshold (`ambang berdampingan`).
    #[schema(example = 4)]
    pub ambang_berdampingan: i64,
    pub aktif: bool,
}

/// Downstream pool create/update body. Mode is not here: the path decides it
/// (a downstream pool serves both `/sync` and `/async`). `prioritas` is derived
/// by the allocation and never writable.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DownstreamPoolConfig {
    /// Pool name; unique across downstream pools (schema `NOT NULL UNIQUE`).
    #[schema(example = "BGN MPM")]
    pub nama: String,
    /// Queue TTL in seconds; default 86_400 (24 hours). Column
    /// `ttl_antrean_detik`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 1, example = 86400)]
    pub ttl_antrean_detik: Option<i64>,
    /// The upstream pools serving this downstream pool. Exactly one in F0.
    #[schema(min_items = 1, max_items = 1)]
    pub upstream_pool_ids: Vec<Uuid>,
    /// Reason; required on update.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(min_length = 10, max_length = 500)]
    pub alasan: Option<String>,
}

/// Downstream pool entity. `prioritas` is derived by the allocation: read-only
/// here, always 1 in F0.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DownstreamPool {
    pub downstream_pool_id: Uuid,
    pub nama: String,
    pub ttl_antrean_detik: i64,
    pub upstream_pool_ids: Vec<Uuid>,
    /// Derived priority; read-only, always 1 in F0.
    #[schema(example = 1, read_only)]
    pub prioritas: i64,
    pub aktif: bool,
}

/// Client configuration body (US-02; L-04 limiter ranges).
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    #[schema(example = "BGN MPM Production")]
    pub nama: String,
    /// 1..=1000, default 20 (L-04).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 1, maximum = 1000, example = 20)]
    pub limiter_penolakan_burst: Option<i64>,
    /// 0.10..=100.00 per second, default 1.00 (L-04).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0.1, maximum = 100.0, example = 1.0)]
    pub limiter_penolakan_isi_ulang_per_detik: Option<f64>,
    /// Reason; required when changing the limiter.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(min_length = 10, max_length = 500)]
    pub alasan: Option<String>,
}

/// Client entity.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Client {
    pub client_id: Uuid,
    #[schema(example = "BGN MPM Production")]
    pub nama: String,
    pub limiter_penolakan_burst: i64,
    pub limiter_penolakan_isi_ulang_per_detik: f64,
    pub aktif: bool,
}

/// `setClientEndpoints` body (U-7 shape).
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ClientEndpoints {
    /// Complete assignment; replaces the previous set. An empty set revokes
    /// every assignment, so the client then receives 404 from every endpoint.
    #[schema(min_items = 0)]
    pub endpoint_ids: Vec<Uuid>,
}

/// Credential entity (list response; no hash is ever returned).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Credential {
    pub credential_id: Uuid,
    pub jenis: JenisKredensial,
    pub status: StatusKredensial,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kedaluwarsa_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dicabut_at: Option<DateTime<Utc>>,
}

/// `createCredential` body (contract section 3.1b). Default and maximum
/// expiry 365 days.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CredentialCreate {
    pub jenis: JenisKredensial,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kedaluwarsa_at: Option<DateTime<Utc>>,
}

/// Secret response; returned exactly once, `no-store`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CredentialSecret {
    pub credential_id: Uuid,
    pub client_id: Uuid,
    /// Client secret: 256-bit base64url. API key: the full string
    /// `pr_<credential_id 32 hex>_<secret 256-bit base64url>` (ADR-0001 5a.5,
    /// SHAA-198 U-13). Never returned again (contract 3.1b).
    #[schema(
        example = "pr_0193b8a27c4e7d108a3f5e6b7c8d9e0f_2fJ8N0qk0e6h7h0F1D2J3k4L5m6N7o8P9Q0r1S2t3U4"
    )]
    pub secret: String,
    pub kedaluwarsa_at: Option<DateTime<Utc>>,
}

/// Webhook configuration view; the secret itself is never returned.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WebhookConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
    pub aktif: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_versi: Option<i64>,
}

/// `putWebhookConfig` body (US-06). Domain must be allowlisted and https;
/// checked on save and on send.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WebhookConfigPut {
    #[schema(example = "https://mpm.example.go.id/webhook/poolrouter")]
    pub webhook_url: String,
    pub aktif: bool,
}

/// Rotated webhook secret; returned exactly once (F-5). Creating the first
/// secret sets `secret_versi: 1`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WebhookSecret {
    pub secret_versi: i64,
    /// `whsec_` + 32-byte base64 (Standard Webhooks).
    #[schema(example = "whsec_2fJ8N0qk0e6h7h0F1D2J3k4L5m6N7o8P9Q0r1S2t3U4v")]
    pub secret: String,
}

/// One webhook delivery (US-06).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WebhookLog {
    pub request_id: Uuid,
    pub sent_at: DateTime<Utc>,
    pub hasil: HasilWebhook,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_class: Option<KelasErrorWebhook>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_versi: Option<i64>,
}

/// One allowlist entry (US-06).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AllowlistEntry {
    #[schema(example = "mpm.example.go.id")]
    pub domain: String,
    pub aktif: bool,
}

/// `putWebhookAllowlistEntry` body.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AllowlistPut {
    pub aktif: bool,
}

/// Endpoint entity.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Endpoint {
    pub endpoint_id: Uuid,
    pub provider_id: Uuid,
    pub downstream_pool_id: Uuid,
    /// Literal parameters, including `TRESHOLD` (BGN mandate).
    pub parameter_literal: BTreeMap<String, String>,
    #[schema(example = "ringkas")]
    pub log_mode: String,
    /// Metadata field definitions; Dukcapil endpoints must include
    /// `status_penerima_manfaat` (M-R1).
    pub metadata_fields: Vec<MetadataFieldDef>,
    /// Fixed 1 in F0.
    #[schema(minimum = 1, maximum = 1, example = 1)]
    pub config_schema_version: i64,
    #[schema(example = 3)]
    pub endpoint_config_version: i64,
    pub aktif: bool,
}

/// Endpoint configuration body (US-15; M-R1).
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EndpointConfig {
    pub provider_id: Uuid,
    pub downstream_pool_id: Uuid,
    /// Literal parameters, including `TRESHOLD` (BGN mandate).
    pub parameter_literal: BTreeMap<String, String>,
    #[schema(example = "ringkas")]
    pub log_mode: String,
    /// Metadata field definitions; Dukcapil endpoints must include
    /// `status_penerima_manfaat` (M-R1).
    pub metadata_fields: Vec<MetadataFieldDef>,
    /// Fixed 1 in F0.
    #[schema(minimum = 1, maximum = 1, example = 1)]
    pub config_schema_version: i64,
    /// Reason; required on update.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(min_length = 10, max_length = 500)]
    pub alasan: Option<String>,
}

/// Metadata field definition (US-15). Also serialized: the endpoint entity
/// returns it so the edit form can be pre-filled.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MetadataFieldDef {
    #[schema(example = "status_penerima_manfaat")]
    pub nama_field: String,
    #[schema(example = "string")]
    pub tipe: String,
    pub wajib: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nilai_diizinkan: Option<Vec<String>>,
    /// Validation rule reference, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aturan_validasi: Option<String>,
    /// Classification (`rahasia` fields are never logged or sent to callbacks).
    #[schema(example = "internal")]
    pub klasifikasi: String,
    /// Whether the field is sent to the data-balikan callback.
    pub kirim_ke_data_balikan: bool,
    pub deskripsi: String,
}

/// K-20 preview (section 3a; no request_id, NIK or payload). Upstream pool
/// previews add the last-active-upstream detail (section 3a.10).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PratinjauK20 {
    pub akan_cancelled: i64,
    pub akan_failed: i64,
    pub dihitung_pada: DateTime<Utc>,
    /// Upstream pool previews only: downstream pools whose last active
    /// upstream is this pool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_aktif_terakhir_untuk: Option<Vec<UpstreamAktifTerakhir>>,
    /// Upstream pool previews only: number of downstream pools allocated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dipakai_oleh_downstream_pool_count: Option<i64>,
}

/// Last-active-upstream detail in a preview (section 3a.10).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UpstreamAktifTerakhir {
    pub downstream_pool_id: Uuid,
    pub nama: String,
}

/// K-20 execution result; the same numbers go to the config audit.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HasilK20 {
    pub jumlah_cancelled: i64,
    pub jumlah_failed: i64,
    pub dilakukan_pada: DateTime<Utc>,
}

/// Reason body for deactivate/revoke (K-20).
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AlasanK20 {
    #[schema(
        min_length = 10,
        max_length = 500,
        example = "Kontrak client berakhir."
    )]
    pub alasan: String,
}

/// One quota tracker row (US-12; KP-07: numbers from the DB, not Redis).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KuotaTracker {
    pub upstream_pool_id: Uuid,
    pub periode_mulai: DateTime<Utc>,
    pub periode_selesai: DateTime<Utc>,
    pub kuota_harian: i64,
    pub kuota_efektif: i64,
    pub terpakai_periode: i64,
    pub sisa_periode: i64,
    pub porsi_sync_maks: i64,
    pub terpakai_sync_jam_berjalan: i64,
    pub terpakai_jam_berjalan: i64,
    pub rate_limit_per_jam: i64,
    pub dihitung_pada: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Log, audit akses, ekspor schemas (US-14, 16; ADR-0003 section 3.6)
// ---------------------------------------------------------------------------

/// `searchLog` body (POST so filters stay out of the access log). Time range
/// required. NIK or NIK fragments in any parameter are 403
/// `pencarian_nik_dilarang` (US-16).
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LogCariRequest {
    pub dari: DateTime<Utc>,
    pub sampai: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kode_transaksi: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<Uuid>,
    /// `diizinkan` | `ditolak`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_akses: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kursor: Option<String>,
}

/// One log row (US-14). `nik` is masked by the server for OP; never the
/// client (U-1).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LogBaris {
    /// Masked for OP: `************1234`; full for auditor searches.
    #[schema(example = "************1234")]
    pub nik: String,
    pub requested_at: DateTime<Utc>,
    /// `diizinkan` | `ditolak`.
    pub status_akses: String,
    pub kode_transaksi: String,
    pub queue_id: Uuid,
    pub client_id: Uuid,
    /// Client name joined by the server (F-4); UI needs it for pagination.
    #[schema(example = "BGN MPM Production")]
    pub client_nama: String,
}

/// Paginated log result.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LogPage {
    pub baris: Vec<LogBaris>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kursor: Option<String>,
    /// true: older rows live in cold storage and are not in this page.
    pub di_luar_retensi_online: bool,
}

/// Full one-entry log row (US-14). Masked for OP.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LogEntry {
    #[schema(example = "************1234")]
    pub nik: String,
    pub requested_at: DateTime<Utc>,
    pub status_akses: String,
    pub kode_transaksi: String,
    pub queue_id: Uuid,
    pub client_id: Uuid,
    #[schema(example = "BGN MPM Production")]
    pub client_nama: String,
}

/// `searchLogByNik` body (auditor only; exact 16-digit NIK, no wildcard or
/// prefix). One `audit_akses` `cari_nik` row per call, including page turns
/// and empty results.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NikCariRequest {
    #[schema(pattern = "^[0-9]{16}$", example = "3175000000000001")]
    pub nik: String,
    pub dari: DateTime<Utc>,
    pub sampai: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kursor: Option<String>,
    #[schema(min_length = 10, max_length = 500, example = "Audit aduan W-2026-07.")]
    pub alasan: String,
}

/// `openPayload` body (auditor only; exactly one of `request_id` or
/// `kode_transaksi`).
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PayloadOpenRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kode_transaksi: Option<String>,
    #[schema(
        min_length = 10,
        max_length = 500,
        example = "Riset banding aduan W-2026-07."
    )]
    pub alasan: String,
}

/// Payload view opened by an auditor (US-16). One `lihat_payload` row per
/// opening; no cache.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PayloadView {
    pub request_id: Uuid,
    pub kode_transaksi: String,
    pub status_akses: String,
    pub requested_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hasil: Option<serde_json::Value>,
}

/// `exportLogAuditor` body (R-18). Above 10,000 rows: 422
/// `ekspor_terlalu_besar` without a file and with a `ditolak` audit row.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EksporRequest {
    #[schema(pattern = "^[0-9]{16}$", example = "3175000000000001")]
    pub nik: String,
    pub dari: DateTime<Utc>,
    pub sampai: DateTime<Utc>,
    #[schema(min_length = 10, max_length = 500, example = "Ekspor audit Q4 2026.")]
    pub alasan: String,
}

/// `searchAuditAkses` body (auditor only; range max 31 days). Rows carry no
/// `nik_hmac`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditCariRequest {
    pub dari: DateTime<Utc>,
    pub sampai: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pengguna_id: Option<Uuid>,
    /// Action filter (`login`, `cari_log`, `lihat_payload`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aksi: Option<String>,
    /// `berhasil` | `ditolak`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hasil: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kursor: Option<String>,
}

/// One `audit_akses` row (no `nik_hmac`).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AuditAksesBaris {
    pub audit_id: Uuid,
    pub dilakukan_pada: DateTime<Utc>,
    pub pengguna_id: Uuid,
    #[schema(example = "lihat_payload")]
    pub aksi: String,
    /// `berhasil` | `ditolak`.
    pub hasil: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alasan: Option<String>,
}

/// Paginated audit akses result.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AuditAksesPage {
    pub baris: Vec<AuditAksesBaris>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kursor: Option<String>,
}

// ---------------------------------------------------------------------------
// Reusable problem responses (R-10: one example per kode, R-16 no-store)
// ---------------------------------------------------------------------------

/// 401 admin: no session (or session expired). On `login` it is
/// `kredensial_salah` without distinguishing username/password (U-1).
#[derive(utoipa::ToResponse)]
#[response(description = "Tanpa sesi (401) atau kredensial login salah; penyebab tidak dibedakan (U-1)", content_type = "application/problem+json", headers(
    ("Cache-Control" = String, description = "Selalu no-store (R-16)")
), examples(
    ("tidak_terautentikasi" = (summary = "Tanpa sesi atau sesi kedaluwarsa", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#tidak_terautentikasi", "title": "Tidak terautentikasi", "status": 401, "kode": "tidak_terautentikasi"}))),
    ("kredensial_salah" = (summary = "Username atau password salah (tidak dibedakan)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#kredensial_salah", "title": "Kredensial salah", "status": 401, "kode": "kredensial_salah"})))
))]
pub struct Problem401Admin(pub Problem);

/// 403 admin: role outside the allowed set, unknown Origin on write, or NIK
/// search outside the auditor path.
#[derive(utoipa::ToResponse)]
#[response(description = "Peran di luar kolom (akses_ditolak), Origin tidak sah pada tulis (origin_tidak_valid), atau pencarian NIK di luar jalur auditor (pencarian_nik_dilarang)", content_type = "application/problem+json", headers(
    ("Cache-Control" = String, description = "Selalu no-store (R-16)")
), examples(
    ("akses_ditolak" = (summary = "Peran tidak termasuk kolom Peran operasi", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#akses_ditolak", "title": "Akses ditolak", "status": 403, "kode": "akses_ditolak"}))),
    ("origin_tidak_valid" = (summary = "Tulis tanpa Origin yang sah", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#origin_tidak_valid", "title": "Origin tidak valid", "status": 403, "kode": "origin_tidak_valid"}))),
    ("pencarian_nik_dilarang" = (summary = "Parameter memuat NIK atau potongannya", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#pencarian_nik_dilarang", "title": "Pencarian NIK dilarang", "status": 403, "kode": "pencarian_nik_dilarang"})))
))]
pub struct Problem403Admin(pub Problem);

/// 400 admin: malformed JSON.
#[derive(utoipa::ToResponse)]
#[response(description = "Body bukan JSON yang sah", content_type = "application/problem+json", headers(
    ("Cache-Control" = String, description = "Selalu no-store (R-16)")
), examples(
    ("validasi_gagal" = (summary = "JSON tidak dapat diurai", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#validasi_gagal", "title": "Validasi gagal", "status": 400, "kode": "validasi_gagal"})))
))]
pub struct Problem400Admin(pub Problem);

/// 404 admin: unknown id or code.
#[derive(utoipa::ToResponse)]
#[response(description = "Objek tidak ditemukan", content_type = "application/problem+json", headers(
    ("Cache-Control" = String, description = "Selalu no-store (R-16)")
), examples(
    ("tidak_ditemukan" = (summary = "Id atau kode transaksi tidak dikenal", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#tidak_ditemukan", "title": "Tidak ditemukan", "status": 404, "kode": "tidak_ditemukan"})))
))]
pub struct Problem404Admin(pub Problem);

/// 422 admin. Template errors carry `posisi {baris, kolom}` (U-9); numeric
/// rule violations carry `batas` with the rule parameter only (R-16).
#[derive(utoipa::ToResponse)]
#[response(description = "Validasi gagal; termasuk template tidak seimbang (dengan posisi), {{secret.X}} tidak ada (nama secret di errors), config_schema_version salah, webhook tidak https / di luar allowlist, metadata wajib hilang (M-R1), atau ekspor di atas 10.000 baris", content_type = "application/problem+json", headers(
    ("Cache-Control" = String, description = "Selalu no-store (R-16)")
), examples(
    ("validasi_gagal" = (summary = "Template body tidak seimbang; posisi menunjuk baris/kolom (U-9)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#validasi_gagal", "title": "Validasi gagal", "status": 422, "kode": "validasi_gagal", "errors": [{"field": "/body_template", "kode": "template_tidak_seimbang", "posisi": {"baris": 2, "kolom": 18}}]}))),
    ("metadata_wajib_hilang" = (summary = "Endpoint Dukcapil tanpa status_penerima_manfaat (M-R1)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#metadata_wajib_hilang", "title": "Metadata wajib hilang", "status": 422, "kode": "metadata_wajib_hilang", "errors": [{"field": "/metadata_fields", "kode": "wajib_hilang"}]}))),
    ("webhook_tidak_https" = (summary = "URL webhook bukan https", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#webhook_tidak_https", "title": "Webhook bukan https", "status": 422, "kode": "webhook_tidak_https"}))),
    ("webhook_domain_tidak_diizinkan" = (summary = "Domain di luar allowlist", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#webhook_domain_tidak_diizinkan", "title": "Domain webhook tidak diizinkan", "status": 422, "kode": "webhook_domain_tidak_diizinkan"}))),
    ("ekspor_terlalu_besar" = (summary = "Lebih dari 10.000 baris; tanpa berkas, baris audit ditolak (R-18)", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#ekspor_terlalu_besar", "title": "Ekspor terlalu besar", "status": 422, "kode": "ekspor_terlalu_besar"})))
))]
pub struct Problem422Admin(pub Problem);

/// 429 admin: login limiter (5 failures per 15 minutes per username).
#[derive(utoipa::ToResponse)]
#[response(description = "Limiter login aktif", content_type = "application/problem+json", headers(
    ("Retry-After" = String, description = "Detik hingga percobaan berikutnya (GCRA)"),
    ("Cache-Control" = String, description = "Selalu no-store (R-16)")
), examples(
    ("login_dibatasi" = (summary = "5 kali gagal dalam 15 menit", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#login_dibatasi", "title": "Login dibatasi", "status": 429, "kode": "login_dibatasi"})))
))]
pub struct Problem429Admin(pub Problem);

/// 409 admin: secret or credential still referenced, or credential limit
/// reached.
#[derive(utoipa::ToResponse)]
#[response(description = "Secret masih dipakai provider aktif (secret_dipakai), kredensial masih dipakai endpoint (kredensial_dicabut), atau batas dua kredensial aktif per client (batas_kredensial)", content_type = "application/problem+json", headers(
    ("Cache-Control" = String, description = "Selalu no-store (R-16)")
), examples(
    ("secret_dipakai" = (summary = "Daftar provider aktif yang memakai secret ada di detail", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#secret_dipakai", "title": "Secret masih dipakai", "status": 409, "kode": "secret_dipakai", "detail": "Masih dipakai 1 provider aktif."}))),
    ("kredensial_dicabut" = (summary = "Kredensial masih dipakai endpoint aktif; U-54b: aktifkan ulang selalu 409", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#kredensial_dicabut", "title": "Kredensial masih dipakai", "status": 409, "kode": "kredensial_dicabut"}))),
    ("batas_kredensial" = (summary = "Sudah ada dua kredensial aktif", value = json!({"type": "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#batas_kredensial", "title": "Batas kredensial tercapai", "status": 409, "kode": "batas_kredensial"})))
))]
pub struct Problem409Admin(pub Problem);

// ---------------------------------------------------------------------------
// Handlers: 501 stubs
// ---------------------------------------------------------------------------

fn stub() -> (StatusCode, Json<Problem>) {
    (StatusCode::NOT_IMPLEMENTED, Json(Problem::stub()))
}

/// Login (ADR-0003 section 3.6). Success sets `__Host-pr_session` cookie.
#[utoipa::path(
    post,
    path = "/admin/v1/auth/login",
    operation_id = "login",
    tag = "admin",
    request_body(content = LoginRequest, content_type = "application/json"),
    responses(
        (status = 204, description = "Login berhasil; cookie sesi diterbitkan", headers(
            ("Set-Cookie" = String, description = "__Host-pr_session=...; HttpOnly; Secure; SameSite=Strict; Path=/"),
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 429, response = Problem429Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(())
)]
async fn login(axum::Json(_body): axum::Json<LoginRequest>) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Second login step: TOTP or a recovery code (ADR-0006 §9, AC-10).
///
/// Reads the `__Host-pr_mfa` pre-session cookie; on success sets
/// `__Host-pr_session` and clears the pre-session cookie.
#[utoipa::path(
    post,
    path = "/admin/v1/auth/mfa",
    operation_id = "mfaVerifikasi",
    tag = "admin",
    request_body(content = KodeMfaRequest, content_type = "application/json"),
    responses(
        (status = 204, description = "Kode diterima; cookie sesi diterbitkan", headers(
            ("Set-Cookie" = String, description = "__Host-pr_session=...; HttpOnly; Secure; SameSite=Strict; Path=/"),
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 429, response = Problem429Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(())
)]
async fn mfa_verifikasi(
    axum::Json(_body): axum::Json<KodeMfaRequest>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Start MFA enrollment: returns the `otpauth://` URI once (AC-12).
#[utoipa::path(
    post,
    path = "/admin/v1/auth/mfa/daftar",
    operation_id = "mfaDaftar",
    tag = "admin",
    request_body(content = DaftarMfaRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "URI otpauth dan secret base32; ditampilkan sekali", body = MfaDaftar, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin)
    ),
    security(())
)]
async fn mfa_daftar(
    axum::Json(_body): axum::Json<DaftarMfaRequest>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Confirm enrollment with one TOTP code; returns the 10 recovery codes (AC-13).
#[utoipa::path(
    post,
    path = "/admin/v1/auth/mfa/daftar/konfirmasi",
    operation_id = "mfaKonfirmasi",
    tag = "admin",
    request_body(content = KonfirmasiMfaRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "MFA aktif; kode pemulihan dan cookie sesi diterbitkan", body = MfaKonfirmasi, headers(
            ("Set-Cookie" = String, description = "__Host-pr_session=...; HttpOnly; Secure; SameSite=Strict; Path=/"),
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(())
)]
async fn mfa_konfirmasi(
    axum::Json(_body): axum::Json<KonfirmasiMfaRequest>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Logout: cookie expired.
#[utoipa::path(
    post,
    path = "/admin/v1/auth/logout",
    operation_id = "logout",
    tag = "admin",
    responses(
        (status = 204, description = "Sesi diakhiri; cookie kedaluwarsa", headers(
            ("Set-Cookie" = String, description = "__Host-pr_session=; Max-Age=0; HttpOnly; Secure; SameSite=Strict; Path=/"),
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn logout() -> (StatusCode, Json<Problem>) {
    stub()
}

/// Current identity (U-1).
#[utoipa::path(
    get,
    path = "/admin/v1/auth/me",
    operation_id = "getMe",
    tag = "admin",
    responses(
        (status = 200, description = "Identitas sesi saat ini", body = Me, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn get_me() -> (StatusCode, Json<Problem>) {
    stub()
}

/// UI preference update (Addendum A section 7.2).
#[utoipa::path(
    patch,
    path = "/admin/v1/auth/me/preferensi",
    operation_id = "updatePreferensi",
    tag = "admin",
    request_body(content = PreferensiUpdate, content_type = "application/json", description = "Tanpa id pengguna; hanya bahasa UI sendiri"),
    responses(
        (status = 200, description = "Preferensi tersimpan", body = PreferensiUpdate, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn update_preferensi(
    axum::Json(_body): axum::Json<PreferensiUpdate>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Session state; does not shift the idle window (rev 5 section 3.6f).
#[utoipa::path(
    get,
    path = "/admin/v1/auth/sesi",
    operation_id = "getSesi",
    tag = "admin",
    responses(
        (status = 200, description = "Status sesi; idle tidak digeser", body = SesiStatus, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("sesi" = (summary = "Sesi aktif; seluruh field wajib", value = json!({"idle_berakhir_pada": "2026-11-06T10:30:00Z", "absolut_berakhir_pada": "2026-11-06T18:00:00Z", "waktu_server": "2026-11-06T10:00:00Z"}))))
        ),
        (status = 401, response = Problem401Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn get_sesi() -> (StatusCode, Json<Problem>) {
    stub()
}

/// Extend the session: resets idle, never shifts the absolute limit, no
/// Set-Cookie (rev 5 section 3.6f).
#[utoipa::path(
    post,
    path = "/admin/v1/auth/sesi/perpanjang",
    operation_id = "perpanjangSesi",
    tag = "admin",
    responses(
        (status = 200, description = "Idle di-reset; batas absolut tetap; tanpa Set-Cookie", body = SesiStatus, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        ),
            examples(("sesi" = (summary = "Idle digeser ke +30 menit; absolut tidak berubah", value = json!({"idle_berakhir_pada": "2026-11-06T10:30:00Z", "absolut_berakhir_pada": "2026-11-06T18:00:00Z", "waktu_server": "2026-11-06T10:00:00Z"}))))
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn perpanjang_sesi() -> (StatusCode, Json<Problem>) {
    stub()
}

/// List providers (PA).
#[utoipa::path(
    get,
    path = "/admin/v1/providers",
    operation_id = "listProviders",
    tag = "admin",
    responses(
        (status = 200, description = "Daftar provider", body = [Provider], headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn list_providers() -> (StatusCode, Json<Problem>) {
    stub()
}

/// Get one provider (PA).
#[utoipa::path(
    get,
    path = "/admin/v1/providers/{id}",
    operation_id = "getProvider",
    tag = "admin",
    params(("id" = Uuid, Path, description = "provider_id")),
    responses(
        (status = 200, description = "Detail provider", body = Provider, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn get_provider(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Create provider (PA). Template errors carry `posisi` (U-9); unknown
/// `{{secret.X}}` puts the missing secret name in `errors`.
#[utoipa::path(
    post,
    path = "/admin/v1/providers",
    operation_id = "createProvider",
    tag = "admin",
    request_body(content = ProviderConfig, content_type = "application/json", description = "Kunci template tersimpan persis (USER_ID, TRESHOLD, NAMA_LGKP)"),
    responses(
        (status = 201, description = "Provider dibuat; provider_config_version naik", body = Provider, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn create_provider(
    axum::Json(_body): axum::Json<ProviderConfig>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Update provider (PA); old versions stay intact (V-03).
#[utoipa::path(
    put,
    path = "/admin/v1/providers/{id}",
    operation_id = "updateProvider",
    tag = "admin",
    params(("id" = Uuid, Path, description = "provider_id")),
    request_body(content = ProviderConfig, content_type = "application/json", description = "alasan wajib (10..500) untuk audit konfigurasi"),
    responses(
        (status = 200, description = "Versi baru; versi lama tetap utuh (V-03)", body = Provider, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn update_provider(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<ProviderConfig>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// List secret names (PA). Values are never returned.
#[utoipa::path(
    get,
    path = "/admin/v1/secrets",
    operation_id = "listSecrets",
    tag = "admin",
    responses(
        (status = 200, description = "Nama secret, tersamar, dan jumlah provider pemakai", body = [SecretRingkas], headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn list_secrets() -> (StatusCode, Json<Problem>) {
    stub()
}

/// Store a secret value (PA); write-only, never echoed (US-02).
#[utoipa::path(
    put,
    path = "/admin/v1/secrets/{nama}",
    operation_id = "putSecret",
    tag = "admin",
    params(("nama" = String, Path, description = "Nama secret; tersimpan persis")),
    request_body(content = SecretNilai, content_type = "application/json"),
    responses(
        (status = 204, description = "Secret tersimpan", headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn put_secret(
    axum::extract::Path(_nama): axum::extract::Path<String>,
    axum::Json(_body): axum::Json<SecretNilai>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Delete a secret (PA). 409 while an active provider references it (US-02
/// AC4; the detail lists those providers).
#[utoipa::path(
    delete,
    path = "/admin/v1/secrets/{nama}",
    operation_id = "deleteSecret",
    tag = "admin",
    params(("nama" = String, Path, description = "Nama secret")),
    responses(
        (status = 204, description = "Secret dihapus", headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin),
        (status = 409, response = Problem409Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn delete_secret(
    axum::extract::Path(_nama): axum::extract::Path<String>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// List upstream pools (PA).
#[utoipa::path(
    get,
    path = "/admin/v1/upstream-pools",
    operation_id = "listUpstreamPools",
    tag = "admin",
    responses(
        (status = 200, description = "Daftar upstream pool", body = [UpstreamPool], headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn list_upstream_pools() -> (StatusCode, Json<Problem>) {
    stub()
}

/// Get one upstream pool (PA).
#[utoipa::path(
    get,
    path = "/admin/v1/upstream-pools/{id}",
    operation_id = "getUpstreamPool",
    tag = "admin",
    params(("id" = Uuid, Path, description = "upstream_pool_id")),
    responses(
        (status = 200, description = "Detail upstream pool", body = UpstreamPool, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn get_upstream_pool(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Create upstream pool (PA). Out-of-range KP-01 values are 422; `kalender`
/// without a source of certainty in `alasan` is 422 (KP-02).
#[utoipa::path(
    post,
    path = "/admin/v1/upstream-pools",
    operation_id = "createUpstreamPool",
    tag = "admin",
    request_body(content = UpstreamPoolConfig, content_type = "application/json"),
    responses(
        (status = 201, description = "Upstream pool dibuat", body = UpstreamPool, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn create_upstream_pool(
    axum::Json(_body): axum::Json<UpstreamPoolConfig>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Update upstream pool (PA). `alasan` wajib; `reset_kuota_*` applies from
/// the next period (KP-02).
#[utoipa::path(
    put,
    path = "/admin/v1/upstream-pools/{id}",
    operation_id = "updateUpstreamPool",
    tag = "admin",
    params(("id" = Uuid, Path, description = "upstream_pool_id")),
    request_body(content = UpstreamPoolConfig, content_type = "application/json", description = "alasan wajib (10..500)"),
    responses(
        (status = 200, description = "Upstream pool diperbarui", body = UpstreamPool, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn update_upstream_pool(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<UpstreamPoolConfig>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// List downstream pools (PA).
#[utoipa::path(
    get,
    path = "/admin/v1/downstream-pools",
    operation_id = "listDownstreamPools",
    tag = "admin",
    responses(
        (status = 200, description = "Daftar downstream pool", body = [DownstreamPool], headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn list_downstream_pools() -> (StatusCode, Json<Problem>) {
    stub()
}

/// Get one downstream pool (PA).
#[utoipa::path(
    get,
    path = "/admin/v1/downstream-pools/{id}",
    operation_id = "getDownstreamPool",
    tag = "admin",
    params(("id" = Uuid, Path, description = "downstream_pool_id")),
    responses(
        (status = 200, description = "Detail downstream pool", body = DownstreamPool, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn get_downstream_pool(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Create downstream pool (PA; F-1: single upstream allocation).
#[utoipa::path(
    post,
    path = "/admin/v1/downstream-pools",
    operation_id = "createDownstreamPool",
    tag = "admin",
    request_body(content = DownstreamPoolConfig, content_type = "application/json"),
    responses(
        (status = 201, description = "Downstream pool dibuat", body = DownstreamPool, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn create_downstream_pool(
    axum::Json(_body): axum::Json<DownstreamPoolConfig>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Update downstream pool (PA; F-1: `prioritas` read-only, tidak di body).
#[utoipa::path(
    put,
    path = "/admin/v1/downstream-pools/{id}",
    operation_id = "updateDownstreamPool",
    tag = "admin",
    params(("id" = Uuid, Path, description = "downstream_pool_id")),
    request_body(content = DownstreamPoolConfig, content_type = "application/json", description = "alasan wajib (10..500)"),
    responses(
        (status = 200, description = "Downstream pool diperbarui", body = DownstreamPool, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn update_downstream_pool(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<DownstreamPoolConfig>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// List clients (PA; CO sees only its own client).
#[utoipa::path(
    get,
    path = "/admin/v1/clients",
    operation_id = "listClients",
    tag = "admin",
    responses(
        (status = 200, description = "Daftar client (CO hanya clientnya)", body = [Client], headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn list_clients() -> (StatusCode, Json<Problem>) {
    stub()
}

/// Get one client (PA; CO only its own).
#[utoipa::path(
    get,
    path = "/admin/v1/clients/{id}",
    operation_id = "getClient",
    tag = "admin",
    params(("id" = Uuid, Path, description = "client_id")),
    responses(
        (status = 200, description = "Detail client", body = Client, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn get_client(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Create client (PA; L-04 limiter ranges).
#[utoipa::path(
    post,
    path = "/admin/v1/clients",
    operation_id = "createClient",
    tag = "admin",
    request_body(content = ClientConfig, content_type = "application/json"),
    responses(
        (status = 201, description = "Client dibuat", body = Client, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn create_client(axum::Json(_body): axum::Json<ClientConfig>) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Update client (PA). `alasan` wajib saat mengubah limiter.
#[utoipa::path(
    put,
    path = "/admin/v1/clients/{id}",
    operation_id = "updateClient",
    tag = "admin",
    params(("id" = Uuid, Path, description = "client_id")),
    request_body(content = ClientConfig, content_type = "application/json", description = "alasan wajib saat ubah limiter"),
    responses(
        (status = 200, description = "Client diperbarui", body = Client, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn update_client(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<ClientConfig>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Replace the client-endpoint assignment (PA; U-7 shape).
#[utoipa::path(
    put,
    path = "/admin/v1/clients/{id}/endpoints",
    operation_id = "setClientEndpoints",
    tag = "admin",
    params(("id" = Uuid, Path, description = "client_id")),
    request_body(content = ClientEndpoints, content_type = "application/json", description = "Penugasan lengkap menggantikan penugasan sebelumnya (U-7)"),
    responses(
        (status = 200, description = "Penugasan tersimpan", body = Client, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn set_client_endpoints(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<ClientEndpoints>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// List client credentials (PA; CO its own client). No hashes.
#[utoipa::path(
    get,
    path = "/admin/v1/clients/{id}/credentials",
    operation_id = "listCredentials",
    tag = "admin",
    params(("id" = Uuid, Path, description = "client_id")),
    responses(
        (status = 200, description = "Kredensial client tanpa hash", body = [Credential], headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn list_credentials(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Create a client credential (PA; CO its own client). Secret returned
/// exactly once, 256-bit base64url; max two active per client.
#[utoipa::path(
    post,
    path = "/admin/v1/clients/{id}/credentials",
    operation_id = "createCredential",
    tag = "admin",
    params(("id" = Uuid, Path, description = "client_id")),
    request_body(content = CredentialCreate, content_type = "application/json", description = "Default dan maksimum kedaluwarsa 365 hari"),
    responses(
        (status = 201, description = "Kredensial dibuat; secret tampil sekali", body = CredentialSecret, headers(
            ("Cache-Control" = String, description = "no-store; secret tidak boleh tersimpan (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 409, response = Problem409Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn create_credential(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<CredentialCreate>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Webhook configuration of one client (PA; CO its own client). 404 when
/// never configured.
#[utoipa::path(
    get,
    path = "/admin/v1/clients/{id}/webhook",
    operation_id = "getWebhookConfig",
    tag = "admin",
    params(("id" = Uuid, Path, description = "client_id")),
    responses(
        (status = 200, description = "Konfigurasi webhook; secret tidak pernah dikembalikan", body = WebhookConfig, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn get_webhook_config(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Save webhook configuration (US-06). URL must be https and its host in the
/// allowlist; checked on save and on send.
#[utoipa::path(
    put,
    path = "/admin/v1/clients/{id}/webhook",
    operation_id = "putWebhookConfig",
    tag = "admin",
    params(("id" = Uuid, Path, description = "client_id")),
    request_body(content = WebhookConfigPut, content_type = "application/json"),
    responses(
        (status = 200, description = "Konfigurasi tersimpan", body = WebhookConfig, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn put_webhook_config(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<WebhookConfigPut>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Rotate the webhook secret (US-06; F-5). Old secret stays active only as
/// long as needed for in-flight deliveries; rotation history is stored.
#[utoipa::path(
    post,
    path = "/admin/v1/clients/{id}/webhook/rotasi-secret",
    operation_id = "rotateWebhookSecret",
    tag = "admin",
    params(("id" = Uuid, Path, description = "client_id")),
    responses(
        (status = 200, description = "Secret baru; `whsec_` + 32-byte base64, tampil sekali", body = WebhookSecret, headers(
            ("Cache-Control" = String, description = "no-store; secret tidak boleh tersimpan (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn rotate_webhook_secret(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Webhook delivery log (US-06): time range, result filter, kursor.
#[utoipa::path(
    get,
    path = "/admin/v1/clients/{id}/webhook/log",
    operation_id = "listWebhookLog",
    tag = "admin",
    params(
        ("id" = Uuid, Path, description = "client_id"),
        ("dari" = Option<String>, Query, description = "ISO 8601 UTC, inklusif"),
        ("sampai" = Option<String>, Query, description = "ISO 8601 UTC, inklusif"),
        ("hasil" = Option<String>, Query, description = "terkirim | gagal"),
        ("kursor" = Option<String>, Query, description = "Kursor halaman berikutnya")
    ),
    responses(
        (status = 200, description = "Riwayat pengiriman", body = [WebhookLog], headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn list_webhook_log(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// List webhook allowlist entries (PA).
#[utoipa::path(
    get,
    path = "/admin/v1/webhook-allowlist",
    operation_id = "listWebhookAllowlist",
    tag = "admin",
    responses(
        (status = 200, description = "Domain diizinkan dan statusnya", body = [AllowlistEntry], headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn list_webhook_allowlist() -> (StatusCode, Json<Problem>) {
    stub()
}

/// Add or update an allowlist entry (PA).
#[utoipa::path(
    put,
    path = "/admin/v1/webhook-allowlist/{domain}",
    operation_id = "putWebhookAllowlistEntry",
    tag = "admin",
    params(("domain" = String, Path, description = "Domain, misal mpm.example.go.id")),
    request_body(content = AllowlistPut, content_type = "application/json"),
    responses(
        (status = 200, description = "Entri tersimpan", body = AllowlistEntry, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn put_webhook_allowlist_entry(
    axum::extract::Path(_domain): axum::extract::Path<String>,
    axum::Json(_body): axum::Json<AllowlistPut>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Remove an allowlist entry (PA).
#[utoipa::path(
    delete,
    path = "/admin/v1/webhook-allowlist/{domain}",
    operation_id = "deleteWebhookAllowlistEntry",
    tag = "admin",
    params(("domain" = String, Path, description = "Domain")),
    responses(
        (status = 204, description = "Entri dihapus", headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn delete_webhook_allowlist_entry(
    axum::extract::Path(_domain): axum::extract::Path<String>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// List endpoints (PA).
#[utoipa::path(
    get,
    path = "/admin/v1/endpoints",
    operation_id = "listEndpoints",
    tag = "admin",
    responses(
        (status = 200, description = "Daftar endpoint", body = [Endpoint], headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn list_endpoints() -> (StatusCode, Json<Problem>) {
    stub()
}

/// Get one endpoint (PA).
#[utoipa::path(
    get,
    path = "/admin/v1/endpoints/{id}",
    operation_id = "getEndpoint",
    tag = "admin",
    params(("id" = Uuid, Path, description = "endpoint_id")),
    responses(
        (status = 200, description = "Detail endpoint", body = Endpoint, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn get_endpoint(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Create endpoint (PA; US-15, M-R1: Dukcapil endpoints must include
/// `status_penerima_manfaat` in metadata_fields).
#[utoipa::path(
    post,
    path = "/admin/v1/endpoints",
    operation_id = "createEndpoint",
    tag = "admin",
    request_body(content = EndpointConfig, content_type = "application/json"),
    responses(
        (status = 201, description = "Endpoint dibuat; endpoint_config_version = 1", body = Endpoint, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn create_endpoint(
    axum::Json(_body): axum::Json<EndpointConfig>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Update endpoint (PA; M-R1).
#[utoipa::path(
    put,
    path = "/admin/v1/endpoints/{id}",
    operation_id = "updateEndpoint",
    tag = "admin",
    params(("id" = Uuid, Path, description = "endpoint_id")),
    request_body(content = EndpointConfig, content_type = "application/json", description = "alasan wajib (10..500)"),
    responses(
        (status = 200, description = "Versi baru; versi lama tetap utuh (V-03)", body = Endpoint, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn update_endpoint(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<EndpointConfig>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

// ---------------------------------------------------------------------------
// K-20: preview, deactivate, activate for six entities (section 3a)
// ---------------------------------------------------------------------------

/// Preview client deactivation: cancels `queued`/`processing` requests of the
/// client, fails the rest (waiting for retry with attempt_count > 0, expired
/// with remaining TTL). No request_id, NIK or payload.
#[utoipa::path(
    get,
    path = "/admin/v1/clients/{id}/pratinjau-nonaktif",
    operation_id = "pratinjauNonaktifkanClient",
    tag = "admin",
    params(("id" = Uuid, Path, description = "client_id")),
    responses(
        (status = 200, description = "Pratinjau dampak per status", body = PratinjauK20, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn pratinjau_nonaktifkan_client(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Deactivate a client (K-20): cancel queued, fail waiting-for-retry, expire
/// the rest. Same numbers go to the config audit.
#[utoipa::path(
    post,
    path = "/admin/v1/clients/{id}/nonaktifkan",
    operation_id = "nonaktifkanClient",
    tag = "admin",
    params(("id" = Uuid, Path, description = "client_id")),
    request_body(content = AlasanK20, content_type = "application/json"),
    responses(
        (status = 200, description = "Eksekusi selesai; jumlah cancel/fail dicatat di audit konfigurasi", body = HasilK20, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn nonaktifkan_client(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<AlasanK20>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Activate a client (section 3a.1). No re-verification required. Applies to
/// requests submitted after this call, needs no K-20 dialog, and is recorded in
/// the configuration audit log.
#[utoipa::path(
    post,
    path = "/admin/v1/clients/{id}/aktifkan",
    operation_id = "aktifkanClient",
    tag = "admin",
    params(("id" = Uuid, Path, description = "client_id")),
    responses(
        (status = 200, description = "Client aktif", body = Client, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn aktifkan_client(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Preview credential revocation (K-20): cancels queued, fails waiting for
/// retry, expires the rest of that client.
#[utoipa::path(
    get,
    path = "/admin/v1/credentials/{id}/pratinjau-cabut",
    operation_id = "pratinjauCabutCredential",
    tag = "admin",
    params(("id" = Uuid, Path, description = "credential_id")),
    responses(
        (status = 200, description = "Pratinjau dampak per status", body = PratinjauK20, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn pratinjau_cabut_credential(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Revoke a credential (K-20): cancel queued, fail waiting for retry, expire
/// the rest.
#[utoipa::path(
    post,
    path = "/admin/v1/credentials/{id}/cabut",
    operation_id = "cabutCredential",
    tag = "admin",
    params(("id" = Uuid, Path, description = "credential_id")),
    request_body(content = AlasanK20, content_type = "application/json"),
    responses(
        (status = 200, description = "Eksekusi selesai; status kredensial jadi dicabut", body = Credential, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn cabut_credential(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<AlasanK20>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Re-activate a credential: always 409 for revoked credentials (U-54b).
#[utoipa::path(
    post,
    path = "/admin/v1/credentials/{id}/aktifkan",
    operation_id = "aktifkanCredential",
    tag = "admin",
    params(("id" = Uuid, Path, description = "credential_id")),
    responses(
        (status = 409, response = Problem409Admin),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn aktifkan_credential(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Preview endpoint deactivation (K-20).
#[utoipa::path(
    get,
    path = "/admin/v1/endpoints/{id}/pratinjau-nonaktif",
    operation_id = "pratinjauNonaktifkanEndpoint",
    tag = "admin",
    params(("id" = Uuid, Path, description = "endpoint_id")),
    responses(
        (status = 200, description = "Pratinjau dampak per status", body = PratinjauK20, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn pratinjau_nonaktifkan_endpoint(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Deactivate an endpoint (K-20).
#[utoipa::path(
    post,
    path = "/admin/v1/endpoints/{id}/nonaktifkan",
    operation_id = "nonaktifkanEndpoint",
    tag = "admin",
    params(("id" = Uuid, Path, description = "endpoint_id")),
    request_body(content = AlasanK20, content_type = "application/json"),
    responses(
        (status = 200, description = "Eksekusi selesai", body = HasilK20, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn nonaktifkan_endpoint(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<AlasanK20>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Activate an endpoint (section 3a.1). Applies to requests submitted after
/// this call, needs no K-20 dialog, and is recorded in the configuration
/// audit log.
#[utoipa::path(
    post,
    path = "/admin/v1/endpoints/{id}/aktifkan",
    operation_id = "aktifkanEndpoint",
    tag = "admin",
    params(("id" = Uuid, Path, description = "endpoint_id")),
    responses(
        (status = 200, description = "Endpoint aktif", body = Endpoint, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn aktifkan_endpoint(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Preview downstream pool deactivation (K-20).
#[utoipa::path(
    get,
    path = "/admin/v1/downstream-pools/{id}/pratinjau-nonaktif",
    operation_id = "pratinjauNonaktifkanDownstreamPool",
    tag = "admin",
    params(("id" = Uuid, Path, description = "downstream_pool_id")),
    responses(
        (status = 200, description = "Pratinjau dampak per status", body = PratinjauK20, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn pratinjau_nonaktifkan_downstream_pool(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Deactivate a downstream pool (K-20).
#[utoipa::path(
    post,
    path = "/admin/v1/downstream-pools/{id}/nonaktifkan",
    operation_id = "nonaktifkanDownstreamPool",
    tag = "admin",
    params(("id" = Uuid, Path, description = "downstream_pool_id")),
    request_body(content = AlasanK20, content_type = "application/json"),
    responses(
        (status = 200, description = "Eksekusi selesai", body = HasilK20, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn nonaktifkan_downstream_pool(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<AlasanK20>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Activate a downstream pool (section 3a.1). Applies to requests submitted after
/// this call, needs no K-20 dialog, and is recorded in the configuration
/// audit log.
#[utoipa::path(
    post,
    path = "/admin/v1/downstream-pools/{id}/aktifkan",
    operation_id = "aktifkanDownstreamPool",
    tag = "admin",
    params(("id" = Uuid, Path, description = "downstream_pool_id")),
    responses(
        (status = 200, description = "Downstream pool aktif", body = DownstreamPool, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn aktifkan_downstream_pool(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Preview upstream pool deactivation (K-20; section 3a.10): cancels/fails
/// requests across allocated pools, and shows where this pool is the last
/// active upstream.
#[utoipa::path(
    get,
    path = "/admin/v1/upstream-pools/{id}/pratinjau-nonaktif",
    operation_id = "pratinjauNonaktifkanUpstreamPool",
    tag = "admin",
    params(("id" = Uuid, Path, description = "upstream_pool_id")),
    responses(
        (status = 200, description = "Pratinjau plus upstream_aktif_terakhir_untuk dan dipakai_oleh_downstream_pool_count", body = PratinjauK20, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn pratinjau_nonaktifkan_upstream_pool(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Deactivate an upstream pool (K-20; section 3a.7 and 3a.10). Only requests
/// whose `upstream_pool_id` is this pool are affected: `akan_cancelled` for
/// `attempt_count = 0`, `akan_failed` for those waiting for a retry. Requests
/// not yet dispatched (`attempt_count = 0` with no `upstream_pool_id` set) are
/// unchanged and stay `queued` until an active upstream pool picks them up or
/// the TTL expires. If this pool is the last active upstream for a downstream
/// pool, see `upstream_aktif_terakhir_untuk`.
#[utoipa::path(
    post,
    path = "/admin/v1/upstream-pools/{id}/nonaktifkan",
    operation_id = "nonaktifkanUpstreamPool",
    tag = "admin",
    params(("id" = Uuid, Path, description = "upstream_pool_id")),
    request_body(content = AlasanK20, content_type = "application/json"),
    responses(
        (status = 200, description = "Eksekusi selesai", body = HasilK20, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn nonaktifkan_upstream_pool(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<AlasanK20>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Activate an upstream pool (section 3a.1). Applies to requests submitted after
/// this call, needs no K-20 dialog, and is recorded in the configuration
/// audit log.
#[utoipa::path(
    post,
    path = "/admin/v1/upstream-pools/{id}/aktifkan",
    operation_id = "aktifkanUpstreamPool",
    tag = "admin",
    params(("id" = Uuid, Path, description = "upstream_pool_id")),
    responses(
        (status = 200, description = "Upstream pool aktif", body = UpstreamPool, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn aktifkan_upstream_pool(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Preview provider deactivation (K-20).
#[utoipa::path(
    get,
    path = "/admin/v1/providers/{id}/pratinjau-nonaktif",
    operation_id = "pratinjauNonaktifkanProvider",
    tag = "admin",
    params(("id" = Uuid, Path, description = "provider_id")),
    responses(
        (status = 200, description = "Pratinjau dampak per status", body = PratinjauK20, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn pratinjau_nonaktifkan_provider(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Deactivate a provider (K-20).
#[utoipa::path(
    post,
    path = "/admin/v1/providers/{id}/nonaktifkan",
    operation_id = "nonaktifkanProvider",
    tag = "admin",
    params(("id" = Uuid, Path, description = "provider_id")),
    request_body(content = AlasanK20, content_type = "application/json"),
    responses(
        (status = 200, description = "Eksekusi selesai", body = HasilK20, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn nonaktifkan_provider(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
    axum::Json(_body): axum::Json<AlasanK20>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Activate a provider (section 3a.1). Applies to requests submitted after
/// this call, needs no K-20 dialog, and is recorded in the configuration
/// audit log.
#[utoipa::path(
    post,
    path = "/admin/v1/providers/{id}/aktifkan",
    operation_id = "aktifkanProvider",
    tag = "admin",
    params(("id" = Uuid, Path, description = "provider_id")),
    responses(
        (status = 200, description = "Provider aktif", body = Provider, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn aktifkan_provider(
    axum::extract::Path(_id): axum::extract::Path<Uuid>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Quota tracker (US-12; KP-07): numbers from the DB, not Redis. Read-only;
/// available to `platform_admin` and `operator`.
#[utoipa::path(
    get,
    path = "/admin/v1/kuota",
    operation_id = "getQuotaTracker",
    tag = "admin",
    params(("upstream_pool_id" = Option<Uuid>, Query, description = "Filter satu upstream pool")),
    responses(
        (status = 200, description = "Baris pelacak kuota per pool/periode; read-only (PA, OP)", body = [KuotaTracker], headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn get_quota_tracker() -> (StatusCode, Json<Problem>) {
    stub()
}

// ---------------------------------------------------------------------------
// Log, audit akses, ekspor (US-14, 16; R-18)
// ---------------------------------------------------------------------------

/// Search the request log (US-14). Every parameter is checked for NIK or NIK
/// fragments (403 `pencarian_nik_dilarang`); the search itself is logged.
#[utoipa::path(
    post,
    path = "/admin/v1/log/cari",
    operation_id = "searchLog",
    tag = "admin",
    request_body(content = LogCariRequest, content_type = "application/json", description = "Rentang waktu wajib; POST agar filter tidak masuk log akses"),
    responses(
        (status = 200, description = "Halaman baris log; nik tersamar untuk OP", body = LogPage, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn search_log(axum::Json(body): axum::Json<LogCariRequest>) -> (StatusCode, Json<Problem>) {
    // Keep S11's closed interval invariant at the trust boundary as well as in
    // the UI; callers cannot bypass the date-range check with a raw request.
    if body.dari > body.sampai {
        let mut problem = Problem::stub();
        problem.status = 422;
        problem.detail = Some("Rentang waktu tidak valid.".into());
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(problem));
    }
    stub()
}

/// One log row by transaction code (US-14); masked for OP.
#[utoipa::path(
    get,
    path = "/admin/v1/log/{kode_transaksi}",
    operation_id = "getLogEntry",
    tag = "admin",
    params(("kode_transaksi" = String, Path, description = "Kode transaksi poolrouter")),
    responses(
        (status = 200, description = "Baris log lengkap; tersamar untuk OP", body = LogEntry, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn get_log_entry(
    axum::extract::Path(_kode): axum::extract::Path<String>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Auditor-only NIK search (exact 16 digits; no wildcard, prefix or partial
/// NIK). Every call gets an `audit_akses` `cari_nik` row, including page
/// turns and empty results.
#[utoipa::path(
    post,
    path = "/admin/v1/auditor/log/cari-nik",
    operation_id = "searchLogByNik",
    tag = "admin",
    request_body(content = NikCariRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Baris log dengan NIK utuh", body = LogPage, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn search_log_by_nik(
    axum::Json(_body): axum::Json<NikCariRequest>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Open a payload view (auditor only). One `audit_akses` `lihat_payload` row
/// per opening.
#[utoipa::path(
    post,
    path = "/admin/v1/auditor/payload",
    operation_id = "openPayload",
    tag = "admin",
    request_body(content = PayloadOpenRequest, content_type = "application/json", description = "Tepat satu dari request_id atau kode_transaksi; alasan wajib"),
    responses(
        (status = 200, description = "Payload permintaan (tanpa rahasia)", body = PayloadView, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 404, response = Problem404Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn open_payload(
    axum::Json(_body): axum::Json<PayloadOpenRequest>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Auditor CSV export (R-18): `Content-Disposition: attachment`,
/// `Cache-Control: no-store`, `Repr-Digest: sha-256=...`,
/// `Accept-Ranges: none`. Above 10,000 rows: 422 `ekspor_terlalu_besar`,
/// no file, `ditolak` audit row.
#[utoipa::path(
    post,
    path = "/admin/v1/auditor/log/ekspor",
    operation_id = "exportLogAuditor",
    tag = "admin",
    request_body(content = EksporRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Berkas CSV", body = String, content_type = "text/csv", headers(
            ("Content-Disposition" = String, description = "attachment; filename=..."),
            ("Cache-Control" = String, description = "Selalu no-store (R-16)"),
            ("Repr-Digest" = String, description = "sha-256=<digest base64>"),
            ("Accept-Ranges" = String, description = "Selalu none")
        ),
            examples(("csv" = (summary = "Dua baris contoh", value = json!("nik,requested_at,status_akses,kode_transaksi,queue_id,client_id\n3175000000000001,2026-11-03T01:02:03Z,diizinkan,PRQ-20261103-000001,0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f,c1d2e3f4-0000-0000-0000-000000000001\n"))))
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn export_log_auditor(
    axum::Json(_body): axum::Json<EksporRequest>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

/// Auditor search of `audit_akses` (who did what, when, how long; without
/// `nik_hmac`). Range max 31 days.
#[utoipa::path(
    post,
    path = "/admin/v1/auditor/audit-akses/cari",
    operation_id = "searchAuditAkses",
    tag = "admin",
    request_body(content = AuditCariRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Halaman baris audit akses", body = AuditAksesPage, headers(
            ("Cache-Control" = String, description = "Selalu no-store (R-16)")
        )
        ),
        (status = 401, response = Problem401Admin),
        (status = 403, response = Problem403Admin),
        (status = 422, response = Problem422Admin)
    ),
    security(("sessionCookie" = []))
)]
async fn search_audit_akses(
    axum::Json(_body): axum::Json<AuditCariRequest>,
) -> (StatusCode, Json<Problem>) {
    stub()
}

// ---------------------------------------------------------------------------
// Document assembly
// ---------------------------------------------------------------------------

/// Adds the admin session-cookie security scheme to the document components.
pub struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "sessionCookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                "__Host-pr_session",
                "Cookie sesi admin; HttpOnly, Secure, SameSite=Strict (ADR-0003 section 3.6)",
            ))),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    info(title = "PoolRouter Admin API", description = "Kontrak admin PoolRouter F0 (SHAA-175): sesi cookie, konfigurasi, K-20, kuota, log/audit. Error memakai RFC 9457 problem+json dengan kode tertutup (Addendum A); rincian tiap kode ada di docs/masalah.md. Tulis hanya dari Origin yang sah; setiap tulis tercatat di audit konfigurasi."),
    paths(
        login, logout, get_me, update_preferensi, get_sesi, perpanjang_sesi,
        mfa_verifikasi, mfa_daftar, mfa_konfirmasi,
        list_providers, get_provider, create_provider, update_provider,
        list_secrets, put_secret, delete_secret,
        list_upstream_pools, get_upstream_pool, create_upstream_pool, update_upstream_pool,
        list_downstream_pools, get_downstream_pool, create_downstream_pool, update_downstream_pool,
        list_clients, get_client, create_client, update_client, set_client_endpoints,
        list_credentials, create_credential,
        get_webhook_config, put_webhook_config, rotate_webhook_secret, list_webhook_log,
        list_webhook_allowlist, put_webhook_allowlist_entry, delete_webhook_allowlist_entry,
        list_endpoints, get_endpoint, create_endpoint, update_endpoint,
        pratinjau_nonaktifkan_client, nonaktifkan_client, aktifkan_client,
        pratinjau_cabut_credential, cabut_credential, aktifkan_credential,
        pratinjau_nonaktifkan_endpoint, nonaktifkan_endpoint, aktifkan_endpoint,
        pratinjau_nonaktifkan_downstream_pool, nonaktifkan_downstream_pool, aktifkan_downstream_pool,
        pratinjau_nonaktifkan_upstream_pool, nonaktifkan_upstream_pool, aktifkan_upstream_pool,
        pratinjau_nonaktifkan_provider, nonaktifkan_provider, aktifkan_provider,
        get_quota_tracker,
        search_log, get_log_entry, search_log_by_nik, open_payload, export_log_auditor, search_audit_akses
    ),
    components(schemas(
        LoginRequest, Me, PreferensiUpdate, SesiStatus,
        KodeMfaRequest, DaftarMfaRequest, KonfirmasiMfaRequest, MfaDaftar, MfaKonfirmasi, MeMfa,
        ProviderConfig, ParameterRule, RuleValidasi, Provider,
        SecretRingkas, SecretNilai,
        UpstreamPoolConfig, UpstreamPool,
        DownstreamPoolConfig, DownstreamPool,
        ClientConfig, Client, ClientEndpoints,
        Credential, CredentialCreate, CredentialSecret,
        WebhookConfig, WebhookConfigPut, WebhookSecret, WebhookLog,
        AllowlistEntry, AllowlistPut,
        Endpoint, EndpointConfig, MetadataFieldDef,
        PratinjauK20, UpstreamAktifTerakhir, HasilK20, AlasanK20,
        KuotaTracker,
        LogCariRequest, LogBaris, LogPage, LogEntry,
        NikCariRequest, PayloadOpenRequest, PayloadView, EksporRequest,
        AuditCariRequest, AuditAksesBaris, AuditAksesPage,
        BahasaUi, JendelaKuota, JenisKredensial, RuleJenis,
        super::problem::ProblemKodeAdmin, super::problem_admin::ProblemAdmin,
        super::problem::ValidasiErrorAdmin, super::problem::Posisi,
        super::problem::JumlahNamaTidakCocok, super::enums::JenisObjek
    ), responses(
        Problem401Admin, Problem403Admin, Problem400Admin, Problem404Admin,
        Problem422Admin, Problem429Admin, Problem409Admin
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "admin", description = "Backend Web UI admin (kontrak review SHAA-175)")
    )
)]
pub struct AdminDoc;

pub fn yaml() -> Result<String, Box<dyn std::error::Error>> {
    Ok(AdminDoc::openapi().to_yaml()?)
}

/// Router with the admin operation stubs (utoipa-axum keeps annotations and
/// routes from the same functions).
#[allow(clippy::too_many_lines)]
pub fn router() -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(login))
        .routes(routes!(mfa_verifikasi))
        .routes(routes!(mfa_daftar))
        .routes(routes!(mfa_konfirmasi))
        .routes(routes!(logout))
        .routes(routes!(get_me))
        .routes(routes!(update_preferensi))
        .routes(routes!(get_sesi))
        .routes(routes!(perpanjang_sesi))
        .routes(routes!(list_providers))
        .routes(routes!(get_provider))
        .routes(routes!(create_provider))
        .routes(routes!(update_provider))
        .routes(routes!(list_secrets))
        .routes(routes!(put_secret))
        .routes(routes!(delete_secret))
        .routes(routes!(list_upstream_pools))
        .routes(routes!(get_upstream_pool))
        .routes(routes!(create_upstream_pool))
        .routes(routes!(update_upstream_pool))
        .routes(routes!(list_downstream_pools))
        .routes(routes!(get_downstream_pool))
        .routes(routes!(create_downstream_pool))
        .routes(routes!(update_downstream_pool))
        .routes(routes!(list_clients))
        .routes(routes!(get_client))
        .routes(routes!(create_client))
        .routes(routes!(update_client))
        .routes(routes!(set_client_endpoints))
        .routes(routes!(list_credentials))
        .routes(routes!(create_credential))
        .routes(routes!(get_webhook_config))
        .routes(routes!(put_webhook_config))
        .routes(routes!(rotate_webhook_secret))
        .routes(routes!(list_webhook_log))
        .routes(routes!(list_webhook_allowlist))
        .routes(routes!(put_webhook_allowlist_entry))
        .routes(routes!(delete_webhook_allowlist_entry))
        .routes(routes!(list_endpoints))
        .routes(routes!(get_endpoint))
        .routes(routes!(create_endpoint))
        .routes(routes!(update_endpoint))
        .routes(routes!(pratinjau_nonaktifkan_client))
        .routes(routes!(nonaktifkan_client))
        .routes(routes!(aktifkan_client))
        .routes(routes!(pratinjau_cabut_credential))
        .routes(routes!(cabut_credential))
        .routes(routes!(aktifkan_credential))
        .routes(routes!(pratinjau_nonaktifkan_endpoint))
        .routes(routes!(nonaktifkan_endpoint))
        .routes(routes!(aktifkan_endpoint))
        .routes(routes!(pratinjau_nonaktifkan_downstream_pool))
        .routes(routes!(nonaktifkan_downstream_pool))
        .routes(routes!(aktifkan_downstream_pool))
        .routes(routes!(pratinjau_nonaktifkan_upstream_pool))
        .routes(routes!(nonaktifkan_upstream_pool))
        .routes(routes!(aktifkan_upstream_pool))
        .routes(routes!(pratinjau_nonaktifkan_provider))
        .routes(routes!(nonaktifkan_provider))
        .routes(routes!(aktifkan_provider))
        .routes(routes!(get_quota_tracker))
        .routes(routes!(search_log))
        .routes(routes!(get_log_entry))
        .routes(routes!(search_log_by_nik))
        .routes(routes!(open_payload))
        .routes(routes!(export_log_auditor))
        .routes(routes!(search_audit_akses))
}
