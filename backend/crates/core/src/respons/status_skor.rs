//! Parser respons `status_skor` untuk provider Dukcapil (ADR-0002 rev 5 D3,
//! contract U-41).
//!
//! Respons Web Service Kesesuaian membungkus satu objek biodata di dalam
//! `content[]`. Nilai tiap elemen berupa teks seperti `Sesuai (100)` atau
//! `Tidak Sesuai`; skor dalam tanda kurung opsional. Literal status tidak
//! ditanam di sini: pemanggil memberikannya dari konfigurasi provider.
//!
//! Tiga aturan membentuk modul ini:
//!
//! - **Fail-closed.** Envelope yang meleset dari bentuk yang disepakati,
//!   elemen yang hilang, nilai yang bukan teks, atau teks di luar literal
//!   dikembalikan sebagai [`ResponsTidakValid`]. Tidak ada output parsial:
//!   pemanggil hanya menerima peta lengkap atau galat.
//! - **Hanya `parameter_output`.** Kunci lain pada objek biodata (misalnya
//!   `NIK`) tidak pernah ikut ke output, walau ikut terkirim Dukcapil
//!   (ADR-0002 D3, contract O-04).
//! - **Galat tanpa isi respons.** Pesan galat menyebut sebab dan nama
//!   parameter, tidak pernah nilai elemen, karena log dan problem detail
//!   tidak boleh membawa data kependudukan (ADR-0001 rev 15 bagian 11).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Status kesesuaian satu elemen biodata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusKesesuaian {
    Sesuai,
    TidakSesuai,
}

/// Hasil baca satu elemen biodata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HasilElemen {
    pub status: StatusKesesuaian,
    /// Skor dalam tanda kurung; `None` bila literal dikirim tanpa skor.
    pub skor: Option<u8>,
}

/// Konfigurasi parser per provider (ADR-0002 rev 5 D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KonfigurasiStatusSkor {
    /// Pasangan teks Dukcapil dengan kode status PoolRouter.
    pub literal: Vec<(String, StatusKesesuaian)>,
    /// Kunci yang diambil dari `content[0]` dan ikut ke output.
    pub parameter_output: Vec<String>,
}

/// Sebab respons ditolak. Satu elemen gagal berarti seluruh respons gagal.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ResponsTidakValid {
    #[error("respons_tidak_valid: envelope")]
    Envelope,
    #[error("respons_tidak_valid: elemen_hilang {0}")]
    ElemenHilang(String),
    #[error("respons_tidak_valid: nilai_tidak_dikenal {0}")]
    NilaiTidakDikenal(String),
    #[error("respons_tidak_valid: skor_di_luar_rentang {0}")]
    SkorDiLuarRentang(String),
}

/// Baca `parameter_output` dari `content[0]` sesuai literal provider.
pub fn parse_respons(
    konf: &KonfigurasiStatusSkor,
    body: &Value,
) -> Result<BTreeMap<String, HasilElemen>, ResponsTidakValid> {
    let objek = objek_tunggal(body)?;

    konf.parameter_output
        .iter()
        .map(|nama| {
            let nilai = objek
                .get(nama)
                .ok_or_else(|| ResponsTidakValid::ElemenHilang(nama.clone()))?;
            let teks = nilai
                .as_str()
                .ok_or_else(|| ResponsTidakValid::NilaiTidakDikenal(nama.clone()))?;
            Ok((nama.clone(), urai_elemen(nama, teks, &konf.literal)?))
        })
        .collect()
}

/// `content` wajib array berisi tepat satu objek.
fn objek_tunggal(body: &Value) -> Result<&serde_json::Map<String, Value>, ResponsTidakValid> {
    let content = body
        .get("content")
        .and_then(Value::as_array)
        .ok_or(ResponsTidakValid::Envelope)?;
    if content.len() != 1 {
        return Err(ResponsTidakValid::Envelope);
    }
    content[0].as_object().ok_or(ResponsTidakValid::Envelope)
}

/// Urai satu teks elemen menjadi status + skor opsional. Literal terpanjang
/// dicoba lebih dulu, supaya `Tidak Sesuai` tidak pernah terbaca sebagai
/// `Sesuai` bila provider mendaftarkan keduanya dengan awalan sama.
fn urai_elemen(
    nama: &str,
    teks: &str,
    literal: &[(String, StatusKesesuaian)],
) -> Result<HasilElemen, ResponsTidakValid> {
    let teks = teks.trim();
    let (isi, skor) = pisah_skor(nama, teks)?;

    literal
        .iter()
        .filter(|(teks_literal, _)| teks_literal.trim() == isi)
        .max_by_key(|(teks_literal, _)| teks_literal.trim().len())
        .map(|(_, status)| HasilElemen {
            status: *status,
            skor,
        })
        .ok_or_else(|| ResponsTidakValid::NilaiTidakDikenal(nama.to_string()))
}

/// Pisahkan `Tidak Sesuai (0)` menjadi `("Tidak Sesuai", Some(0))`. Teks tanpa
/// tanda kurung dikembalikan apa adanya dengan skor `None`; tanda kurung yang
/// tidak utuh atau angka di luar 0–100 ditolak.
fn pisah_skor<'a>(nama: &str, teks: &'a str) -> Result<(&'a str, Option<u8>), ResponsTidakValid> {
    let Some(awal) = teks.find(" (") else {
        return if teks.contains(['(', ')']) {
            Err(ResponsTidakValid::NilaiTidakDikenal(nama.to_string()))
        } else {
            Ok((teks, None))
        };
    };
    let angka = teks[awal + 2..]
        .strip_suffix(')')
        .ok_or_else(|| ResponsTidakValid::NilaiTidakDikenal(nama.to_string()))?;
    // `u8::from_str` menerima tanda plus dan nol di depan; keduanya bukan
    // bentuk yang dikirim Dukcapil.
    if angka.is_empty() || !angka.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ResponsTidakValid::NilaiTidakDikenal(nama.to_string()));
    }
    let skor: u8 = angka
        .parse()
        .map_err(|_| ResponsTidakValid::SkorDiLuarRentang(nama.to_string()))?;
    if skor > 100 {
        return Err(ResponsTidakValid::SkorDiLuarRentang(nama.to_string()));
    }
    if angka != skor.to_string() {
        return Err(ResponsTidakValid::SkorDiLuarRentang(nama.to_string()));
    }

    Ok((&teks[..awal], Some(skor)))
}
