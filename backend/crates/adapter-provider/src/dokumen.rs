//! Dokumen konfigurasi provider dan render template ganda (ADR-0001 bagian 7,
//! ADR-0002 D1–D3).
//!
//! Dipindah dari `poolrouter-worker` (ARC-71) ke adapter (ARC-66) supaya
//! worker dan jalur sync webserver memakai satu jalur kode yang sama; isinya
//! tidak berubah.
//!
//! Satu kali rakit menghasilkan dua keluaran dari template yang sama:
//!
//! - **kabel**: nilai secret asli, hanya hidup di memori untuk panggilan;
//! - **log**: setiap `{{secret.X}}` tetap tertulis sebagai penanda
//!   `{{secret.X}}`.
//!
//! Keluaran log tidak dibuat dengan mencari-dan-mengganti nilai secret di
//! keluaran kabel. `render_teks` tidak pernah me-render ulang nilai yang sudah
//! disisipkan, jadi penanda di keluaran log adalah teks biasa.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::client::TlsTrust;
use crate::template::{KUNCI_SISTEM_F0, Nilai, RenderError, render_teks};
use poolrouter_core::respons::{KonfigurasiStatusSkor, LiteralStatus, StatusKesesuaian};
use serde_json::{Value, json};
use uuid::Uuid;

/// Dokumen konfigurasi provider yang tidak bisa dipakai worker. Pesannya hanya
/// menyebut kunci yang bermasalah, tidak pernah nilai.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GalatKonfigurasi(pub String);

impl fmt::Display for GalatKonfigurasi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Satu parameter output (ADR-0002 D3, rev 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterOutput {
    pub nama: String,
    /// Parameter input sumber elemen ini; bawaan sama dengan `nama`.
    pub parameter_input_sumber: String,
    /// P-1: elemen dibuang dari output bila inputnya tidak dikirim client.
    pub abaikan_jika_input_kosong: bool,
}

/// Skema respons provider: parser `status_skor` plus parameter output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KonfigurasiRespons {
    pub status_skor: KonfigurasiStatusSkor,
    pub output: Vec<ParameterOutput>,
}

/// Bagian dokumen `provider_config_versi.dokumen` yang dibaca worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DokumenProvider {
    pub metode: String,
    pub base_url: String,
    pub body_template: String,
    pub tls_trust: String,
    /// Bahan `ca_kustom` (ARC-239): satu atau lebih sertifikat CA dalam PEM.
    pub tls_ca_pem: Option<String>,
    /// Bahan `pin_spki` (ARC-239): SHA-256 SPKI dalam base64 standar.
    pub tls_pin_spki: Option<Vec<String>>,
    pub respons: KonfigurasiRespons,
}

/// Batas bahan CA kustom (ARC-239): ukuran PEM dan jumlah sertifikat.
pub const PEM_MAKS_BYTE: usize = 64 * 1024;
pub const PEM_MAKS_SERTIFIKAT: usize = 10;
/// Batas jumlah pin SPKI per provider (ARC-239).
pub const PIN_MAKS: usize = 10;

impl DokumenProvider {
    /// Trust TLS yang dirakit dari mode dan bahannya. Mode wajib cocok dengan
    /// bahan: `system` tanpa bahan, `ca_kustom` dengan PEM tanpa pin, `pin_spki`
    /// dengan pin tanpa PEM. Selain itu galat (fail-closed). Pesan hanya
    /// menyebut kunci, tidak pernah nilai.
    pub fn trust(&self) -> Result<TlsTrust, GalatKonfigurasi> {
        let mode = TlsTrust::dari_str(&self.tls_trust)
            .ok_or_else(|| galat(format!("tls_trust {} tidak dikenal", self.tls_trust)))?;
        match (mode, &self.tls_ca_pem, &self.tls_pin_spki) {
            (TlsTrust::System, None, None) => Ok(TlsTrust::System),
            (TlsTrust::CaKustom { .. }, Some(pem), None) => Ok(TlsTrust::CaKustom {
                ca_pem: pem.clone().into_bytes(),
            }),
            (TlsTrust::PinSpki { .. }, None, Some(pin)) if !pin.is_empty() => {
                let pins = pin
                    .iter()
                    .map(|p| pin_dari_base64(p))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| galat("dokumen provider: `tls_pin_spki` berisi pin tidak sah"))?;
                Ok(TlsTrust::PinSpki { pins })
            }
            _ => Err(galat(format!(
                "tls_trust {} tidak cocok dengan bahan `tls_ca_pem`/`tls_pin_spki`",
                self.tls_trust
            ))),
        }
    }
}

