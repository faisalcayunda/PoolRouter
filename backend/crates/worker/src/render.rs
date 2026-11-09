//! Dokumen konfigurasi provider dan render template ganda (ADR-0001 bagian 7,
//! ADR-0002 D1–D3).
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

use poolrouter_adapter_provider::template::{KUNCI_SISTEM_F0, Nilai, RenderError, render_teks};
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
    pub respons: KonfigurasiRespons,
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
    Ok(DokumenProvider {
        metode: teks("metode")?,
        base_url: teks("base_url")?,
        body_template: teks("body_template")?,
        tls_trust: teks("tls_trust")?,
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
}
