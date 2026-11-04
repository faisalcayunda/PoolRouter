//! RFC 9457 `application/problem+json` (R-10, ADR-0001 section 5a.7).
//!
//! `kode` is a closed enum per document (`ProblemKodeDown` / `ProblemKodeAdmin`);
//! `type` is the stable anchor URL into `docs/masalah.md`. Each document's code
//! set must be a subset of the anchors in that document — asserted by
//! `api::tests::masalah_anchors`, which also checks that the two sets together
//! equal the anchors. One code per anchor lives here; `docs/masalah.md` stays a
//! single combined document shared by both APIs.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::enums::{ApiStatus, JenisObjek};

const ANCHOR: &str = "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#";

// Catatan internal (sengaja bukan doc comment, karena doc comment ikut ke YAML
// yang diserahkan ke BGN): menambah nilai di enum ini membekukan ulang daftar kode
// dan memerlukan keputusan versi API; kode khusus admin tidak boleh ditambahkan ke
// enum ini.

/// Kode untuk klien downstream. Daftar kode bersifat tertutup: klien tidak boleh
/// menganggap kode lain mungkin muncul, dan kode baru hanya dikirim bersama versi
/// API baru.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKodeDown {
    TidakTerautentikasi,
    /// 429: token throttle exhausted (60/minute per `client_id`, burst 10).
    TokenDibatasi,
    /// 400 (malformed JSON / unsupported grant) and 422 (field validation).
    ValidasiGagal,
    /// 422: metadata keys do not match the endpoint contract.
    MetadataTidakValid,
    /// 400: `Idempotency-Key` sent on the sync path.
    IdempotencyKeyTidakDidukung,
    /// 422: same key, different fingerprint.
    IdempotencyKeyKonflik,
    /// 429: period quota exhausted.
    KuotaHabis,
    /// 429: sync portion exhausted on the sync path.
    PorsiSyncHabis,
    /// 429: rejection limiter engaged.
    PenolakanBerlebih,
    /// 403 (endpoint) or 503 (pool/provider).
    ObjekNonaktif,
    /// 409: cancel of a `processing` or terminal request.
    TidakBisaDibatalkan,
    /// 503: Redis fail-closed; always `Retry-After: 5`.
    LayananTidakTersedia,
    /// 502: retry window exhausted.
    RetryHabis,
    /// 502: non-retryable provider failure class.
    ProviderErrorPermanen,
    /// 504: sync call did not finish in time.
    TimeoutSync,
    /// 404: unknown resource or another client's resource.
    TidakDitemukan,
}