/// Satu pin SPKI: base64 standar (dengan padding) yang terdekode tepat 32 byte.
pub fn pin_dari_base64(pin: &str) -> Result<[u8; 32], GalatKonfigurasi> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(pin)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
        .ok_or_else(|| galat("pin SPKI harus base64 standar dari 32 byte"))
}

/// Periksa bahan CA kustom: hanya blok `CERTIFICATE` yang bisa diparse sebagai
/// X.509, 1 sampai [`PEM_MAKS_SERTIFIKAT`] blok, paling besar [`PEM_MAKS_BYTE`].
/// Blok jenis lain (termasuk `PRIVATE KEY`) ditolak eksplisit karena bahan ini
/// disimpan sebagai konfigurasi biasa dan ikut dikembalikan GET. Hasilnya kode
/// galat: `terlalu_besar` atau `pem_tidak_sah`.
pub fn validasi_ca_pem(pem: &str) -> Result<(), &'static str> {
    use rustls::pki_types::CertificateDer;
    use rustls::pki_types::pem::PemObject;

    if pem.len() > PEM_MAKS_BYTE {
        return Err("terlalu_besar");
    }
    // `pem_slice_iter` diam-diam melewati blok non-sertifikat, jadi penanda
    // dihitung sendiri dan harus sama dengan jumlah sertifikat yang terparse.
    let penanda: Vec<&str> = pem
        .lines()
        .filter(|l| l.contains("-----BEGIN "))
        .map(str::trim)
        .collect();
    if penanda.is_empty()
        || penanda.len() > PEM_MAKS_SERTIFIKAT
        || penanda.iter().any(|l| *l != "-----BEGIN CERTIFICATE-----")
    {
        return Err("pem_tidak_sah");
    }
    let certs = CertificateDer::pem_slice_iter(pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "pem_tidak_sah")?;
    if certs.len() != penanda.len() {
        return Err("pem_tidak_sah");
    }
    for c in &certs {
        match x509_parser::parse_x509_certificate(c.as_ref()) {
            Ok(([], _)) => {}
            _ => return Err("pem_tidak_sah"),
        }
    }
    Ok(())
}

fn galat(pesan: impl Into<String>) -> GalatKonfigurasi {
    GalatKonfigurasi(pesan.into())
}

/// Baca dokumen provider (bentuk yang ditulis admin API `createProvider`).
pub fn baca_dokumen_provider(dok: &Value) -> Result<DokumenProvider, GalatKonfigurasi> {
    let teks = |kunci: &str| -> Result<String, GalatKonfigurasi> {
        dok.get(kunci)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                galat(format!(
                    "dokumen provider: `{kunci}` tidak ada atau bukan teks"
                ))
            })
    };
    let skema = dok
        .get("respons_skema")
        .ok_or_else(|| galat("dokumen provider: `respons_skema` tidak ada"))?;
    // Bahan TLS (ARC-239): kunci tidak ada atau `null` = tidak diisi.
    let tls_ca_pem = match dok.get("tls_ca_pem") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err(galat("dokumen provider: `tls_ca_pem` bukan teks")),
    };
    let tls_pin_spki = match dok.get("tls_pin_spki") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => Some(
            a.iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| galat("dokumen provider: `tls_pin_spki` harus array teks"))?,
        ),
        Some(_) => return Err(galat("dokumen provider: `tls_pin_spki` bukan array")),
    };
    Ok(DokumenProvider {
        metode: teks("metode")?,
        base_url: teks("base_url")?,
        body_template: teks("body_template")?,
        tls_trust: teks("tls_trust")?,
        tls_ca_pem,
        tls_pin_spki,
        respons: baca_respons_skema(skema)?,
    })
}

