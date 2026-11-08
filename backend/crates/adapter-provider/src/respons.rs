//! Provider response parsing (ADR-0002 D3).
//!
//! The `status_skor` rules — envelope shape, literal table, score split —
//! belong to `poolrouter_core::respons::status_skor` (ARC-46). This module
//! adds only what core does not do: decoding raw provider bytes into a
//! `Value` and reporting a decode failure the same way the adapter reports
//! every other invalid response (U-43).
//!
//! Keeping the parsing rules in one place is the whole point: R1 T5 uses the
//! pilot module rather than reimplementing it.

use poolrouter_core::respons::{
    HasilElemen, KonfigurasiStatusSkor, ResponsTidakValid, parse_respons,
};
use serde_json::Value;
use thiserror::Error;

/// A body the adapter could not turn into a `status_skor` result.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BacaResponsError {
    /// Not JSON, or not UTF-8 JSON.
    #[error("respons bukan JSON")]
    BukanJson,
    /// The pilot parser rejected the body.
    #[error("{0}")]
    TidakValid(#[from] ResponsTidakValid),
}

/// One datapath element read from the provider response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HasilBaca {
    /// Element name as the contract spells it (e.g. `NAMA_LGKP`).
    pub elemen: String,
    pub hasil: HasilElemen,
}

/// Decode `body` and read every configured element.
///
/// One bad element fails the whole response: F0 emits no partial output
/// (U-43). The caller maps this to `ErrorClass::ResponsTidakValid`, which
/// `retryable` reports as final.
pub fn baca_respons(
    body: &[u8],
    konfigurasi: &KonfigurasiStatusSkor,
) -> Result<Vec<HasilBaca>, BacaResponsError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| BacaResponsError::BukanJson)?;
    let peta = parse_respons(konfigurasi, &value)?;
    Ok(peta
        .into_iter()
        .map(|(elemen, hasil)| HasilBaca { elemen, hasil })
        .collect())
}
