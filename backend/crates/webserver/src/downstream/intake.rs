//! Intake validation shared by both submit paths (ADR-0001 rev 15 section 4.1,
//! ADR-0002 rev 5 D6; SHAA-198 part 2).
//!
//! This module only decides what the body *is*. It never touches the database,
//! Redis or the clock, so every rule here is a pure function and unit testable.
//! The order it enforces is the order the flow diagram requires:
//!
//! 1. the body must be JSON with no duplicated key (400 `validasi_gagal`);
//! 2. `input` keys must be exactly the endpoint contract's input fields, and
//!    `metadata` keys exactly its metadata fields (422, before any quota check);
//! 3. `Idempotency-Key` is refused on the sync path (400) and normalised on the
//!    async path.
//!
//! Rejecting before the quota decision is what keeps a malformed body from
//! spending the pool's daily allowance (ADR-0002 D6, contract v0.7.0 section 11
//! point 29).

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};

use crate::api::problem::{JumlahNamaTidakCocok, ValidasiErrorDown};

/// A field name is echoed back only when it looks like a contract name.
/// Anything else is withheld and counted (docs/masalah.md `validasi_gagal`).
const NAMA_SAH_MIN: usize = 1;
const NAMA_SAH_MAKS: usize = 64;

/// `Idempotency-Key` is at most 128 ASCII characters (ADR-0001 section 4.1).
pub const IDEMPOTENCY_KEY_MAKS: usize = 128;

/// Why a body was refused. Each variant maps to one problem body.
///
/// `PartialEq` without `Eq`: the payload carries `ValidasiErrorDown`, whose
/// `batas` is an `f64`.
#[derive(Debug, Clone, PartialEq)]
pub enum TolakIntake {
    /// 400: the bytes are not JSON, or a key appears twice in one object.
    BodyTidakSah(&'static str),
    /// 422 `validasi_gagal`: `input` does not match the endpoint contract.
    InputTidakValid {
        errors: Vec<ValidasiErrorDown>,
        jumlah_nama_tidak_cocok: Option<JumlahNamaTidakCocok>,
    },
    /// 422 `metadata_tidak_valid`: `metadata` does not match the contract.
    MetadataTidakValid {
        errors: Vec<ValidasiErrorDown>,
        jumlah_nama_tidak_cocok: Option<JumlahNamaTidakCocok>,
    },
    /// 400 `idempotency_key_tidak_didukung`: header sent to the sync path.
    IdempotencyKeyTidakDidukung,
    /// 422 `idempotency_key_konflik`: same key, different fingerprint.
    IdempotencyKeyKonflik,
    /// 400 `validasi_gagal`: the header itself breaks the format rule.
    IdempotencyKeyTidakSah,
}

/// One input field of the endpoint contract, as intake needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldInputKontrak {
    pub nama: String,
    pub tipe: String,
    pub wajib: bool,
}

/// One metadata field of the endpoint contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldMetadataKontrak {
    pub nama_field: String,
    pub wajib: bool,
    pub nilai_diizinkan: Option<Vec<String>>,
}

/// The endpoint contract, reduced to what intake checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KontrakIntake {
    pub field_input: Vec<FieldInputKontrak>,
    pub field_metadata: Vec<FieldMetadataKontrak>,
}