/// Baca `respons_skema`. Bentuk yang diterima (satu-satunya di F0):
///
/// ```json
/// {"parser": "status_skor",
///  "literal": [{"teks": "Sesuai", "status": "sesuai"},
///              {"teks": "Tidak Sesuai", "status": "tidak_sesuai"}],
///  "parameter_output": [{"nama": "NAMA_LGKP",
///                        "parameter_input_sumber": "NAMA_LGKP",
///                        "abaikan_jika_input_kosong": true}]}
/// ```
///
/// Kontrak repo belum menetapkan bentuk `respons_skema` (admin API
/// menyimpannya sebagai objek bebas); bentuk ini dicatat sebagai deviasi
/// ARC-71. Bentuk lain ditolak (fail-closed).
pub fn baca_respons_skema(skema: &Value) -> Result<KonfigurasiRespons, GalatKonfigurasi> {
    if skema.get("parser").and_then(Value::as_str) != Some("status_skor") {
        return Err(galat("respons_skema: `parser` harus `status_skor`"));
    }
    let literal = skema
        .get("literal")
        .and_then(Value::as_array)
        .ok_or_else(|| galat("respons_skema: `literal` harus array"))?
        .iter()
        .map(|l| {
            let teks = l
                .get("teks")
                .and_then(Value::as_str)
                .ok_or_else(|| galat("respons_skema: `literal[].teks` harus teks"))?;
            let status = match l.get("status").and_then(Value::as_str) {
                Some("sesuai") => StatusKesesuaian::Sesuai,
                Some("tidak_sesuai") => StatusKesesuaian::TidakSesuai,
                _ => return Err(galat("respons_skema: `literal[].status` tidak dikenal")),
            };
            Ok(LiteralStatus::baru(teks, status))
        })
        .collect::<Result<Vec<_>, GalatKonfigurasi>>()?;
    let output = skema
        .get("parameter_output")
        .and_then(Value::as_array)
        .ok_or_else(|| galat("respons_skema: `parameter_output` harus array"))?
        .iter()
        .map(|p| {
            let nama = p
                .get("nama")
                .and_then(Value::as_str)
                .ok_or_else(|| galat("respons_skema: `parameter_output[].nama` harus teks"))?;
            let sumber = p
                .get("parameter_input_sumber")
                .and_then(Value::as_str)
                .unwrap_or(nama);
            let abaikan = p
                .get("abaikan_jika_input_kosong")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            Ok(ParameterOutput {
                nama: nama.to_owned(),
                parameter_input_sumber: sumber.to_owned(),
                abaikan_jika_input_kosong: abaikan,
            })
        })
        .collect::<Result<Vec<_>, GalatKonfigurasi>>()?;
    Ok(KonfigurasiRespons {
        status_skor: KonfigurasiStatusSkor {
            literal,
            parameter_output: output.iter().map(|o| o.nama.clone()).collect(),
        },
        output,
    })
}

fn sebagai_teks(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        lain => lain.to_string(),
    }
}

/// Field input client dari `request.input_payload` (objek nama → nilai).
pub fn nilai_input(payload: Option<&Value>) -> BTreeMap<String, String> {
    payload
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), sebagai_teks(v)))
                .collect()
        })
        .unwrap_or_default()
}

/// `parameter_literal` dokumen endpoint (ADR-0002 D3, TRESHOLD).
pub fn parameter_literal(dokumen_endpoint: Option<&Value>) -> BTreeMap<String, String> {
    dokumen_endpoint
        .and_then(|d| d.get("parameter_literal"))
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), sebagai_teks(v)))
                .collect()
        })
        .unwrap_or_default()
}

/// Nama setiap `{{secret.X}}` yang dirujuk teks template, tanpa duplikat.
pub fn nama_secret(teks: &[&str]) -> Vec<String> {
    let mut nama = BTreeSet::new();
    for t in teks {
        let mut sisa = *t;
        while let Some(buka) = sisa.find("{{") {
            let setelah = &sisa[buka + 2..];
            let Some(tutup) = setelah.find("}}") else {
                break;
            };
            if let Some(n) = setelah[..tutup].trim().strip_prefix("secret.") {
                nama.insert(n.to_owned());
            }
            sisa = &setelah[tutup + 2..];
        }
    }
    nama.into_iter().collect()
}

fn penanda_secret(nama: &str) -> String {
    format!("{{{{secret.{nama}}}}}")
}

/// Hasil render ganda. `Debug` hanya menampilkan keluaran log, karena keluaran
/// kabel memuat secret dan NIK.
pub struct Rakitan {
    pub kabel_url: String,
    pub kabel_body: Option<String>,
    pub log_url: String,
    pub log_body: Option<String>,
}

impl fmt::Debug for Rakitan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rakitan")
            .field("kabel", &"<redacted>")
            .field("log_url", &self.log_url)
            .finish()
    }
}

