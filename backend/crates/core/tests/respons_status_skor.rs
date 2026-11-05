//! Uji tabel parser `status_skor` (contract U-41; ADR-0002 rev 5 D3).
//!
//! Hanya seam publik `respons::status_skor::parse_respons` yang disentuh,
//! sesuai Testing Decisions spec `pilot-d49-status-skor`. Body JSON sintetis,
//! tanpa mock.

use std::collections::BTreeMap;

use poolrouter_core::respons::status_skor::{
    HasilElemen, KonfigurasiStatusSkor, ResponsTidakValid, StatusKesesuaian, parse_respons,
};
use serde_json::{Value, json};

/// Konfigurasi provider Dukcapil: dua literal, dua parameter output.
fn konf() -> KonfigurasiStatusSkor {
    KonfigurasiStatusSkor {
        literal: vec![
            ("Sesuai".to_string(), StatusKesesuaian::Sesuai),
            ("Tidak Sesuai".to_string(), StatusKesesuaian::TidakSesuai),
        ],
        parameter_output: vec!["NAMA_LGKP".to_string(), "ALAMAT".to_string()],
    }
}

/// Envelope Dukcapil: `content` berisi tepat satu objek. Kunci `NIK` selalu ikut
/// walau tidak ada di `parameter_output`, seperti respons asli.
fn envelope(nama_lgkp: Value, alamat: Value) -> Value {
    json!({
        "content": [{
            "NIK": "3273010101010001",
            "NAMA_LGKP": nama_lgkp,
            "ALAMAT": alamat,
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
    // (nama kasus, teks NAMA_LGKP, teks ALAMAT, hasil keduanya)
    struct Kasus {
        nama: &'static str,
        nama_lgkp: &'static str,
        alamat: &'static str,
        hasil: [HasilElemen; 2],
    }

    let sesuai = StatusKesesuaian::Sesuai;
    let tidak_sesuai = StatusKesesuaian::TidakSesuai;
    let kasus = [
        Kasus {
            nama: "literal saja",
            nama_lgkp: "Sesuai",
            alamat: "Sesuai",
            hasil: [
                HasilElemen {
                    status: sesuai,
                    skor: None,
                },
                HasilElemen {
                    status: sesuai,
                    skor: None,
                },
            ],
        },
        Kasus {
            nama: "literal tidak sesuai",
            nama_lgkp: "Tidak Sesuai",
            alamat: "Tidak Sesuai",
            hasil: [
                HasilElemen {
                    status: tidak_sesuai,
                    skor: None,
                },
                HasilElemen {
                    status: tidak_sesuai,
                    skor: None,
                },
            ],
        },
        Kasus {
            nama: "skor 100",
            nama_lgkp: "Sesuai (100)",
            alamat: "Sesuai (100)",
            hasil: [
                HasilElemen {
                    status: sesuai,
                    skor: Some(100),
                },
                HasilElemen {
                    status: sesuai,
                    skor: Some(100),
                },
            ],
        },
        Kasus {
            nama: "skor 0",
            nama_lgkp: "Tidak Sesuai (0)",
            alamat: "Tidak Sesuai (0)",
            hasil: [
                HasilElemen {
                    status: tidak_sesuai,
                    skor: Some(0),
                },
                HasilElemen {
                    status: tidak_sesuai,
                    skor: Some(0),
                },
            ],
        },
        Kasus {
            nama: "spasi di tepi",
            nama_lgkp: "  Sesuai (50)  ",
            alamat: "\tTidak Sesuai \n",
            hasil: [
                HasilElemen {
                    status: sesuai,
                    skor: Some(50),
                },
                HasilElemen {
                    status: tidak_sesuai,
                    skor: None,
                },
            ],
        },
    ];

    for kasus in kasus {
        let hasil = parse(json!(kasus.nama_lgkp), json!(kasus.alamat))
            .unwrap_or_else(|galat| panic!("{}: seharusnya valid, dapat {galat}", kasus.nama));

        assert_eq!(
            hasil.get("NAMA_LGKP"),
            Some(&kasus.hasil[0]),
            "{}: NAMA_LGKP",
            kasus.nama
        );
        assert_eq!(
            hasil.get("ALAMAT"),
            Some(&kasus.hasil[1]),
            "{}: ALAMAT",
            kasus.nama
        );
    }
}

#[test]
fn delapan_bentuk_tidak_valid_ditolak() {
    let kasus: [(&str, Value); 8] = [
        (
            "skor di atas 100",
            envelope(json!("Sesuai (101)"), json!("Sesuai")),
        ),
        (
            "teks di luar literal",
            envelope(json!("Cocok"), json!("Sesuai")),
        ),
        ("teks kosong", envelope(json!(""), json!("Sesuai"))),
        (
            "content kosong",
            json!({ "content": [], "NIK": "3273010101010001" }),
        ),
        (
            "content dua elemen",
            json!({ "content": [
                { "NAMA_LGKP": "Sesuai", "ALAMAT": "Sesuai" },
                { "NAMA_LGKP": "Sesuai", "ALAMAT": "Sesuai" },
            ] }),
        ),
        (
            "content bukan array",
            json!({ "content": { "NAMA_LGKP": "Sesuai", "ALAMAT": "Sesuai" } }),
        ),
        (
            "elemen output hilang",
            json!({ "content": [{ "NAMA_LGKP": "Sesuai" }] }),
        ),
        ("nilai bukan string", envelope(json!("Sesuai"), json!(100))),
    ];

    for (nama, body) in kasus {
        let hasil = parse_respons(&konf(), &body);
        assert!(
            hasil.is_err(),
            "{nama}: seharusnya ditolak, dapat {hasil:?}"
        );
    }
}

#[test]
fn satu_elemen_rusak_membatalkan_seluruh_respons() {
    // ALAMAT valid, NAMA_LGKP tidak: tidak boleh ada output sebagian.
    let hasil = parse(json!("Cocok"), json!("Sesuai"));

    assert_eq!(
        hasil,
        Err(ResponsTidakValid::NilaiTidakDikenal(
            "NAMA_LGKP".to_string()
        ))
    );
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
    let penanda = "COCOK-3273010101010001";
    let galat = parse(json!(penanda), json!("Sesuai")).unwrap_err();
    let pesan = galat.to_string();

    assert!(
        !pesan.contains(penanda) && !pesan.contains("3273010101010001"),
        "pesan galat memuat isi respons: {pesan}"
    );
}

#[test]
fn literal_berawalan_sama_dibaca_utuh() {
    // `Tidak Sesuai` tidak boleh terbaca sebagai `Sesuai`.
    let hasil = parse(json!("Tidak Sesuai (12)"), json!("Sesuai (0)")).unwrap();

    assert_eq!(
        hasil.get("NAMA_LGKP"),
        Some(&HasilElemen {
            status: StatusKesesuaian::TidakSesuai,
            skor: Some(12),
        })
    );
}

#[test]
fn literal_konfigurasi_bertrepi_tetap_cocok() {
    let konf = KonfigurasiStatusSkor {
        literal: vec![
            (" Sesuai ".to_string(), StatusKesesuaian::Sesuai),
            ("Tidak Sesuai".to_string(), StatusKesesuaian::TidakSesuai),
        ],
        parameter_output: vec!["NAMA_LGKP".to_string()],
    };

    let hasil = parse_respons(&konf, &envelope(json!("Sesuai (7)"), json!("Sesuai"))).unwrap();

    assert_eq!(
        hasil.get("NAMA_LGKP"),
        Some(&HasilElemen {
            status: StatusKesesuaian::Sesuai,
            skor: Some(7),
        })
    );
}