/// A field name is safe to echo only if it matches `^[A-Za-z0-9_]{1,64}$`.
fn nama_sah(nama: &str) -> bool {
    (NAMA_SAH_MIN..=NAMA_SAH_MAKS).contains(&nama.len())
        && nama.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Collects at most one error per field and counts the names that must be
/// withheld instead of echoed.
#[derive(Debug, Default)]
struct Pengumpul {
    errors: Vec<ValidasiErrorDown>,
    tidak_cocok: u32,
}

impl Pengumpul {
    fn tolak(&mut self, nama: &str, kode: &str, batas: Option<f64>) {
        if nama_sah(nama) {
            self.errors.push(ValidasiErrorDown {
                field: nama.to_owned(),
                kode: kode.to_owned(),
                batas,
            });
        } else {
            self.tidak_cocok = self.tidak_cocok.saturating_add(1);
        }
    }

    fn kosong(&self) -> bool {
        self.errors.is_empty() && self.tidak_cocok == 0
    }

    /// `JumlahNamaTidakCocok` is never zero on the wire, so a zero count is
    /// reported as absent.
    fn jumlah(&self) -> Option<JumlahNamaTidakCocok> {
        JumlahNamaTidakCocok::try_from(self.tidak_cocok).ok()
    }

    /// Consumes the collector into the two halves a rejection carries. Both
    /// leave together because the count has to be read before `errors` is moved
    /// out of `self`.
    fn selesai(self) -> (Vec<ValidasiErrorDown>, Option<JumlahNamaTidakCocok>) {
        let jumlah = self.jumlah();
        (self.errors, jumlah)
    }
}

/// Recursive JSON reader that refuses a duplicated key in any object. Plain
/// `serde_json::Value` silently keeps the last one, which would let a client
/// smuggle `{"NIK": "1", "NIK": "2"}` past validation.
struct TanpaGanda(Value);

impl<'de> Deserialize<'de> for TanpaGanda {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(PembacaGanda).map(Self)
    }
}

struct PembacaGanda;

impl<'de> Visitor<'de> for PembacaGanda {
    type Value = Value;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("sebuah nilai JSON tanpa kunci ganda")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        serde_json::Number::from_f64(v).map_or_else(
            || Err(E::custom("angka bukan bilangan hingga")),
            |n| Ok(Value::Number(n)),
        )
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        TanpaGanda::deserialize(d).map(|t| t.0)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut out = Vec::new();
        while let Some(TanpaGanda(v)) = seq.next_element()? {
            out.push(v);
        }
        Ok(Value::Array(out))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut out = Map::new();
        while let Some(kunci) = map.next_key::<String>()? {
            let TanpaGanda(nilai) = map.next_value()?;
            if out.insert(kunci.clone(), nilai).is_some() {
                return Err(de::Error::custom(format!("kunci ganda: {kunci}")));
            }
        }
        Ok(Value::Object(out))
    }
}

/// Parses a submit body, refusing duplicated keys anywhere in the tree.
pub fn baca_body(bytes: &[u8]) -> Result<Value, TolakIntake> {
    serde_json::from_slice::<TanpaGanda>(bytes)
        .map(|t| t.0)
        .map_err(|_| TolakIntake::BodyTidakSah("Body bukan JSON yang sah atau memuat kunci ganda."))
}

/// Checks `input` against the endpoint contract. Unknown keys are refused
/// rather than ignored (D6): `TRESHOLD`, `USER_ID`, `PASSWORD`, `IP_USER` and
/// `transactionId` are not contract fields, so they are `field_tak_dikenal`,
/// never a value-below-minimum error.
pub fn validasi_input(input: &Value, kontrak: &[FieldInputKontrak]) -> Result<(), TolakIntake> {
    let objek = input.as_object().ok_or(TolakIntake::InputTidakValid {
        errors: vec![ValidasiErrorDown {
            field: "input".to_owned(),
            kode: "tipe_salah".to_owned(),
            batas: None,
        }],
        jumlah_nama_tidak_cocok: None,
    })?;

    let mut p = Pengumpul::default();

    for nama in objek.keys() {
        if !kontrak.iter().any(|f| &f.nama == nama) {
            p.tolak(nama, "field_tak_dikenal", None);
        }
    }

    for field in kontrak {
        match objek.get(&field.nama) {
            None => {
                if field.wajib {
                    p.tolak(&field.nama, "wajib", None);
                }
            }
            Some(Value::Null) => {
                if field.wajib {
                    p.tolak(&field.nama, "wajib", None);
                }
            }
            Some(v) if field.wajib && kosong(v) => p.tolak(&field.nama, "wajib", None),
            Some(v) => {
                if !tipe_cocok(v, &field.tipe) {
                    p.tolak(&field.nama, "tipe_salah", None);
                }
            }
        }
    }

    if p.kosong() {
        Ok(())
    } else {
        let (errors, jumlah_nama_tidak_cocok) = p.selesai();
        Err(TolakIntake::InputTidakValid {
            errors,
            jumlah_nama_tidak_cocok,
        })
    }
}

