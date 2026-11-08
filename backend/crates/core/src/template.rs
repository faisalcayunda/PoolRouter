//! Pure template validator for provider and endpoint bodies (ARC-49; AC-13,
//! U-46, ADR-0002 §7 test 4).
//!
//! The template is the document an operator types in the admin UI. It is
//! validated before it is stored, and the downstream request path only ever
//! reads a configuration that already passed this function (GW6).
//!
//! Two nodes can appear so far, and the grammar is deliberately small:
//! - `{{input.NAMA}}` — a client input element. The name must be in
//!   `parameters`, because an element the contract does not publish can never
//!   be filled.
//! - `{{endpoint.NAMA}}` — a literal from the endpoint config. Also checked
//!   against `parameters` here: the endpoint layers on top of the provider and
//!   reuses the same parameter table.
//! - `{{secret.NAMA}}` — a stored secret, by name only. The value never appears
//!   in a template (R-16), so the validator only asks whether the name exists.
//! - `{{system.kunci_transaksi}}` — filled by the gateway (ADR-0002 D2), so it
//!   is not checked against `parameters`.
//!
//! Anything else is an unknown namespace and is rejected.
//!
//! The function is pure: no DB, no clock, no I/O. That is what makes the
//! U-46 table a plain unit test, and it is why the handler can call it while
//! still inside its transaction.

use std::collections::BTreeSet;

/// A single template failure, positioned for the operator.
///
/// `field` is the JSON pointer of the offending member (`/body_template`), and
/// `posisi` is the 1-based line and column of the opener when the failure is
/// about a specific placeholder. A missing secret is reported with the secret
/// name in `nama` so the UI can offer to create it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateError {
    /// JSON pointer of the rejected body member, e.g. `/body_template`.
    pub field: String,
    /// Validation code, stable across releases: `template_tidak_seimbang`,
    /// `template_placeholder_tidak_dikenal`, `template_parameter_tidak_ada`,
    /// `template_secret_tidak_ada`, `config_schema_version_tidak_didukung`.
    pub kode: &'static str,
    /// 1-based line of the operator that failed, when the failure points at one.
    pub posisi: Option<BarisKolom>,
    /// Name that failed (parameter or secret), when the failure is about one.
    /// Never a value.
    pub nama: Option<String>,
}

/// 1-based line and column inside the template text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarisKolom {
    pub baris: i64,
    pub kolom: i64,
}

/// Parameter the template may reference, with the places it can be filled from.
///
/// The provider entity stores one flat parameter table (US-03), so one entry can
/// legitimately serve `{{input.*}}` and `{{endpoint.*}}` at once. The validator
/// therefore does not distinguish the two namespaces: both must name an entry
/// in this table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterTemplate {
    pub nama: String,
}

/// Split a template into its placeholder bodies and their opener positions, or
/// say where it first goes wrong.
///
/// A placeholder body may not cross a `{` or a newline, so `{{tanggal_lahir}`
/// and `{{a{b}}` are both unbalanced.
fn pecah_placeholder(template: &str) -> Result<Vec<(String, BarisKolom)>, TemplateError> {
    let mut hasil = Vec::new();
    let mut i = 0usize;
    while let Some(rel) = template[i..].find("{{") {
        let buka = i + rel;
        let awal = posisi_di(template, buka);
        // The body runs to the next `}}` and may not cross a `{` or a newline.
        let mut j = buka + 2;
        loop {
            if j >= template.len() {
                return Err(TemplateError {
                    field: String::new(),
                    kode: "template_tidak_seimbang",
                    posisi: Some(awal),
                    nama: None,
                });
            }
            let sisa = &template[j..];
            if sisa.starts_with("}}") {
                hasil.push((template[buka + 2..j].to_owned(), awal));
                i = j + 2;
                break;
            }
            let c = sisa.chars().next().expect("non-empty");
            if c == '{' || c == '\n' {
                return Err(TemplateError {
                    field: String::new(),
                    kode: "template_tidak_seimbang",
                    posisi: Some(awal),
                    nama: None,
                });
            }
            j += c.len_utf8();
        }
    }
    Ok(hasil)
}

