//! Hasil satu percobaan dan status berikutnya (ADR-0001 bagian 3, 5a.3;
//! ADR-0002 D3). Murni: tanpa database, tanpa jam, tanpa jaringan.
//!
//! Dua langkah:
//!
//! 1. [`nilai_hasil`]: panggilan → `status_akses`, `error_class`, kode HTTP,
//!    dan output bila sukses. Kelas memakai tabel `core::retry` lewat
//!    `adapter_provider::kelas_error` (ARC-4); parser `status_skor` lewat
//!    `adapter_provider::baca_respons`.
//! 2. [`status_final`]: hasil + kebijakan pool → `done`, `failed` dengan
//!    `failure_reason`, atau kembali `queued` dengan `not_before` (full jitter).

use chrono::{DateTime, Utc};
use poolrouter_core::retry::{ErrorClass, KebijakanBackoff, KeputusanRetry, SumberAcakBackoff};
use serde_json::Value;

// Langkah 1 dipindah ke adapter (ARC-66) dan dipakai juga oleh jalur sync
// webserver; re-export menjaga path `crate::keputusan::*`.
pub use poolrouter_adapter_provider::hasil::{Akhir, nilai_hasil};

/// Status request setelah percobaan.
#[derive(Debug, Clone, PartialEq)]
pub enum StatusFinal {
    /// `done` dengan `output_payload`.
    Done(Value),
    /// `failed` dengan `failure_reason`.
    Gagal(&'static str),
    /// Kembali `queued` dengan `not_before` (async saja).
    Antre(DateTime<Utc>),
}

/// Putuskan status berikutnya. `nomor` adalah `attempt_count` setelah
/// percobaan ini; `ttl` adalah `ttl_expires_at` request.
pub fn status_final(
    akhir: &Akhir,
    kebijakan: &KebijakanBackoff,
    nomor: u32,
    sekarang: DateTime<Utc>,
    ttl: DateTime<Utc>,
    acak: &dyn SumberAcakBackoff,
) -> StatusFinal {
    if let (None, Some(output)) = (akhir.error_class, &akhir.output) {
        return StatusFinal::Done(output.clone());
    }
    let kelas = akhir.error_class.unwrap_or(ErrorClass::Lainnya);
    match kebijakan.putuskan(kelas, nomor, sekarang, ttl, acak) {
        KeputusanRetry::Jadwalkan(not_before) => StatusFinal::Antre(not_before),
        KeputusanRetry::BatasPercobaanHabis | KeputusanRetry::MelewatiTtl => {
            StatusFinal::Gagal("retry_habis")
        }
        KeputusanRetry::TidakDiretry => StatusFinal::Gagal("provider_error_permanen"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{KonfigurasiRespons, baca_respons_skema};
    use crate::upstream::ResponsMentah;
    use poolrouter_adapter_provider::GagalPanggilProvider;
    use std::collections::BTreeMap;
    use chrono::{Duration, TimeZone};
    use poolrouter_core::retry::AcakBatasAtas;
    use serde_json::json;

    fn konf() -> KonfigurasiRespons {
        baca_respons_skema(&json!({
            "parser": "status_skor",
            "literal": [
                {"teks": "Sesuai", "status": "sesuai"},
                {"teks": "Tidak Sesuai", "status": "tidak_sesuai"}
            ],
            "parameter_output": [{"nama": "NAMA_LGKP"}, {"nama": "TMPT_LHR"}]
        }))
        .unwrap()
    }

    fn input(pasangan: &[(&str, &str)]) -> BTreeMap<String, String> {
        pasangan
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn ok(status: u16, body: Value) -> Result<ResponsMentah, GagalPanggilProvider> {
        Ok(ResponsMentah {
            status,
            body: body.to_string().into_bytes(),
        })
    }

    fn t(detik: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000 + detik, 0).unwrap()
    }

    #[test]
    fn sukses_membuang_elemen_yang_tidak_dikirim_client() {
        let body = json!({"content": [{
            "NAMA_LGKP": "Sesuai (90)", "TMPT_LHR": "Tidak Sesuai", "NIK": "Sesuai"
        }]});
        let a = nilai_hasil(&ok(200, body), &konf(), &input(&[("NAMA_LGKP", "BUDI")]));
        assert_eq!(a.status_akses, "berhasil");
        assert_eq!(a.error_class, None);
        assert_eq!(
            a.output,
            Some(json!({"NAMA_LGKP": {"status": "sesuai", "skor": 90}}))
        );
    }

    #[test]
    fn respons_tidak_valid_tidak_diretry() {
        let a = nilai_hasil(&ok(200, json!({"content": []})), &konf(), &input(&[]));
        assert_eq!(a.status_akses, "gagal_provider");
        assert_eq!(a.error_class, Some(ErrorClass::ResponsTidakValid));
        let s = status_final(
            &a,
            &KebijakanBackoff::default(),
            1,
            t(0),
            t(3600),
            &AcakBatasAtas,
        );
        assert_eq!(s, StatusFinal::Gagal("provider_error_permanen"));
    }

    #[test]
    fn http_dipetakan_ke_tabel_error_class() {
        let kasus = [
            (401, ErrorClass::AuthDitolak),
            (429, ErrorClass::RateLimitProvider),
            (503, ErrorClass::ProviderSementara),
            (404, ErrorClass::Lainnya),
        ];
        for (kode, kelas) in kasus {
            let a = nilai_hasil(&ok(kode, json!({})), &konf(), &input(&[]));
            assert_eq!(a.error_class, Some(kelas), "{kode}");
            assert_eq!(a.status_akses, "gagal_provider");
            assert_eq!(a.http_status, Some(kode));
            assert!(a.ada_respons);
        }
    }

    #[test]
    fn transport_gagal_jaringan_tanpa_respons() {
        let a = nilai_hasil(&Err(GagalPanggilProvider::Timeout), &konf(), &input(&[]));
        assert_eq!(a.status_akses, "gagal_jaringan");
        assert_eq!(a.error_class, Some(ErrorClass::Timeout));
        assert!(!a.ada_respons);
        let tls = nilai_hasil(
            &Err(GagalPanggilProvider::Tls("x".into())),
            &konf(),
            &input(&[]),
        );
        assert_eq!(tls.error_class, Some(ErrorClass::Tls));
        assert_eq!(tls.http_status, None);
    }

    #[test]
    fn retry_full_jitter_batas_atas_dan_batasnya() {
        let p = KebijakanBackoff::default();
        let a = nilai_hasil(&ok(503, json!({})), &konf(), &input(&[]));
        // Stub batas atas: n = 2 → min(300, 5 × 2) = 10 s.
        assert_eq!(
            status_final(&a, &p, 2, t(0), t(3600), &AcakBatasAtas),
            StatusFinal::Antre(t(0) + Duration::seconds(10))
        );
        // Batas percobaan pool tercapai.
        assert_eq!(
            status_final(&a, &p, 3, t(0), t(3600), &AcakBatasAtas),
            StatusFinal::Gagal("retry_habis")
        );
        // not_before >= ttl_expires_at.
        assert_eq!(
            status_final(&a, &p, 1, t(0), t(5), &AcakBatasAtas),
            StatusFinal::Gagal("retry_habis")
        );
    }
}
