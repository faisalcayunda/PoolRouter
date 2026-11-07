//! Request template rendering (ADR-0002 D1, D2).
//!
//! A template is provider configuration, not code: method, path, query,
//! headers and an optional body, with `{{...}}` placeholders. The model is
//! the same for every HTTP method (D1), so Dukcapil needs no special case
//! (G-01).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Method plus path, query, headers and optional body.
///
/// `body` stays optional because D1 permits a body on any method but never
/// requires one (a Dukcapil GET carries none).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateRequest {
    pub method: String,
    pub path: String,
    pub query: BTreeMap<String, String>,
    pub header: BTreeMap<String, String>,
    pub body: Option<String>,
}

/// Values a template can refer to.
///
/// The allowed namespaces are a closed set (ADR-0001 §7, ADR-0002 D2):
/// `input.*` (client fields on the endpoint's input contract), `secret.*`
/// (provider credentials), `system.kode_transaksi` (the only `system.*` key
/// in F0) and `endpoint.*` (endpoint `parameter_literal`). Anything else is
/// rejected, so an unknown namespace never reaches a provider.
#[derive(Clone, Default)]
pub struct Nilai {
    pub input: BTreeMap<String, String>,
    pub secret: BTreeMap<String, String>,
    pub sistem: BTreeMap<String, String>,
    pub endpoint: BTreeMap<String, String>,
}

impl std::fmt::Debug for Nilai {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Nilai")
            .field("input", &self.input)
            .field("secret", &"<redacted>")
            .field("sistem", &self.sistem)
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

/// The only `{{system.*}}` key F0 recognises (ADR-0002 D2).
pub const KUNCI_SISTEM_F0: &str = "kode_transaksi";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RenderError {
    /// A placeholder names a value nobody supplied.
    #[error("placeholder tak dikenal: {{{0}}}")]
    PlaceholderTakDikenal(String),
    /// A `{{` never closed with `}}`.
    #[error("kurung kurawal tidak utuh pada posisi {0}")]
    KurungTidakUtuh(usize),
    /// A placeholder with no source prefix, e.g. `{{no_prefix}}`.
    #[error("placeholder tanpa awalan sumber: {{{0}}}")]
    TanpaAwalan(String),
    /// A placeholder with an empty body, e.g. `{{}}`.
    #[error("placeholder kosong: {{{0}}}")]
    Kosong(String),
}

impl Nilai {
    fn cari(&self, nama: &str) -> Result<&str, RenderError> {
        let (awalan, kunci) = nama
            .split_once('.')
            .ok_or_else(|| RenderError::TanpaAwalan(nama.to_string()))?;
        if kunci.is_empty() {
            return Err(RenderError::Kosong(nama.to_string()));
        }
        let peta = match awalan {
            "input" => &self.input,
            "secret" => &self.secret,
            "endpoint" => &self.endpoint,
            "system" => {
                // F0 closes the namespace to one key; any other system.*
                // reference is rejected like an unknown variable
                // (ADR-0002 D2).
                if kunci != KUNCI_SISTEM_F0 {
                    return Err(RenderError::PlaceholderTakDikenal(nama.to_string()));
                }
                &self.sistem
            }
            // An unknown namespace is as unresolved as an unknown key: it
            // can never be filled, so it must fail before any call goes out.
            _ => return Err(RenderError::PlaceholderTakDikenal(nama.to_string())),
        };
        peta.get(kunci)
            .map(String::as_str)
            .ok_or_else(|| RenderError::PlaceholderTakDikenal(nama.to_string()))
    }
}

/// Replace every `{{...}}` in `teks`.
///
/// Rendering fails closed: one unresolved placeholder aborts the render, so
/// a provider never receives a partially filled request (U-43).
pub fn render_teks(teks: &str, nilai: &Nilai) -> Result<String, RenderError> {
    let mut hasil = String::with_capacity(teks.len());
    let mut sisa = teks;
    while let Some(buka) = sisa.find("{{") {
        hasil.push_str(&sisa[..buka]);
        let setelah = &sisa[buka + 2..];
        let tutup = setelah
            .find("}}")
            .ok_or(RenderError::KurungTidakUtuh(teks.len() - sisa.len() + buka))?;
        let nama = setelah[..tutup].trim();
        if nama.is_empty() {
            return Err(RenderError::Kosong(String::new()));
        }
        hasil.push_str(nilai.cari(nama)?);
        sisa = &setelah[tutup + 2..];
    }
    // A lone `}}` without an opener is literal text, matching how template
    // engines treat unmatched closers; only unclosed openers are an error.
    hasil.push_str(sisa);
    Ok(hasil)
}

/// Render every placeholder in the template.
pub fn render_template(
    template: &TemplateRequest,
    nilai: &Nilai,
) -> Result<TemplateRequest, RenderError> {
    let render_map = |peta: &BTreeMap<String, String>| {
        peta.iter()
            .map(|(k, v)| Ok((k.clone(), render_teks(v, nilai)?)))
            .collect::<Result<BTreeMap<_, _>, RenderError>>()
    };

    Ok(TemplateRequest {
        method: template.method.clone(),
        path: render_teks(&template.path, nilai)?,
        query: render_map(&template.query)?,
        header: render_map(&template.header)?,
        body: match &template.body {
            Some(b) => Some(render_teks(b, nilai)?),
            None => None,
        },
    })
}