/// 1-based line and column of byte offset `offset`, counting columns in
/// characters so a non-ASCII literal does not shift the report.
fn posisi_di(template: &str, offset: usize) -> BarisKolom {
    let depan = &template[..offset];
    let baris = depan.matches('\n').count() as i64 + 1;
    let kolom = match depan.rfind('\n') {
        Some(nl) => depan[nl + 1..].chars().count() as i64 + 1,
        None => depan.chars().count() as i64 + 1,
    };
    BarisKolom { baris, kolom }
}

/// Validate one provider/endpoint body template against the stored parameter
/// table and the names of the secrets that exist (AC-13, U-46).
///
/// `field` is the JSON pointer the errors are reported under, so the same
/// function serves provider and endpoint bodies.
///
/// An empty error vector means the template may be stored.
pub fn validasi_template(
    field: &str,
    template: &str,
    parameters: &[ParameterTemplate],
    secret_tersedia: &BTreeSet<String>,
) -> Result<(), Vec<TemplateError>> {
    let mut galat = Vec::new();
    let parameter: BTreeSet<&str> = parameters.iter().map(|p| p.nama.as_str()).collect();

    let placeholder = match pecah_placeholder(template) {
        Ok(p) => p,
        Err(mut e) => {
            e.field = field.to_owned();
            return Err(vec![e]);
        }
    };

    for (isi, posisi) in placeholder {
        let Some((namespace, nama)) = isi.split_once('.') else {
            galat.push(TemplateError {
                field: field.to_owned(),
                kode: "template_placeholder_tidak_dikenal",
                posisi: Some(posisi),
                nama: Some(isi.clone()),
            });
            continue;
        };
        match namespace {
            "secret" => {
                if !secret_tersedia.contains(nama) {
                    galat.push(TemplateError {
                        field: field.to_owned(),
                        kode: "template_secret_tidak_ada",
                        posisi: Some(posisi),
                        nama: Some(nama.to_owned()),
                    });
                }
            }
            // `input` and `endpoint` are both the provider parameter table
            // (US-03); the provider stores one flat list. `system` is filled by
            // the gateway (ADR-0002 D2) and is not an operator parameter.
            "input" | "endpoint" => {
                if !parameter.contains(nama) {
                    galat.push(TemplateError {
                        field: field.to_owned(),
                        kode: "template_parameter_tidak_ada",
                        posisi: Some(posisi),
                        nama: Some(nama.to_owned()),
                    });
                }
            }
            "system" => {}
            _ => galat.push(TemplateError {
                field: field.to_owned(),
                kode: "template_placeholder_tidak_dikenal",
                posisi: Some(posisi),
                nama: Some(isi.clone()),
            }),
        }
    }

    if galat.is_empty() { Ok(()) } else { Err(galat) }
}

/// Version gate for a stored configuration document (V-05, AC-13).
///
/// F0 writes and reads version 1. F1 adds version 2 (data_balikan block in AC-20).
/// An unknown version is a 422, never a silent upgrade and never a guess:
/// by ADR-0001 rev 6, adding a field is what bumps this number, so a version
/// this release does not know may carry fields whose meaning changed.
pub const CONFIG_SCHEMA_VERSION_F0: i64 = 1;
pub const CONFIG_SCHEMA_VERSION_F1: i64 = 2;

