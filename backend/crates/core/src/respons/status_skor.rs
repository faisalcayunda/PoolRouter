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
//!   parameter, tidak pernah nilai elemen, karena log dan trace tidak boleh
//!   membawa data kependudukan (ADR-0001 rev 15 bagian 7, NFR-08).

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

/// Satu pasangan teks Dukcapil dengan kode status PoolRouter. `teks`
/// dibandingkan persis (case-sensitive) setelah dipangkas spasinya. Seperti
/// literal, bagian teks elemen juga dibandingkan tanpa spasi tepi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiteralStatus {
    pub teks: String,
    pub status: StatusKesesuaian,
}

impl LiteralStatus {
    pub fn baru(teks: impl Into<String>, status: StatusKesesuaian) -> Self {
        Self {
            teks: teks.into(),
            status,
        }
    }
}

/// Konfigurasi parser per provider (ADR-0002 rev 5 D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KonfigurasiStatusSkor {
    pub literal: Vec<LiteralStatus>,
    /// Kunci yang diambil dari `content[0]` dan ikut ke output.
    pub parameter_output: Vec<String>,
}

/// Sebab respons ditolak. Satu elemen gagal berarti seluruh respons gagal,
/// dan `Display` hanya menyebut nama parameter, bukan nilainya.
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

impl ResponsTidakValid {
    fn tidak_dikenal(nama: &str) -> Self {
        Self::NilaiTidakDikenal(nama.to_string())
    }

    fn skor_di_luar_rentang(nama: &str) -> Self {
        Self::SkorDiLuarRentang(nama.to_string())
    }
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
                .ok_or_else(|| ResponsTidakValid::tidak_dikenal(nama))?;
            let hasil = urai_elemen(nama, teks, &konf.literal)?;
            Ok((nama.clone(), hasil))
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

/// Urai satu teks elemen menjadi status + skor opsional.
fn urai_elemen(
    nama: &str,
    teks: &str,
    literal: &[LiteralStatus],
) -> Result<HasilElemen, ResponsTidakValid> {
    let (isi, skor) = pisah_skor(nama, teks.trim())?;

    literal
        .iter()
        .find(|l| l.teks.trim() == isi)
        .map(|l| HasilElemen {
            status: l.status,
            skor,
        })
        .ok_or_else(|| ResponsTidakValid::tidak_dikenal(nama))
}

/// Pisahkan `Tidak Sesuai (0)` menjadi `("Tidak Sesuai", Some(0))`. Bila teks
/// ditutup tanda kurung, bagian dalamnya wajib skor berbentuk `0`-`100`:
///
/// - tanda kurung yang tidak utuh ditolak tanpa membaca isinya;
/// - angka dengan nol di depan atau tanda plus ditolak sebagai
///   [`ResponsTidakValid::NilaiTidakDikenal`], supaya `skor_di_luar_rentang`
///   hanya dipakai untuk skor yang memang di luar rentang.
fn pisah_skor<'a>(nama: &str, teks: &'a str) -> Result<(&'a str, Option<u8>), ResponsTidakValid> {
    let Some(sisa) = teks.strip_suffix(')') else {
        return if teks.contains(['(', ')']) {
            Err(ResponsTidakValid::tidak_dikenal(nama))
        } else {
            Ok((teks, None))
        };
    };
    let (isi, angka) = sisa
        .rsplit_once('(')
        .ok_or_else(|| ResponsTidakValid::tidak_dikenal(nama))?;
    let isi = isi.strip_suffix(' ').unwrap_or(isi);
    if isi.is_empty() || angka.contains(' ') {
        return Err(ResponsTidakValid::tidak_dikenal(nama));
    }
    let digit = angka.strip_prefix('-').unwrap_or(angka);
    if digit.is_empty()
        || !digit.bytes().all(|b| b.is_ascii_digit())
        || (digit.len() > 1 && digit.starts_with('0'))
    {
        return Err(ResponsTidakValid::tidak_dikenal(nama));
    }
    let skor: i16 = angka
        .parse()
        .map_err(|_| ResponsTidakValid::tidak_dikenal(nama))?;
    if !(0..=100).contains(&skor) {
        return Err(ResponsTidakValid::skor_di_luar_rentang(nama));
    }

    Ok((isi, Some(skor as u8)))
}
