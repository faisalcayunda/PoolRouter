//! Uji tabel parser `status_skor` (contract U-41; ADR-0002 rev 5 D3).
//!
//! Hanya seam publik `respons::status_skor::parse_respons` yang disentuh,
//! sesuai Testing Decisions spec `pilot-d49-status-skor`. Body JSON sintetis
//! dengan NIK berprefiks `00` (ADR-0001 rev 15 bagian 7), tanpa mock.

use std::collections::BTreeMap;

use poolrouter_core::respons::status_skor::{
    HasilElemen, KonfigurasiStatusSkor, LiteralStatus, ResponsTidakValid, StatusKesesuaian,
    parse_respons,
};
use serde_json::{Value, json};

const NAMA_LGKP: &str = "NAMA_LGKP";
const ALAMAT: &str = "ALAMAT";

/// Konfigurasi provider Dukcapil: dua literal, dua parameter output.
fn konf() -> KonfigurasiStatusSkor {
    KonfigurasiStatusSkor {
        literal: vec![
            LiteralStatus::baru("Sesuai", StatusKesesuaian::Sesuai),
            LiteralStatus::baru("Tidak Sesuai", StatusKesesuaian::TidakSesuai),
        ],
        parameter_output: vec![NAMA_LGKP.to_string(), ALAMAT.to_string()],
    }
}

/// Envelope Dukcapil: `content` berisi tepat satu objek. Kunci `NIK` selalu
/// ikut walau tidak ada di `parameter_output`, seperti respons asli.
fn envelope(nama_lgkp: Value, alamat: Value) -> Value {
    json!({
        "content": [{
            "NIK": "0000000000000001",
            NAMA_LGKP: nama_lgkp,
            ALAMAT: alamat,
        }]
    })
}

fn parse(
    nama_lgkp: Value,
    alamat: Value,
) -> Result<BTreeMap<String, HasilElemen>, ResponsTidakValid> {
    parse_respons(&konf(), &envelope(nama_lgkp, alamat))
}

#[test]
fn tabel_bentuk_valid_menghasilkan_nilai_benar() {
    let sesuai = StatusKesesuaian::Sesuai;
    let tidak_sesuai = StatusKesesuaian::TidakSesuai;

    // (nama kasus, teks NAMA_LGKP, teks ALAMAT, hasil keduanya)
    let kasus = [
        (
            "literal saja",
            "Sesuai",
            "Sesuai",
            [sesuai, sesuai],
            [None, None],
        ),
        (
            "literal tidak sesuai",
            "Tidak Sesuai",
            "Tidak Sesuai",
            [tidak_sesuai, tidak_sesuai],
            [None, None],
        ),
        (
            "skor 100",
            "Sesuai (100)",
            "Sesuai (100)",
            [sesuai, sesuai],
            [Some(100), Some(100)],
        ),
        (
            "skor 0",
            "Tidak Sesuai (0)",
            "Tidak Sesuai (0)",
            [tidak_sesuai, tidak_sesuai],
            [Some(0), Some(0)],
        ),
        (
            "spasi di tepi",
            "  Sesuai (50)  ",
            "\tTidak Sesuai \n",
            [sesuai, tidak_sesuai],
            [Some(50), None],
        ),
    ];

    for (nama, teks_nama, teks_alamat, status, skor) in kasus {
        let hasil = parse(json!(teks_nama), json!(teks_alamat))
            .unwrap_or_else(|galat| panic!("{nama}: seharusnya valid, dapat {galat}"));

        let diharapkan = [
            HasilElemen {
                status: status[0],
                skor: skor[0],
            },
            HasilElemen {
                status: status[1],
                skor: skor[1],
            },
        ];
        assert_eq!(
            hasil.get(NAMA_LGKP),
            Some(&diharapkan[0]),
            "{nama}: NAMA_LGKP"
        );
        assert_eq!(hasil.get(ALAMAT), Some(&diharapkan[1]), "{nama}: ALAMAT");
    }
}