/// Checks `metadata` against the contract's metadata fields, including the
/// closed value list of a bounded field.
pub fn validasi_metadata(
    metadata: &Value,
    kontrak: &[FieldMetadataKontrak],
) -> Result<(), TolakIntake> {
    let objek = metadata
        .as_object()
        .ok_or(TolakIntake::MetadataTidakValid {
            errors: vec![ValidasiErrorDown {
                field: "metadata".to_owned(),
                kode: "tipe_salah".to_owned(),
                batas: None,
            }],
            jumlah_nama_tidak_cocok: None,
        })?;

    let mut p = Pengumpul::default();

    for nama in objek.keys() {
        if !kontrak.iter().any(|f| &f.nama_field == nama) {
            p.tolak(nama, "field_tak_dikenal", None);
        }
    }

    for field in kontrak {
        match objek.get(&field.nama_field) {
            None => {
                if field.wajib {
                    p.tolak(&field.nama_field, "wajib", None);
                }
            }
            Some(Value::Null) => {
                if field.wajib {
                    p.tolak(&field.nama_field, "wajib", None);
                }
            }
            Some(v) => {
                if let Some(diizinkan) = &field.nilai_diizinkan {
                    let cocok = v.as_str().is_some_and(|s| diizinkan.iter().any(|d| d == s));
                    if !cocok {
                        p.tolak(&field.nama_field, "nilai_tidak_diizinkan", None);
                    }
                }
            }
        }
    }

    if p.kosong() {
        Ok(())
    } else {
        let (errors, jumlah_nama_tidak_cocok) = p.selesai();
        Err(TolakIntake::MetadataTidakValid {
            errors,
            jumlah_nama_tidak_cocok,
        })
    }
}

/// A required string must not be blank; other types only have to be present.
fn kosong(v: &Value) -> bool {
    match v {
        Value::String(s) => s.trim().is_empty(),
        Value::Null => true,
        _ => false,
    }
}

/// Logical type check. `string` accepts only JSON scalars (strings, numbers,
/// booleans, null), never arrays or objects. Providers express numeric
/// identifiers as strings; per-field rules do the real work. `number`/`boolean`
/// stay strict.
fn tipe_cocok(v: &Value, tipe: &str) -> bool {
    match tipe {
        "number" | "integer" => v.is_number(),
        "boolean" => v.is_boolean(),
        "string" => !v.is_object() && !v.is_array(),
        _ => true,
    }
}

/// An accepted `Idempotency-Key`, normalised once so the column and the
/// uniqueness index always see the same bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Reads the header for a path. The sync path has no idempotency, so a header
/// there is `idempotency_key_tidak_didukung` (400) and never silently ignored.
pub fn baca_idempotency_key(
    header: Option<&str>,
    asinkron: bool,
) -> Result<Option<IdempotencyKey>, TolakIntake> {
    let Some(raw) = header else {
        return Ok(None);
    };
    if !asinkron {
        return Err(TolakIntake::IdempotencyKeyTidakDidukung);
    }
    let terpotong = raw.trim();
    if terpotong.is_empty()
        || terpotong.len() > IDEMPOTENCY_KEY_MAKS
        || !terpotong.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
    {
        return Err(TolakIntake::IdempotencyKeyTidakSah);
    }
    Ok(Some(IdempotencyKey(terpotong.to_owned())))
}

