//! Response reading tests (ADR-0002 D3, AC-21).
//!
//! The rules themselves are the pilot parser's; these tests pin the adapter
//! seam: bytes in, elements out, one bad element fails everything.

use poolrouter_adapter_provider::respons::{BacaResponsError, baca_respons};
use poolrouter_core::respons::{KonfigurasiStatusSkor, LiteralStatus, StatusKesesuaian};

fn konfigurasi() -> KonfigurasiStatusSkor {
    KonfigurasiStatusSkor {
        literal: vec![
            LiteralStatus::baru("SESUAI", StatusKesesuaian::Sesuai),
            LiteralStatus::baru("TIDAK SESUAI", StatusKesesuaian::TidakSesuai),
        ],
        parameter_output: vec!["NAMA_LGKP".to_string(), "TGL_LAHIR".to_string()],
    }
}

#[test]
fn membaca_elemen_sesuai_dengan_skor() {
    let body = br#"{"content":[{"NAMA_LGKP":"SESUAI (95)","TGL_LAHIR":"SESUAI (90)"}]}"#;

    let hasil = baca_respons(body, &konfigurasi()).expect("harus berhasil");

    assert_eq!(hasil.len(), 2);
    assert_eq!(hasil[0].elemen, "NAMA_LGKP");
    assert_eq!(hasil[0].hasil.status, StatusKesesuaian::Sesuai);
    assert_eq!(hasil[0].hasil.skor, Some(95));
    assert_eq!(hasil[1].elemen, "TGL_LAHIR");
}

#[test]
fn membaca_literal_tanpa_skor() {
    let body = br#"{"content":[{"NAMA_LGKP":"TIDAK SESUAI","TGL_LAHIR":"SESUAI"}]}"#;

    let hasil = baca_respons(body, &konfigurasi()).expect("harus berhasil");

    assert_eq!(hasil[0].hasil.status, StatusKesesuaian::TidakSesuai);
    assert_eq!(hasil[0].hasil.skor, None);
}

#[test]
fn satu_elemen_hilang_menggagalkan_semuanya() {
    // AC-21 / U-43: no partial output.
    let body = br#"{"content":[{"NAMA_LGKP":"SESUAI (95)"}]}"#;

    let galat = baca_respons(body, &konfigurasi()).expect_err("harus gagal");
    match galat {
        BacaResponsError::TidakValid(e) => {
            assert_eq!(
                e.to_string(),
                "respons_tidak_valid: elemen_hilang TGL_LAHIR"
            );
        }
        lain => panic!("kelas galat salah: {lain:?}"),
    }
}

#[test]
fn nilai_tidak_dikenal_menggagalkan_semuanya() {
    let body = br#"{"content":[{"NAMA_LGKP":"MUNGKIN","TGL_LAHIR":"SESUAI"}]}"#;

    let galat = baca_respons(body, &konfigurasi()).expect_err("harus gagal");
    assert!(matches!(galat, BacaResponsError::TidakValid(_)));
}

#[test]
fn envelope_salah_menggagalkan_semuanya() {
    let body = br#"{"hasil":{"NAMA_LGKP":"SESUAI"}}"#;

    let galat = baca_respons(body, &konfigurasi()).expect_err("harus gagal");
    assert!(matches!(galat, BacaResponsError::TidakValid(_)));
}

#[test]
fn bukan_json_dilaporkan_terpisah() {
    let galat = baca_respons(b"<html>502</html>", &konfigurasi()).expect_err("harus gagal");
    assert_eq!(galat, BacaResponsError::BukanJson);
}

#[test]
fn galat_tidak_membocorkan_nilai_elemen() {
    // Display names the parameter, never the provider value it carried.
    let body = br#"{"content":[{"NAMA_LGKP":"MUNGKIN","TGL_LAHIR":"SESUAI"}]}"#;

    let galat = baca_respons(body, &konfigurasi()).expect_err("harus gagal");
    let teks = galat.to_string();
    assert!(!teks.contains("MUNGKIN"), "nilai bocor: {teks}");
    assert!(teks.contains("NAMA_LGKP"));
}

#[test]
fn konfigurasi_kosong_menghasilkan_daftar_kosong() {
    let konf = KonfigurasiStatusSkor {
        literal: vec![LiteralStatus::baru("SESUAI", StatusKesesuaian::Sesuai)],
        parameter_output: vec![],
    };

    let hasil = baca_respons(br#"{"content":[{}]}"#, &konf).expect("harus berhasil");
    assert!(hasil.is_empty());
}

#[test]
fn hanya_parameter_output_yang_dibaca() {
    // Extra keys in the provider body are not passed through (D3 / P-1).
    let body = br#"{"content":[{"NAMA_LGKP":"SESUAI (95)","TGL_LAHIR":"SESUAI","NIK":"3200000000000001"}]}"#;

    let hasil = baca_respons(body, &konfigurasi()).expect("harus berhasil");

    let nama: Vec<&str> = hasil.iter().map(|h| h.elemen.as_str()).collect();
    assert_eq!(nama, vec!["NAMA_LGKP", "TGL_LAHIR"]);
}