#[test]
fn delapan_bentuk_tidak_valid_ditolak_dengan_sebab_yang_tepat() {
    let elemen_dikenal = || ResponsTidakValid::NilaiTidakDikenal(NAMA_LGKP.to_string());

    let kasus = [
        (
            "skor di atas 100",
            envelope(json!("Sesuai (101)"), json!("Sesuai")),
            ResponsTidakValid::SkorDiLuarRentang(NAMA_LGKP.to_string()),
        ),
        (
            "teks di luar literal",
            envelope(json!("Cocok"), json!("Sesuai")),
            elemen_dikenal(),
        ),
        (
            "teks kosong",
            envelope(json!(""), json!("Sesuai")),
            elemen_dikenal(),
        ),
        (
            "content kosong",
            json!({ "content": [], "NIK": "0000000000000001" }),
            ResponsTidakValid::Envelope,
        ),
        (
            "content dua elemen",
            json!({ "content": [
                { NAMA_LGKP: "Sesuai", ALAMAT: "Sesuai" },
                { NAMA_LGKP: "Sesuai", ALAMAT: "Sesuai" },
            ] }),
            ResponsTidakValid::Envelope,
        ),
        (
            "content bukan array",
            json!({ "content": { NAMA_LGKP: "Sesuai", ALAMAT: "Sesuai" } }),
            ResponsTidakValid::Envelope,
        ),
        (
            "elemen output hilang",
            json!({ "content": [{ NAMA_LGKP: "Sesuai" }] }),
            ResponsTidakValid::ElemenHilang(ALAMAT.to_string()),
        ),
        (
            "nilai bukan string",
            envelope(json!("Sesuai"), json!(100)),
            ResponsTidakValid::NilaiTidakDikenal(ALAMAT.to_string()),
        ),
    ];

    for (nama, body, diharapkan) in kasus {
        assert_eq!(
            parse_respons(&konf(), &body),
            Err(diharapkan),
            "{nama}: sebab penolakan harus tepat"
        );
    }
}

#[test]
fn alamat_rusak_membatalkan_seluruh_respons() {
    // NAMA_LGKP valid, ALAMAT tidak: tidak boleh ada output sebagian.
    assert_eq!(
        parse(json!("Sesuai"), json!("Cocok")),
        Err(ResponsTidakValid::NilaiTidakDikenal(ALAMAT.to_string()))
    );
}

#[test]
fn bentuk_skor_menyimpang_ditolak() {
    for teks in [
        "Sesuai ()",
        "Sesuai ( )",
        "Sesuai (-1)",
        "Sesuai (+5)",
        "Sesuai (050)",
        "Sesuai (5",
        "Sesuai 5)",
        "Sesuai (5) x",
        // Bagian sebelum kurung kosong: jangan terbaca sebagai elemen kosong.
        "(5)",
    ] {
        let galat = parse(json!(teks), json!("Sesuai")).unwrap_err();
        assert!(
            matches!(
                galat,
                ResponsTidakValid::NilaiTidakDikenal(_) | ResponsTidakValid::SkorDiLuarRentang(_)
            ),
            "{teks}: seharusnya ditolak, dapat {galat}"
        );
    }
}

#[test]
fn kunci_di_luar_parameter_output_tidak_muncul() {
    let hasil = parse(json!("Sesuai"), json!("Tidak Sesuai")).unwrap();

    let mut diharapkan = konf().parameter_output;
    diharapkan.sort();
    assert_eq!(
        hasil.keys().cloned().collect::<Vec<_>>(),
        diharapkan,
        "output hanya berisi parameter_output"
    );
    assert!(
        !hasil.contains_key("NIK"),
        "NIK tidak boleh ikut: {hasil:?}"
    );
}

#[test]
fn pesan_galat_tidak_memuat_nilai_elemen() {
    // Nilai penanda yang mudah dicari; galat tidak boleh membawanya.
    let penanda = "Cocok-0000000000000001";
    let galat = parse(json!(penanda), json!("Sesuai")).unwrap_err();
    let pesan = galat.to_string();

    assert!(
        !pesan.contains(penanda) && !pesan.contains("0000000000000001"),
        "pesan galat memuat isi respons: {pesan}"
    );
}

#[test]
fn literal_konfigurasi_bertrepi_tetap_cocok() {
    let konf = KonfigurasiStatusSkor {
        literal: vec![LiteralStatus::baru("  Sesuai  ", StatusKesesuaian::Sesuai)],
        parameter_output: vec![NAMA_LGKP.to_string()],
    };

    let hasil = parse_respons(&konf, &envelope(json!("Sesuai (7)"), json!("Sesuai"))).unwrap();

    assert_eq!(
        hasil.get(NAMA_LGKP),
        Some(&HasilElemen {
            status: StatusKesesuaian::Sesuai,
            skor: Some(7),
        })
    );
}