pub fn validasi_config_schema_version(field: &str, versi: i64) -> Result<(), TemplateError> {
    if versi == CONFIG_SCHEMA_VERSION_F0 || versi == CONFIG_SCHEMA_VERSION_F1 {
        Ok(())
    } else {
        Err(TemplateError {
            field: field.to_owned(),
            kode: "config_schema_version_tidak_didukung",
            posisi: None,
            nama: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parameter(names: &[&str]) -> Vec<ParameterTemplate> {
        names
            .iter()
            .map(|n| ParameterTemplate {
                nama: (*n).to_owned(),
            })
            .collect()
    }

    fn rahasia(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    /// The canonical provider body: input, endpoint literal and secret in one
    /// document. `USER_ID`/`TRESHOLD`/`NAMA_LGKP` are the names AC-13 says must
    /// be stored verbatim.
    const DUKCAPIL: &str = "{\"NIK\":\"{{input.NAMA_LGKP}}\",\
\"UID\":\"{{secret.USER_ID}}\",\
\"TRESHOLD\":\"{{endpoint.TRESHOLD}}\"}";

    fn peta_dukcapil() -> (Vec<ParameterTemplate>, BTreeSet<String>) {
        (
            parameter(&["NAMA_LGKP", "TRESHOLD"]),
            rahasia(&["USER_ID", "PASSWORD"]),
        )
    }

    #[test]
    fn template_dukcapil_lolos() {
        let (p, s) = peta_dukcapil();
        assert_eq!(
            validasi_template("/body_template", DUKCAPIL, &p, &s),
            Ok(())
        );
    }

    #[test]
    fn kurung_tidak_seimbang_melaporkan_posisi() {
        // `{{tanggal_lahir}` closes with one brace: the opener at column 1 of
        // line 1 never finds `}}`.
        let (p, s) = peta_dukcapil();
        let err = validasi_template("/body_template", "{{tanggal_lahir}", &p, &s).unwrap_err();
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].kode, "template_tidak_seimbang");
        assert_eq!(err[0].posisi, Some(BarisKolom { baris: 1, kolom: 1 }));
    }

    #[test]
    fn kurung_tidak_seimbang_di_baris_kedua_melaporkan_baris_dua() {
        let (p, s) = peta_dukcapil();
        let err =
            validasi_template("/body_template", "{\n  \"a\": \"{{x}\"\n}", &p, &s).unwrap_err();
        assert_eq!(err[0].kode, "template_tidak_seimbang");
        // Line 2 is `  "a": "{{x}"`: two spaces, `"a": "` is 6 columns, so the
        // opener sits at column 9.
        assert_eq!(err[0].posisi, Some(BarisKolom { baris: 2, kolom: 9 }));
    }

    #[test]
    fn kurung_kurawal_tunggal_di_luar_placeholder_bukan_galat() {
        // JSON braces are literal; only `{{` opens a placeholder.
        let (p, s) = peta_dukcapil();
        assert_eq!(
            validasi_template("/body_template", "{\"a\": {\"b\": 1}}", &p, &s),
            Ok(())
        );
    }

    #[test]
    fn secret_yang_tidak_ada_dilaporkan_dengan_namanya() {
        let (p, mu) = peta_dukcapil();
        let err = validasi_template("/body_template", "{{secret.TIDAK_ADA}}", &p, &mu).unwrap_err();
        assert_eq!(err[0].kode, "template_secret_tidak_ada");
        assert_eq!(err[0].nama.as_deref(), Some("TIDAK_ADA"));
        // The secret name is echoed so the UI can offer to create it; it is a
        // name, never a value (R-16).
        assert_eq!(err[0].posisi, Some(BarisKolom { baris: 1, kolom: 1 }));
    }

    #[test]
    fn input_dan_endpoint_divalidasi_terhadap_parameters() {
        let (p, s) = peta_dukcapil();
        let err = validasi_template("/body_template", "{{input.TIDAK_ADA}}", &p, &s).unwrap_err();
        assert_eq!(err[0].kode, "template_parameter_tidak_ada");
        assert_eq!(err[0].nama.as_deref(), Some("TIDAK_ADA"));

        let err =
            validasi_template("/body_template", "{{endpoint.TIDAK_ADA}}", &p, &s).unwrap_err();
        assert_eq!(err[0].kode, "template_parameter_tidak_ada");
    }

    #[test]
    fn kunci_template_tersimpan_persis_termasuk_huruf_besar() {
        // AC-13: `USER_ID`, `TRESHOLD`, `NAMA_LGKP` are matched case-sensitively;
        // a lower-case spelling is a different (absent) name.
        let (p, s) = peta_dukcapil();
        let err = validasi_template("/body_template", "{{input.nama_lgkp}}", &p, &s).unwrap_err();
        assert_eq!(err[0].kode, "template_parameter_tidak_ada");
        assert_eq!(err[0].nama.as_deref(), Some("nama_lgkp"));
    }

    #[test]
    fn namespace_system_tidak_dicek_ke_parameters() {
        // ADR-0002 D2: `{{system.kode_transaksi}}` is filled by the gateway.
        let (p, s) = peta_dukcapil();
        assert_eq!(
            validasi_template("/body_template", "{{system.kode_transaksi}}", &p, &s),
            Ok(())
        );
    }

    #[test]
    fn namespace_tak_dikenal_ditolak() {
        let (p, s) = peta_dukcapil();
        let err = validasi_template("/body_template", "{{foo.bar}}", &p, &s).unwrap_err();
        assert_eq!(err[0].kode, "template_placeholder_tidak_dikenal");
        assert_eq!(err[0].nama.as_deref(), Some("foo.bar"));
    }

    #[test]
    fn placeholder_tanpa_titik_ditolak() {
        let (p, s) = peta_dukcapil();
        let err = validasi_template("/body_template", "{{USER_ID}}", &p, &s).unwrap_err();
        assert_eq!(err[0].kode, "template_placeholder_tidak_dikenal");
    }

    #[test]
    fn semua_galat_dikumpulkan_bukan_hanya_yang_pertama() {
        let (p, s) = peta_dukcapil();
        let err = validasi_template(
            "/body_template",
            "{{secret.A}} {{input.B}} {{foo.C}}",
            &p,
            &s,
        )
        .unwrap_err();
        assert_eq!(err.len(), 3);
        let kode: Vec<_> = err.iter().map(|e| e.kode).collect();
        assert_eq!(
            kode,
            vec![
                "template_secret_tidak_ada",
                "template_parameter_tidak_ada",
                "template_placeholder_tidak_dikenal"
            ]
        );
        // Columns are the three openers in order: `{{secret.A}}` is 12 columns,
        // the space makes 13, so the next opener is at 14; `{{input.B}}` is 11
        // columns ending at 24, the space makes 25, so the last opener is at 26.
        assert_eq!(
            err.iter()
                .map(|e| e.posisi.unwrap().kolom)
                .collect::<Vec<_>>(),
            vec![1, 14, 26]
        );
    }

    #[test]
    fn field_diteruskan_ke_setiap_galat() {
        let (p, s) = peta_dukcapil();
        let err =
            validasi_template("/respons_skema/x", "{{secret.A}}{{secret.B}}", &p, &s).unwrap_err();
        assert!(err.iter().all(|e| e.field == "/respons_skema/x"));
    }

    #[test]
    fn posisi_menghitung_karakter_bukan_byte() {
        // `é` is two bytes in UTF-8 but one column. `"kota":"é" ` is 11 columns,
        // so the opener is at 12; byte arithmetic would have said 13.
        let (p, s) = peta_dukcapil();
        let err =
            validasi_template("/body_template", "\"kota\":\"é\" {{secret.X}}", &p, &s).unwrap_err();
        assert_eq!(
            err[0].posisi,
            Some(BarisKolom {
                baris: 1,
                kolom: 12
            })
        );
    }

    #[test]
    fn template_kosong_lolos() {
        let (p, s) = peta_dukcapil();
        assert_eq!(validasi_template("/body_template", "", &p, &s), Ok(()));
    }

    #[test]
    fn versi_skema_selain_satu_ditolak() {
        assert_eq!(
            validasi_config_schema_version("/config_schema_version", 1),
            Ok(())
        );
        for v in [0, 2, 99] {
            let e = validasi_config_schema_version("/config_schema_version", v).unwrap_err();
            assert_eq!(e.kode, "config_schema_version_tidak_didukung");
            assert_eq!(e.field, "/config_schema_version");
        }
    }
}