/// The canonical bytes a fingerprint is taken over: the endpoint plus the body
/// with object keys sorted, so key order never changes the result.
pub fn kanonik_idempoten(endpoint_id: uuid::Uuid, body: &Value) -> String {
    let mut kanonik = String::new();
    kanonik.push_str(&endpoint_id.to_string());
    kanonik.push('\n');
    tulis_kanonik(body, &mut kanonik);
    kanonik
}

/// The stored fingerprint: HMAC-SHA256 over the canonical form (ADR-0001
/// section 4.1). A bare hash is not used because a 16-digit NIK is structured
/// and easy to guess back from a digest; the key makes that infeasible.
pub fn sidik_idempoten(kunci: &[u8], endpoint_id: uuid::Uuid, body: &Value) -> String {
    use aws_lc_rs::hmac;

    let tag = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, kunci),
        kanonik_idempoten(endpoint_id, body).as_bytes(),
    );
    tag.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn tulis_kanonik(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let urut: BTreeMap<&String, &Value> = m.iter().collect();
            out.push('{');
            for (i, (k, v)) in urut.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                tulis_kanonik(v, out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, v) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                tulis_kanonik(v, out);
            }
            out.push(']');
        }
        lain => out.push_str(&lain.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_kontrak() -> Vec<FieldInputKontrak> {
        vec![
            FieldInputKontrak {
                nama: "NIK".to_owned(),
                tipe: "string".to_owned(),
                wajib: true,
            },
            FieldInputKontrak {
                nama: "NAMA_LGKAP".to_owned(),
                tipe: "string".to_owned(),
                wajib: true,
            },
        ]
    }

    #[test]
    fn body_dengan_kunci_ganda_ditolak_400() {
        let galat = baca_body(br#"{"input": {"NIK": "1", "NIK": "2"}}"#).unwrap_err();
        assert!(matches!(galat, TolakIntake::BodyTidakSah(_)));
    }

    #[test]
    fn kunci_ganda_di_kedalaman_mana_pun_ditolak() {
        let galat = baca_body(br#"{"input": {"NIK": "1"}, "metadata": {"a": {"b": 1, "b": 2}}}"#)
            .unwrap_err();
        assert!(matches!(galat, TolakIntake::BodyTidakSah(_)));
    }

    #[test]
    fn body_sah_terbaca_sebagai_json_biasa() {
        let v = baca_body(br#"{"input": {"NIK": "1"}}"#).unwrap();
        assert_eq!(v["input"]["NIK"], "1");
    }

    #[test]
    fn field_tak_dikenal_menjadi_field_tak_dikenal_bukan_nilai_minimum() {
        let input = serde_json::json!({
            "NIK": "1234567890123456",
            "NAMA_LGKAP": "Budi",
            "TRESHOLD": 10,
            "USER_ID": "x",
            "PASSWORD": "y",
            "IP_USER": "1.2.3.4",
            "transactionId": "z"
        });
        let TolakIntake::InputTidakValid { errors, .. } =
            validasi_input(&input, &input_kontrak()).unwrap_err()
        else {
            panic!("harus input tidak valid");
        };
        let nama: Vec<&str> = errors.iter().map(|e| e.field.as_str()).collect();
        for field in [
            "TRESHOLD",
            "USER_ID",
            "PASSWORD",
            "IP_USER",
            "transactionId",
        ] {
            assert!(nama.contains(&field), "{field} harus ditolak");
        }
        assert!(errors.iter().all(|e| e.kode == "field_tak_dikenal"));
    }

    #[test]
    fn field_wajib_kosong_ditolak_wajib() {
        let input = serde_json::json!({"NIK": "1234567890123456", "NAMA_LGKAP": "   "});
        let TolakIntake::InputTidakValid { errors, .. } =
            validasi_input(&input, &input_kontrak()).unwrap_err()
        else {
            panic!("harus input tidak valid");
        };
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "NAMA_LGKAP");
        assert_eq!(errors[0].kode, "wajib");
        assert!(
            errors[0].batas.is_none(),
            "batas tidak pernah nilai yang ditolak"
        );
    }

    #[test]
    fn nama_tidak_wajar_dihitung_dan_tidak_dikembalikan() {
        let panjang = "A".repeat(65);
        let mut input = serde_json::json!({"NIK": "1234567890123456", "NAMA_LGKAP": "Budi"});
        input[panjang.clone()] = serde_json::json!(1);
        input["bad-name!".to_owned()] = serde_json::json!(1);
        // A well-formed unknown key, so the test also pins what *is* echoed.
        input["TRESHOLD".to_owned()] = serde_json::json!(1);
        let TolakIntake::InputTidakValid {
            errors,
            jumlah_nama_tidak_cocok,
        } = validasi_input(&input, &input_kontrak()).unwrap_err()
        else {
            panic!("harus input tidak valid");
        };
        assert_eq!(
            errors.len(),
            1,
            "hanya nama yang bentuknya wajar yang dikembalikan"
        );
        assert_eq!(errors[0].field, "TRESHOLD");
        assert_eq!(errors[0].kode, "field_tak_dikenal");
        assert_eq!(u32::from(jumlah_nama_tidak_cocok.unwrap()), 2);
        assert!(
            errors
                .iter()
                .all(|e| e.field != panjang && e.field != "bad-name!")
        );
    }

    #[test]
    fn nama_tidak_wajar_jumlah_nol_kembali_none() {
        // Ketika tidak ada nama field yang tidak wajar, jumlah_nama_tidak_cocok harus None (absen),
        // meski ada kesalahan validasi lain (misal field wajib hilang).
        let kontrak = vec![FieldInputKontrak {
            nama: "NIK".to_owned(),
            tipe: "string".to_owned(),
            wajib: true,
        }];
        // Input kosong → NIK wajib hilang, tapi tidak ada field "tidak wajar"
        let input = serde_json::json!({});
        let TolakIntake::InputTidakValid {
            errors,
            jumlah_nama_tidak_cocok,
        } = validasi_input(&input, &kontrak).unwrap_err()
        else {
            panic!("harus input tidak valid");
        };
        assert_eq!(errors.len(), 1, "harus ada error field wajib hilang");
        assert_eq!(errors[0].field, "NIK");
        assert_eq!(errors[0].kode, "wajib");
        // jumlah_nama_tidak_cocok harus None karena tidak ada field "tidak wajar" di input
        assert!(
            jumlah_nama_tidak_cocok.is_none(),
            "jumlah_nama_tidak_cocok harus None saat tidak ada nama yang tidak wajar"
        );
    }

    #[test]
    fn input_valid_diterima() {
        let input = serde_json::json!({"NIK": "1234567890123456", "NAMA_LGKAP": "Budi"});
        assert!(validasi_input(&input, &input_kontrak()).is_ok());
    }

    #[test]
    fn field_opsional_boleh_absen_tapi_field_wajib_tidak() {
        let kontrak = vec![
            FieldInputKontrak {
                nama: "NIK".to_owned(),
                tipe: "string".to_owned(),
                wajib: true,
            },
            FieldInputKontrak {
                nama: "NAMA_LGKAP".to_owned(),
                tipe: "string".to_owned(),
                wajib: false,
            },
        ];
        let tanpa_opsional = serde_json::json!({"NIK": "1234567890123456"});
        assert!(validasi_input(&tanpa_opsional, &kontrak).is_ok());

        let tanpa_wajib = serde_json::json!({"NAMA_LGKAP": "Budi"});
        let TolakIntake::InputTidakValid { errors, .. } =
            validasi_input(&tanpa_wajib, &kontrak).unwrap_err()
        else {
            panic!("harus input tidak valid");
        };
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "NIK");
        assert_eq!(errors[0].kode, "wajib");
    }

    #[test]
    fn input_bukan_objek_ditolak() {
        let galat = validasi_input(&serde_json::json!([1, 2]), &input_kontrak()).unwrap_err();
        let TolakIntake::InputTidakValid { errors, .. } = galat else {
            panic!("harus input tidak valid");
        };
        assert_eq!(errors[0].field, "input");
        assert_eq!(errors[0].kode, "tipe_salah");
    }

    #[test]
    fn error_body_tidak_bocor_input_nik_secret() {
        // AC: error body hanya boleh punya field nama, kode, batas (jika ada);
        // tidak boleh ada nilai input, NIK, atau secret.
        let input = serde_json::json!({
            "NIK": "1234567890123456",
            "NAMA_LGKAP": "   ",
            "secret_field": "rahasia"
        });
        let TolakIntake::InputTidakValid { errors, .. } =
            validasi_input(&input, &input_kontrak()).unwrap_err()
        else {
            panic!("harus input tidak valid");
        };
        // Serialize errors ke JSON
        let errors_json = serde_json::to_string(&errors).unwrap();
        // Pastikan tidak ada NIK, nilai rahasia, atau secret di dalam body
        assert!(
            !errors_json.contains("1234567890123456"),
            "NIK bocor ke error body"
        );
        assert!(
            !errors_json.contains("rahasia"),
            "nilai secret bocor ke error body"
        );
        // Cek struktur: hanya field, kode, batas
        for err in &errors {
            assert!(!err.field.is_empty(), "field harus ada");
            assert!(!err.kode.is_empty(), "kode harus ada");
            // batas bisa None atau Some
        }
    }

    #[test]
    fn tipe_number_menolak_string() {
        let kontrak = vec![FieldInputKontrak {
            nama: "jumlah".to_owned(),
            tipe: "number".to_owned(),
            wajib: true,
        }];
        let galat = validasi_input(&serde_json::json!({"jumlah": "12"}), &kontrak).unwrap_err();
        let TolakIntake::InputTidakValid { errors, .. } = galat else {
            panic!("harus input tidak valid");
        };
        assert_eq!(errors[0].kode, "tipe_salah");
    }

    #[test]
    fn tipe_string_menolak_objek_dan_array() {
        let kontrak = vec![FieldInputKontrak {
            nama: "NIK".to_owned(),
            tipe: "string".to_owned(),
            wajib: true,
        }];
        // Objek harus ditolak untuk field string
        let galat_objek =
            validasi_input(&serde_json::json!({"NIK": {"nested": "value"}}), &kontrak).unwrap_err();
        let TolakIntake::InputTidakValid { errors, .. } = galat_objek else {
            panic!("harus input tidak valid");
        };
        assert_eq!(errors[0].kode, "tipe_salah");
        // Array harus ditolak untuk field string
        let galat_array =
            validasi_input(&serde_json::json!({"NIK": ["1", "2"]}), &kontrak).unwrap_err();
        let TolakIntake::InputTidakValid { errors, .. } = galat_array else {
            panic!("harus input tidak valid");
        };
        assert_eq!(errors[0].kode, "tipe_salah");
    }

    #[test]
    fn metadata_kunci_tak_dikenal_memakai_kode_metadata() {
        let kontrak = vec![FieldMetadataKontrak {
            nama_field: "status_penerima_manfaat".to_owned(),
            wajib: true,
            nilai_diizinkan: Some(vec!["ya".to_owned(), "tidak".to_owned()]),
        }];
        let metadata = serde_json::json!({"status_penerima_manfaat": "ya", "lain": 1});
        let TolakIntake::MetadataTidakValid { errors, .. } =
            validasi_metadata(&metadata, &kontrak).unwrap_err()
        else {
            panic!("harus metadata tidak valid");
        };
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "lain");
        assert_eq!(errors[0].kode, "field_tak_dikenal");
    }

    #[test]
    fn metadata_di_luar_daftar_tertutup_ditolak() {
        let kontrak = vec![FieldMetadataKontrak {
            nama_field: "status_penerima_manfaat".to_owned(),
            wajib: true,
            nilai_diizinkan: Some(vec!["ya".to_owned(), "tidak".to_owned()]),
        }];
        let metadata = serde_json::json!({"status_penerima_manfaat": "mungkin"});
        let TolakIntake::MetadataTidakValid { errors, .. } =
            validasi_metadata(&metadata, &kontrak).unwrap_err()
        else {
            panic!("harus metadata tidak valid");
        };
        assert_eq!(errors[0].kode, "nilai_tidak_diizinkan");
    }

    #[test]
    fn idempotency_key_pada_sync_ditolak() {
        let galat = baca_idempotency_key(Some("abc"), false).unwrap_err();
        assert_eq!(galat, TolakIntake::IdempotencyKeyTidakDidukung);
    }

    #[test]
    fn idempotency_key_terlalu_panjang_ditolak() {
        let panjang = "a".repeat(IDEMPOTENCY_KEY_MAKS + 1);
        let galat = baca_idempotency_key(Some(&panjang), true).unwrap_err();
        assert_eq!(galat, TolakIntake::IdempotencyKeyTidakSah);
    }

    #[test]
    fn idempotency_key_di_tepi_batas_diterima_dan_dirapikan() {
        let pas = "a".repeat(IDEMPOTENCY_KEY_MAKS);
        let k = baca_idempotency_key(Some(&format!("  {pas}  ")), true)
            .unwrap()
            .unwrap();
        assert_eq!(k.as_str().len(), IDEMPOTENCY_KEY_MAKS);
        assert_eq!(k.as_str(), pas);
    }

    #[test]
    fn sidik_idempoten_tidak_bergantung_urutan_kunci() {
        let e = uuid::Uuid::nil();
        let kunci = b"kunci-hmac-sintetis";
        let a = serde_json::json!({"input": {"NIK": "1", "NAMA_LGKAP": "B"}, "metadata": {}});
        let b = serde_json::json!({"metadata": {}, "input": {"NAMA_LGKAP": "B", "NIK": "1"}});
        assert_eq!(sidik_idempoten(kunci, e, &a), sidik_idempoten(kunci, e, &b));
    }

    #[test]
    fn sidik_idempoten_berbeda_untuk_isi_berbeda() {
        let e = uuid::Uuid::nil();
        let kunci = b"kunci-hmac-sintetis";
        let a = serde_json::json!({"input": {"NIK": "1"}});
        let b = serde_json::json!({"input": {"NIK": "2"}});
        assert_ne!(sidik_idempoten(kunci, e, &a), sidik_idempoten(kunci, e, &b));
    }

    #[test]
    fn sidik_idempoten_bergantung_kunci_hmac() {
        let e = uuid::Uuid::nil();
        let a = serde_json::json!({"input": {"NIK": "1"}});
        assert_ne!(
            sidik_idempoten(b"satu", e, &a),
            sidik_idempoten(b"dua", e, &a)
        );
    }

    #[test]
    fn sidik_idempoten_berbentuk_hex_64_untuk_kolom() {
        let s = sidik_idempoten(b"k", uuid::Uuid::nil(), &serde_json::json!({}));
        assert_eq!(s.len(), 64);
        assert!(
            s.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
    }

    #[test]
    fn sidik_idempoten_membedakan_endpoint() {
        let kunci = b"k";
        let a = serde_json::json!({"input": {"NIK": "1"}});
        assert_ne!(
            sidik_idempoten(kunci, uuid::Uuid::nil(), &a),
            sidik_idempoten(kunci, uuid::Uuid::max(), &a)
        );
    }
}