impl ProblemKodeDown {
    /// Every downstream value, in wire order.
    pub const ALL: &'static [ProblemKodeDown] = &[
        ProblemKodeDown::TidakTerautentikasi,
        ProblemKodeDown::TokenDibatasi,
        ProblemKodeDown::ValidasiGagal,
        ProblemKodeDown::MetadataTidakValid,
        ProblemKodeDown::IdempotencyKeyTidakDidukung,
        ProblemKodeDown::IdempotencyKeyKonflik,
        ProblemKodeDown::KuotaHabis,
        ProblemKodeDown::PorsiSyncHabis,
        ProblemKodeDown::PenolakanBerlebih,
        ProblemKodeDown::ObjekNonaktif,
        ProblemKodeDown::TidakBisaDibatalkan,
        ProblemKodeDown::LayananTidakTersedia,
        ProblemKodeDown::RetryHabis,
        ProblemKodeDown::ProviderErrorPermanen,
        ProblemKodeDown::TimeoutSync,
        ProblemKodeDown::TidakDitemukan,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TidakTerautentikasi => "tidak_terautentikasi",
            Self::TokenDibatasi => "token_dibatasi",
            Self::ValidasiGagal => "validasi_gagal",
            Self::MetadataTidakValid => "metadata_tidak_valid",
            Self::IdempotencyKeyTidakDidukung => "idempotency_key_tidak_didukung",
            Self::IdempotencyKeyKonflik => "idempotency_key_konflik",
            Self::KuotaHabis => "kuota_habis",
            Self::PorsiSyncHabis => "porsi_sync_habis",
            Self::PenolakanBerlebih => "penolakan_berlebih",
            Self::ObjekNonaktif => "objek_nonaktif",
            Self::TidakBisaDibatalkan => "tidak_bisa_dibatalkan",
            Self::LayananTidakTersedia => "layanan_tidak_tersedia",
            Self::RetryHabis => "retry_habis",
            Self::ProviderErrorPermanen => "provider_error_permanen",
            Self::TimeoutSync => "timeout_sync",
            Self::TidakDitemukan => "tidak_ditemukan",
        }
    }

    /// RFC 9457 `title`. Human summary; details live in `docs/masalah.md`.
    pub fn title(&self) -> &'static str {
        match self {
            Self::TidakTerautentikasi => "Tidak terautentikasi",
            Self::TokenDibatasi => "Token dibatasi",
            Self::ValidasiGagal => "Validasi gagal",
            Self::MetadataTidakValid => "Metadata tidak valid",
            Self::IdempotencyKeyTidakDidukung => "Idempotency key tidak didukung",
            Self::IdempotencyKeyKonflik => "Konflik idempotency key",
            Self::KuotaHabis => "Kuota habis",
            Self::PorsiSyncHabis => "Porsi sync habis",
            Self::PenolakanBerlebih => "Penolakan berlebih",
            Self::ObjekNonaktif => "Objek nonaktif",
            Self::TidakBisaDibatalkan => "Tidak bisa dibatalkan",
            Self::LayananTidakTersedia => "Layanan tidak tersedia",
            Self::RetryHabis => "Retry habis",
            Self::ProviderErrorPermanen => "Kesalahan provider permanen",
            Self::TimeoutSync => "Timeout sync",
            Self::TidakDitemukan => "Tidak ditemukan",
        }
    }

    /// Stable `type` anchor (ADR-0001 section 5a.7).
    pub fn type_url(&self) -> String {
        format!("{ANCHOR}{}", self.as_str())
    }
}

/// Admin (Web UI backend) codes. Not visible to external clients, so a new
/// value here does not change the downstream contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKodeAdmin {
    TidakTerautentikasi,
    /// 401 on `POST /admin/v1/login`: wrong credentials, not distinguished.
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
    /// 400 (malformed JSON) and 422 (field/template validation).
    ValidasiGagal,
    /// 404: unknown resource.
    TidakDitemukan,
    /// 503: Redis fail-closed or component down.
    LayananTidakTersedia,
}

