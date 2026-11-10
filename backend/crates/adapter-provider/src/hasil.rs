//! Respons mentah provider dan pemetaannya ke `status_akses`/`error_class`
//! (ADR-0001 bagian 3, 5a.3; ADR-0002 D3). Murni: tanpa database, tanpa jam,
//! tanpa jaringan.
//!
//! Dipindah dari `poolrouter-worker` (`upstream::ResponsMentah` dan
//! `keputusan::nilai_hasil`, ARC-71) ke adapter (ARC-66) supaya jalur sync
//! webserver memetakan hasil dengan fungsi yang sama dengan worker. Keputusan
//! retry (`status_final`) tetap di worker karena hanya berlaku untuk async.

use std::collections::BTreeMap;
use std::fmt;

use poolrouter_core::retry::ErrorClass;
use serde_json::{Map, Value};

use crate::dokumen::KonfigurasiRespons;
use crate::{GagalPanggilProvider, baca_respons, dari_status_http, kelas_error};

/// Respons HTTP apa pun kodenya. Klasifikasi ada di [`nilai_hasil`].
#[derive(Clone, PartialEq, Eq)]
pub struct ResponsMentah {
    pub status: u16,
    pub body: Vec<u8>,
}

impl fmt::Debug for ResponsMentah {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResponsMentah")
            .field("status", &self.status)
            .field("panjang_body", &self.body.len())
            .finish()
    }
}

impl ResponsMentah {
    /// SHA-256 body mentah sebagai 64 hex huruf kecil, bentuk
    /// `transaction_log.response_hash`. Dihitung atas byte yang diterima,
    /// sebelum parse dan sebelum penyamaran secret, sehingga tetap berguna
    /// sebagai bukti pada `log_mode = metadata_only` (body tidak disimpan).
    pub fn sha256_hex(&self) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(&self.body))
    }
}

/// Hasil satu percobaan, siap ditulis ke `transaction_log`.
#[derive(Debug, Clone, PartialEq)]
pub struct Akhir {
    /// `berhasil`, `gagal_provider`, atau `gagal_jaringan`.
    pub status_akses: &'static str,
    pub error_class: Option<ErrorClass>,
    pub http_status: Option<u16>,
    /// Provider menjawab (ada respons HTTP); mengisi `responded_at`.
    pub ada_respons: bool,
    /// Output `done` (peta elemen → `HasilElemen`), hanya bila berhasil.
    pub output: Option<Value>,
}

fn gagal(kelas: ErrorClass, http_status: Option<u16>) -> Akhir {
    let status_akses = match kelas {
        ErrorClass::Timeout | ErrorClass::Koneksi | ErrorClass::Tls => "gagal_jaringan",
        _ => "gagal_provider",
    };
    Akhir {
        status_akses,
        error_class: Some(kelas),
        http_status,
        ada_respons: http_status.is_some(),
        output: None,
    }
}

/// Petakan hasil panggilan. `input` adalah field input client, dipakai untuk
/// membuang elemen yang tidak dikirim client (P-1) setelah seluruh elemen
/// lolos parse fail-closed.
pub fn nilai_hasil(
    hasil: &Result<ResponsMentah, GagalPanggilProvider>,
    konfigurasi: &KonfigurasiRespons,
    input: &BTreeMap<String, String>,
) -> Akhir {
    let respons = match hasil {
        Err(g) => return gagal(kelas_error(g), None),
        Ok(r) => r,
    };
    if !(200..300).contains(&respons.status) {
        let kelas = kelas_error(&dari_status_http(respons.status));
        return gagal(kelas, Some(respons.status));
    }
    let elemen = match baca_respons(&respons.body, &konfigurasi.status_skor) {
        Ok(e) => e,
        Err(_) => return gagal(ErrorClass::ResponsTidakValid, Some(respons.status)),
    };
    let mut output = Map::new();
    for e in elemen {
        let dibuang = konfigurasi
            .output
            .iter()
            .find(|o| o.nama == e.elemen)
            .is_some_and(|o| {
                o.abaikan_jika_input_kosong
                    && input
                        .get(&o.parameter_input_sumber)
                        .is_none_or(|v| v.trim().is_empty())
            });
        if dibuang {
            continue;
        }
        let nilai = serde_json::to_value(e.hasil).unwrap_or(Value::Null);
        output.insert(e.elemen, nilai);
    }
    Akhir {
        status_akses: "berhasil",
        error_class: None,
        http_status: Some(respons.status),
        ada_respons: true,
        output: Some(Value::Object(output)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_atas_byte_mentah_huruf_kecil_64_karakter() {
        let r = ResponsMentah {
            status: 503,
            body: b"abc".to_vec(),
        };
        assert_eq!(
            r.sha256_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let kosong = ResponsMentah {
            status: 204,
            body: Vec::new(),
        };
        assert_eq!(
            kosong.sha256_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
