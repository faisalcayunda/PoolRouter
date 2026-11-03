//! Problem codes (`ProblemKode`) and the anchor check against `docs/masalah.md`.
//!
//! ADR-0001 rev 15 section 5a.7: `type` = `https://github.com/faisalcayunda/
//! PoolRouter/blob/main/docs/masalah.md#<kode>`, so the anchor set in the doc
//! must equal the code set exactly. This test enforces that.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Closed set of `kode` values for `application/problem+json` bodies on the
/// downstream and admin APIs (ADR-0001 rev 15 section 5a.7, section 4.1;
/// admin codes from ADR-0003 and `backend/openapi/DESIGN.md` section 1.1).
/// `Problem.kode` is still `String` until the contract handlers land; this
/// enum is the source of truth for the docs anchor check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKode {
    // Downstream (ADR-0001 section 4.1).
    ValidasiGagal,
    MetadataTidakValid,
    IdempotencyKeyTidakDidukung,
    IdempotencyKeyKonflik,
    TidakTerAutentikasi,
    TidakDitemukan,
    ObjekNonaktif,
    KuotaHabis,
    PorsiSyncHabis,
    PenolakanBerlebih,
    TokenDibatasi,
    TidakBisaDibatalkan,
    LayananTidakTersedia,
    RetryHabis,
    ProviderErrorPermanen,
    TimeoutSync,
    // Admin (ADR-0003, backend/openapi/DESIGN.md section 1.1).
    EksporTerlaluBesar,
    KredensialSalah,
    LoginDibatasi,
    OriginTidakValid,
    AksesDitolak,
    PencarianNikDilarang,
    SecretDipakai,
    KredensialDicabut,
    BatasKredensial,
    WebhookTidakHttps,
    WebhookDomainTidakDiizinkan,
    MetadataWajibHilang,
}

impl ProblemKode {
    /// Wire value (`kode` in the problem body; anchor in `docs/masalah.md`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ValidasiGagal => "validasi_gagal",
            Self::MetadataTidakValid => "metadata_tidak_valid",
            Self::IdempotencyKeyTidakDidukung => "idempotency_key_tidak_didukung",
            Self::IdempotencyKeyKonflik => "idempotency_key_konflik",
            Self::TidakTerAutentikasi => "tidak_terautentikasi",
            Self::TidakDitemukan => "tidak_ditemukan",
            Self::ObjekNonaktif => "objek_nonaktif",
            Self::KuotaHabis => "kuota_habis",
            Self::PorsiSyncHabis => "porsi_sync_habis",
            Self::PenolakanBerlebih => "penolakan_berlebih",
            Self::TokenDibatasi => "token_dibatasi",
            Self::TidakBisaDibatalkan => "tidak_bisa_dibatalkan",
            Self::LayananTidakTersedia => "layanan_tidak_tersedia",
            Self::RetryHabis => "retry_habis",
            Self::ProviderErrorPermanen => "provider_error_permanen",
            Self::TimeoutSync => "timeout_sync",
            Self::EksporTerlaluBesar => "ekspor_terlalu_besar",
            Self::KredensialSalah => "kredensial_salah",
            Self::LoginDibatasi => "login_dibatasi",
            Self::OriginTidakValid => "origin_tidak_valid",
            Self::AksesDitolak => "akses_ditolak",
            Self::PencarianNikDilarang => "pencarian_nik_dilarang",
            Self::SecretDipakai => "secret_dipakai",
            Self::KredensialDicabut => "kredensial_dicabut",
            Self::BatasKredensial => "batas_kredensial",
            Self::WebhookTidakHttps => "webhook_tidak_https",
            Self::WebhookDomainTidakDiizinkan => "webhook_domain_tidak_diizinkan",
            Self::MetadataWajibHilang => "metadata_wajib_hilang",
        }
    }

    pub const ALL: [Self; 28] = [
        Self::ValidasiGagal,
        Self::MetadataTidakValid,
        Self::IdempotencyKeyTidakDidukung,
        Self::IdempotencyKeyKonflik,
        Self::TidakTerAutentikasi,
        Self::TidakDitemukan,
        Self::ObjekNonaktif,
        Self::KuotaHabis,
        Self::PorsiSyncHabis,
        Self::PenolakanBerlebih,
        Self::TokenDibatasi,
        Self::TidakBisaDibatalkan,
        Self::LayananTidakTersedia,
        Self::RetryHabis,
        Self::ProviderErrorPermanen,
        Self::TimeoutSync,
        Self::EksporTerlaluBesar,
        Self::KredensialSalah,
        Self::LoginDibatasi,
        Self::OriginTidakValid,
        Self::AksesDitolak,
        Self::PencarianNikDilarang,
        Self::SecretDipakai,
        Self::KredensialDicabut,
        Self::BatasKredensial,
        Self::WebhookTidakHttps,
        Self::WebhookDomainTidakDiizinkan,
        Self::MetadataWajibHilang,
    ];
}

/// The doc lives at the repository root, outside the backend crate. Inside
/// the podman toolchain container `docs/` is mounted at `/docs` (backend
/// Makefile); with `RUNNER=native` the relative path from the crate resolves.
#[cfg(test)]
fn doc_body() -> String {
    let candidates = ["/docs/masalah.md", "../../docs/masalah.md"];
    for path in candidates {
        if let Ok(body) = std::fs::read_to_string(path) {
            return body;
        }
    }
    panic!("docs/masalah.md not found at {candidates:?}");
}

#[test]
fn problem_kode_matches_docs() {
    let body = doc_body();

    let anchors: Vec<&str> = body
        .lines()
        .filter_map(|l| l.strip_prefix("### "))
        .map(|h| h.split_whitespace().next().expect("heading is not empty"))
        .collect();

    let codes: Vec<&str> = ProblemKode::ALL.iter().map(|k| k.as_str()).collect();

    let mut a = anchors.clone();
    a.sort();
    a.dedup();
    let mut b = codes.clone();
    b.sort();
    b.dedup();
    assert_eq!(a, b, "anchor set in docs/masalah.md must equal ProblemKode");

    // Every section heading must appear exactly once.
    let mut counts = std::collections::BTreeMap::new();
    for anchor in &anchors {
        *counts.entry(*anchor).or_insert(0usize) += 1;
    }
    let dupes: Vec<_> = counts.iter().filter(|(_, c)| **c > 1).collect();
    assert!(dupes.is_empty(), "duplicate anchors: {dupes:?}");
}

#[test]
fn type_uri_uses_docs_anchor() {
    let kode = ProblemKode::KuotaHabis.as_str();
    assert_eq!(
        crate::api::problem::Problem::uri(ProblemKode::KuotaHabis),
        format!("https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#{kode}")
    );
}