/// Render URL dan body sekali untuk kabel dan sekali untuk log, dengan
/// `kode_transaksi` yang sama (ADR-0002 D2).
pub fn rakit(
    dok: &DokumenProvider,
    input: &BTreeMap<String, String>,
    literal: &BTreeMap<String, String>,
    secret: &BTreeMap<String, String>,
    kode_transaksi: Uuid,
) -> Result<Rakitan, RenderError> {
    let mut sistem = BTreeMap::new();
    sistem.insert(KUNCI_SISTEM_F0.to_owned(), kode_transaksi.to_string());
    let kabel = Nilai {
        input: input.clone(),
        secret: secret.clone(),
        sistem: sistem.clone(),
        endpoint: literal.clone(),
    };
    let log = Nilai {
        input: input.clone(),
        secret: secret
            .keys()
            .map(|n| (n.clone(), penanda_secret(n)))
            .collect(),
        sistem,
        endpoint: literal.clone(),
    };
    let body = |nilai: &Nilai| -> Result<Option<String>, RenderError> {
        if dok.body_template.trim().is_empty() {
            Ok(None)
        } else {
            render_teks(&dok.body_template, nilai).map(Some)
        }
    };
    Ok(Rakitan {
        kabel_url: render_teks(&dok.base_url, &kabel)?,
        kabel_body: body(&kabel)?,
        log_url: render_teks(&dok.base_url, &log)?,
        log_body: body(&log)?,
    })
}

fn json_atau_teks(teks: String) -> Value {
    match serde_json::from_str::<Value>(&teks) {
        Ok(v) => v,
        Err(_) => Value::String(teks),
    }
}

/// `transaction_log.request_payload` untuk `full_payload`: hanya keluaran log.
pub fn payload_permintaan(metode: &str, rakitan: &Rakitan) -> Value {
    json!({
        "metode": metode,
        "url": rakitan.log_url,
        "body": rakitan.log_body.clone().map(json_atau_teks),
    })
}