impl ProblemKodeAdmin {
    /// Every admin value, in wire order.
    pub const ALL: &'static [ProblemKodeAdmin] = &[
        ProblemKodeAdmin::TidakTerautentikasi,
        ProblemKodeAdmin::KredensialSalah,
        ProblemKodeAdmin::LoginDibatasi,
        ProblemKodeAdmin::OriginTidakValid,
        ProblemKodeAdmin::AksesDitolak,
        ProblemKodeAdmin::PencarianNikDilarang,
        ProblemKodeAdmin::SecretDipakai,
        ProblemKodeAdmin::KredensialDicabut,
        ProblemKodeAdmin::BatasKredensial,
        ProblemKodeAdmin::WebhookDomainTidakDiizinkan,
        ProblemKodeAdmin::WebhookTidakHttps,
        ProblemKodeAdmin::MetadataWajibHilang,
        ProblemKodeAdmin::EksporTerlaluBesar,
        ProblemKodeAdmin::ValidasiGagal,
        ProblemKodeAdmin::TidakDitemukan,
        ProblemKodeAdmin::LayananTidakTersedia,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TidakTerautentikasi => "tidak_terautentikasi",
            Self::KredensialSalah => "kredensial_salah",
            Self::LoginDibatasi => "login_dibatasi",
            Self::OriginTidakValid => "origin_tidak_valid",
            Self::AksesDitolak => "akses_ditolak",
            Self::PencarianNikDilarang => "pencarian_nik_dilarang",
            Self::SecretDipakai => "secret_dipakai",
            Self::KredensialDicabut => "kredensial_dicabut",
            Self::BatasKredensial => "batas_kredensial",
            Self::WebhookDomainTidakDiizinkan => "webhook_domain_tidak_diizinkan",
            Self::WebhookTidakHttps => "webhook_tidak_https",
            Self::MetadataWajibHilang => "metadata_wajib_hilang",
            Self::EksporTerlaluBesar => "ekspor_terlalu_besar",
            Self::ValidasiGagal => "validasi_gagal",
            Self::TidakDitemukan => "tidak_ditemukan",
            Self::LayananTidakTersedia => "layanan_tidak_tersedia",
        }
    }

    /// RFC 9457 `title`. Human summary; details live in `docs/masalah.md`.
    pub fn title(&self) -> &'static str {
        match self {
            Self::TidakTerautentikasi => "Tidak terautentikasi",
            Self::KredensialSalah => "Kredensial salah",
            Self::LoginDibatasi => "Login dibatasi",
            Self::OriginTidakValid => "Origin tidak valid",
            Self::AksesDitolak => "Akses ditolak",
            Self::PencarianNikDilarang => "Pencarian NIK dilarang",
            Self::SecretDipakai => "Secret masih dipakai",
            Self::KredensialDicabut => "Kredensial masih dipakai",
            Self::BatasKredensial => "Batas kredensial tercapai",
            Self::WebhookDomainTidakDiizinkan => "Domain webhook tidak diizinkan",
            Self::WebhookTidakHttps => "Webhook bukan https",
            Self::MetadataWajibHilang => "Metadata wajib hilang",
            Self::EksporTerlaluBesar => "Ekspor terlalu besar",
            Self::ValidasiGagal => "Validasi gagal",
            Self::TidakDitemukan => "Tidak ditemukan",
            Self::LayananTidakTersedia => "Layanan tidak tersedia",
        }
    }

    /// Stable `type` anchor (ADR-0001 section 5a.7).
    pub fn type_url(&self) -> String {
        format!("{ANCHOR}{}", self.as_str())
    }
}

/// Position inside the provider body template. Admin API only, validation of
/// the template only — downstream bodies never carry it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Posisi {
    /// 1-based line in the body template.
    #[schema(minimum = 1)]
    pub baris: i64,
    /// 1-based column in the body template.
    #[schema(minimum = 1)]
    pub kolom: i64,
}

/// Satu field input yang gagal validasi pada API downstream. `kode` adalah kode
/// validasi per field; `batas` hanya membawa parameter angka dari aturan yang
/// dilanggar (tidak pernah nilai yang ditolak). Tidak ada `posisi`: error
/// template hanya ada di API admin.
///
/// `PartialEq` tanpa `Eq` dan itu disengaja: `batas` bertipe `f64`, sehingga
/// perbandingan hanya untuk uji dan tipe ini tidak boleh dipakai sebagai kunci
/// map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ValidasiErrorDown {
    /// Nama field yang ditolak, persis seperti dikirim (`TRESHOLD`, `NAMA_LGKAP`,
    /// atau kunci metadata). Nama yang tidak cocok dengan `^[A-Za-z0-9_]{1,64}$`
    /// tidak pernah dikembalikan; nama seperti itu hanya dihitung di
    /// `jumlah_nama_tidak_cocok`.
    pub field: String,
    pub kode: String,
    /// Parameter angka aturan yang dilanggar, bila ada. Opsional: tidak ada bila
    /// aturan tidak punya parameter angka (misalnya `wajib` atau aturan yang tidak
    /// dikenal). Bila ada, nilainya selalu angka, tidak pernah `null`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = f64)]
    pub batas: Option<f64>,
}