/// `transaction_log.response_payload` untuk `full_payload`: nilai secret yang
/// digemakan provider diganti penanda sebelum ditulis (ADR-0001 bagian 7).
pub fn payload_respons(body: &[u8], secret: &BTreeMap<String, String>) -> Value {
    let mut teks = String::from_utf8_lossy(body).into_owned();
    for (nama, nilai) in secret {
        if !nilai.is_empty() {
            teks = teks.replace(nilai.as_str(), &penanda_secret(nama));
        }
    }
    json_atau_teks(teks)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NIK: &str = "0012345678901234";

    fn skema() -> Value {
        json!({
            "parser": "status_skor",
            "literal": [
                {"teks": "Sesuai", "status": "sesuai"},
                {"teks": "Tidak Sesuai", "status": "tidak_sesuai"}
            ],
            "parameter_output": [
                {"nama": "NAMA_LGKP"},
                {"nama": "TMPT_LHR", "abaikan_jika_input_kosong": false}
            ]
        })
    }

    fn dokumen() -> DokumenProvider {
        baca_dokumen_provider(&json!({
            "metode": "POST",
            "base_url": "https://dukcapil.contoh.test/api",
            "body_template": "{\"NIK\":\"{{input.NIK}}\",\"USER_ID\":\"{{secret.USER_ID}}\",\
                              \"transactionId\":\"{{system.kode_transaksi}}\",\
                              \"TRESHOLD\":\"{{endpoint.TRESHOLD}}\"}",
            "tls_trust": "system",
            "respons_skema": skema(),
            "parameters": [],
            "log_mode": "full_payload"
        }))
        .expect("dokumen sah")
    }

    #[test]
    fn skema_respons_dibaca_dengan_bawaan_p1() {
        let k = baca_respons_skema(&skema()).unwrap();
        assert_eq!(k.status_skor.parameter_output, ["NAMA_LGKP", "TMPT_LHR"]);
        assert_eq!(k.output[0].parameter_input_sumber, "NAMA_LGKP");
        assert!(k.output[0].abaikan_jika_input_kosong);
        assert!(!k.output[1].abaikan_jika_input_kosong);
    }

    #[test]
    fn skema_respons_lain_ditolak() {
        assert!(baca_respons_skema(&json!({"status_skema": "status_skor"})).is_err());
        assert!(baca_respons_skema(&json!({"parser": "json", "literal": []})).is_err());
        let mut s = skema();
        s["literal"][0]["status"] = json!("cocok");
        assert!(baca_respons_skema(&s).is_err());
    }

    #[test]
    fn nama_secret_dari_semua_teks() {
        let n = nama_secret(&[
            "{{secret.B}} {{ secret.A }} {{input.NIK}}",
            "https://x/{{secret.B}}",
        ]);
        assert_eq!(n, ["A", "B"]);
    }

    #[test]
    fn render_ganda_kabel_berisi_secret_log_berisi_penanda() {
        let dok = dokumen();
        let input = nilai_input(Some(&json!({"NIK": NIK, "UMUR": 5, "KOSONG": null})));
        assert_eq!(input["UMUR"], "5");
        assert_eq!(input["KOSONG"], "");
        let literal = parameter_literal(Some(&json!({"parameter_literal": {"TRESHOLD": "80"}})));
        let mut secret = BTreeMap::new();
        secret.insert("USER_ID".to_owned(), "rahasia-sintetis".to_owned());
        let kode = Uuid::now_v7();

        let r = rakit(&dok, &input, &literal, &secret, kode).unwrap();
        let kabel = r.kabel_body.as_deref().unwrap();
        let log = r.log_body.as_deref().unwrap();
        assert!(kabel.contains("rahasia-sintetis"));
        assert!(kabel.contains(&kode.to_string()));
        assert!(kabel.contains("\"TRESHOLD\":\"80\""));
        assert!(!log.contains("rahasia-sintetis"));
        assert!(log.contains("{{secret.USER_ID}}"));
        assert!(log.contains(&kode.to_string()));
        assert!(!format!("{r:?}").contains("rahasia-sintetis"));

        let p = payload_permintaan("POST", &r);
        assert_eq!(p["body"]["USER_ID"], "{{secret.USER_ID}}");
        assert_eq!(p["body"]["NIK"], NIK);
    }

    #[test]
    fn render_gagal_tertutup_bila_secret_tidak_ada() {
        let dok = dokumen();
        let input = nilai_input(Some(&json!({"NIK": NIK})));
        let literal = parameter_literal(Some(&json!({"parameter_literal": {"TRESHOLD": "80"}})));
        assert!(rakit(&dok, &input, &literal, &BTreeMap::new(), Uuid::now_v7()).is_err());
    }

    #[test]
    fn gema_secret_di_respons_disamarkan() {
        let mut secret = BTreeMap::new();
        secret.insert("USER_ID".to_owned(), "rahasia-sintetis".to_owned());
        let v = payload_respons(br#"{"UID":"rahasia-sintetis","ok":1}"#, &secret);
        assert_eq!(v["UID"], "{{secret.USER_ID}}");
        assert_eq!(v["ok"], 1);
        assert_eq!(payload_respons(b"bukan json", &secret), json!("bukan json"));
    }

    // ------------------------------------------------------------ ARC-239

    fn pem_ca() -> String {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut p = rcgen::CertificateParams::new(Vec::new()).unwrap();
        p.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        p.self_signed(&key).unwrap().pem()
    }

    fn pin_b64() -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode([7u8; 32])
    }

    fn dok_tls(tls: Value) -> Result<DokumenProvider, GalatKonfigurasi> {
        let mut d = json!({
            "metode": "POST",
            "base_url": "https://dukcapil.contoh.test/api",
            "body_template": "{}",
            "respons_skema": skema(),
        });
        for (k, v) in tls.as_object().unwrap() {
            d[k] = v.clone();
        }
        baca_dokumen_provider(&d)
    }

    #[test]
    fn pin_harus_base64_standar_32_byte() {
        assert_eq!(pin_dari_base64(&pin_b64()).unwrap(), [7u8; 32]);
        use base64::Engine as _;
        let e = base64::engine::general_purpose::STANDARD;
        let url_safe = base64::engine::general_purpose::URL_SAFE.encode([0xfbu8; 32]);
        for salah in [
            "",
            "bukan base64!",
            // 31 dan 33 byte.
            &e.encode([1u8; 31]),
            &e.encode([1u8; 33]),
            // tanpa padding dan alfabet url-safe bukan base64 standar.
            pin_b64().trim_end_matches('='),
            &url_safe,
        ] {
            assert!(pin_dari_base64(salah).is_err(), "{salah:?}");
        }
    }

    #[test]
    fn ca_pem_hanya_blok_certificate_yang_sah() {
        let satu = pem_ca();
        assert_eq!(validasi_ca_pem(&satu), Ok(()));
        // Bundle dua sertifikat (CA lama + baru saat rotasi).
        assert_eq!(validasi_ca_pem(&format!("{satu}{}", pem_ca())), Ok(()));

        let kunci = "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n";
        let sampah = "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n";
        for (nama, pem) in [
            ("kosong", String::new()),
            ("tanpa blok", "bukan pem".to_owned()),
            ("kunci privat saja", kunci.to_owned()),
            // `pem_slice_iter` melewati blok lain, jadi ini harus tetap ditolak.
            ("sertifikat plus kunci privat", format!("{satu}{kunci}")),
            (
                "TRUSTED CERTIFICATE",
                satu.replace("BEGIN CERTIFICATE", "BEGIN TRUSTED CERTIFICATE")
                    .replace("END CERTIFICATE", "END TRUSTED CERTIFICATE"),
            ),
            (
                "X509 CRL",
                "-----BEGIN X509 CRL-----\nAAAA\n-----END X509 CRL-----\n".to_owned(),
            ),
            ("DER bukan X.509", sampah.to_owned()),
            (
                "tanpa penutup",
                satu.replace("-----END CERTIFICATE-----", ""),
            ),
        ] {
            assert_eq!(validasi_ca_pem(&pem), Err("pem_tidak_sah"), "{nama}");
        }
        let sebelas = satu.repeat(PEM_MAKS_SERTIFIKAT + 1);
        assert_eq!(validasi_ca_pem(&sebelas), Err("pem_tidak_sah"));
        assert_eq!(
            validasi_ca_pem(&satu.repeat(PEM_MAKS_SERTIFIKAT)),
            Ok(()),
            "tepat di batas jumlah"
        );
        let besar = format!("{satu}{}", " ".repeat(PEM_MAKS_BYTE));
        assert_eq!(validasi_ca_pem(&besar), Err("terlalu_besar"));
    }

    #[test]
    fn dokumen_membaca_bahan_tls_dan_trust_cocok_dengan_mode() {
        let pem = pem_ca();
        // Dokumen lama tanpa kedua kunci, dan `null`, sama dengan tidak diisi.
        let lama = dok_tls(json!({"tls_trust": "system"})).unwrap();
        assert_eq!((&lama.tls_ca_pem, &lama.tls_pin_spki), (&None, &None));
        assert_eq!(lama.trust(), Ok(TlsTrust::System));
        let nol = dok_tls(json!({"tls_trust": "system", "tls_ca_pem": null, "tls_pin_spki": null}))
            .unwrap();
        assert_eq!(nol.trust(), Ok(TlsTrust::System));

        let ca = dok_tls(json!({"tls_trust": "ca_kustom", "tls_ca_pem": pem})).unwrap();
        assert_eq!(
            ca.trust(),
            Ok(TlsTrust::CaKustom {
                ca_pem: pem.clone().into_bytes()
            })
        );
        let pin = dok_tls(json!({"tls_trust": "pin_spki", "tls_pin_spki": [pin_b64()]})).unwrap();
        assert_eq!(
            pin.trust(),
            Ok(TlsTrust::PinSpki {
                pins: vec![[7u8; 32]]
            })
        );

        // Mode dan bahan harus cocok; selain itu fail-closed.
        for tls in [
            json!({"tls_trust": "ca_kustom"}),
            json!({"tls_trust": "pin_spki"}),
            json!({"tls_trust": "pin_spki", "tls_pin_spki": []}),
            json!({"tls_trust": "pin_spki", "tls_pin_spki": ["xx"]}),
            json!({"tls_trust": "system", "tls_ca_pem": pem}),
            json!({"tls_trust": "system", "tls_pin_spki": [pin_b64()]}),
            json!({"tls_trust": "ca_kustom", "tls_ca_pem": pem, "tls_pin_spki": [pin_b64()]}),
            json!({"tls_trust": "pin_spki", "tls_ca_pem": pem, "tls_pin_spki": [pin_b64()]}),
            json!({"tls_trust": "matikan"}),
        ] {
            let d = dok_tls(tls.clone()).unwrap();
            assert!(d.trust().is_err(), "{tls}");
        }
        // Bentuk kunci yang salah ditolak saat baca.
        assert!(dok_tls(json!({"tls_trust": "ca_kustom", "tls_ca_pem": 1})).is_err());
        assert!(dok_tls(json!({"tls_trust": "pin_spki", "tls_pin_spki": "x"})).is_err());
        assert!(dok_tls(json!({"tls_trust": "pin_spki", "tls_pin_spki": [1]})).is_err());
    }
}