/// One failing field, admin. Field names are usually JSON pointers into the
/// request body; template errors carry `posisi`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ValidasiErrorAdmin {
    /// Name of the rejected field exactly as sent, or a JSON pointer for
    /// admin request bodies and template errors (D6).
    pub field: String,
    pub kode: String,
    /// Numeric rule parameter of the violated rule, if any. Optional: absent
    /// when the rule has no numeric parameter (for example `wajib` or an
    /// unknown rule). When present the value is always a number, never `null`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = f64)]
    pub batas: Option<f64>,
    /// Template position. Admin API and template errors only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posisi: Option<Posisi>,
}

/// Count of failing names that are withheld instead of echoed (D6). A count of
/// zero is never emitted: the field is absent when no name was withheld.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(try_from = "u32", into = "u32")]
#[schema(value_type = u32, minimum = 1)]
pub struct JumlahNamaTidakCocok(u32);

impl TryFrom<u32> for JumlahNamaTidakCocok {
    type Error = &'static str;

    fn try_from(nilai: u32) -> Result<Self, Self::Error> {
        if nilai >= 1 {
            Ok(Self(nilai))
        } else {
            Err("jumlah_nama_tidak_cocok minimal 1")
        }
    }
}

impl From<JumlahNamaTidakCocok> for u32 {
    fn from(nilai: JumlahNamaTidakCocok) -> Self {
        nilai.0
    }
}

/// Body problem RFC 9457 untuk API downstream. `detail` tidak pernah memuat nilai
/// input, NIK, atau secret. Nama field memakai bahasa Indonesia, kecuali anggota
/// tetap yang ditetapkan RFC.
///
/// Body ini juga dipakai sebagai dasar body admin: keduanya membawa `type`,
/// `title`, `status`, `kode`, `detail`, dan `errors`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Problem {
    /// Stable anchor `https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#<kode>`.
    #[serde(rename = "type")]
    pub type_: String,
    pub title: String,
    /// HTTP status, 400..=599.
    #[schema(minimum = 400, maximum = 599)]
    pub status: u16,
    pub kode: ProblemKodeDown,
    /// Penjelasan untuk manusia tanpa nilai sensitif; tidak ada bila tidak perlu.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Field yang gagal validasi; hanya pada body 422 `validasi_gagal` dan
    /// `metadata_tidak_valid`. Isinya nama field seperti yang dikirim, tidak pernah
    /// nilainya. Tidak ada bila tidak ada field yang gagal; tidak pernah `null`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub errors: Option<Vec<ValidasiErrorDown>>,
    /// Jumlah nama yang gagal validasi karena tidak cocok dengan
    /// `^[A-Za-z0-9_]{1,64}$`, sehingga hanya dihitung dan tidak dikembalikan. Field
    /// ini ada hanya bila sedikitnya satu nama seperti itu ditolak; tidak pernah
    /// `null`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub jumlah_nama_tidak_cocok: Option<JumlahNamaTidakCocok>,
    /// Jenis objek yang nonaktif; hanya pada `objek_nonaktif`, tidak pernah `null`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub jenis_objek: Option<JenisObjek>,
    /// Status API terkini; hanya pada 409 `tidak_bisa_dibatalkan`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_api: Option<ApiStatus>,
    /// `queue_id` permintaan yang mengalami kegagalan (body 502/504).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<Uuid>,
    /// Kode `error` menurut RFC 6749 untuk token endpoint. Tidak pernah dikirim
    /// bersama 429.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Problem {
    pub(crate) fn new(kode: ProblemKodeDown, status: u16) -> Self {
        Self {
            type_: kode.type_url(),
            title: kode.title().into(),
            status,
            kode,
            detail: None,
            errors: None,
            jumlah_nama_tidak_cocok: None,
            jenis_objek: None,
            status_api: None,
            request_id: None,
            error: None,
        }
    }

    /// Placeholder body of the 501 handler stubs.
    pub fn stub() -> Self {
        Self::new(ProblemKodeDown::ValidasiGagal, 501)
    }
}

impl Default for Problem {
    fn default() -> Self {
        Self::stub()
    }
}
