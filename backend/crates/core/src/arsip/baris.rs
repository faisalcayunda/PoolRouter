//! One archive row and the value types around it (DA9, DA10, AC-16, AC-20).
//!
//! [`BarisArsip`] is the *decrypted* row the job hands to the writer and gets
//! back from the reader. The ciphertext lives on the object, not in this struct:
//! the writer seals `nik` and the payload on the way out and the reader opens
//! them on the way in, so a caller never handles a sealed cell and cannot
//! accidentally write one back.
//!
//! The struct keeps the NIK behind [`NilaiAman`] and implements `Debug` by hand.
//! Without that, `tracing`'s `?baris` and a `{:?}` on a `Vec<BarisArsip>` in a
//! test assertion would print 16-digit NIKs into a log — which is the leak
//! AC-20 exists to prevent.

use std::fmt;

use chrono::{DateTime, Utc};

use crate::admin::aman::{Nik, NilaiAman};
use crate::audit::NikHmac;

/// A column the archive carries beyond the bookkeeping ones.
///
/// The list is closed on purpose: an unknown column would need a Parquet schema
/// decision, and the format is a contract (DA9), not something the job grows at
/// runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KolomArsip {
    /// `kode_transaksi`, the sort tiebreaker and the range in the summary.
    KodeTransaksi,
    /// Everything else, sealed like the payload.
    Payload,
}

/// Version of the NIK HMAC key the row was hashed with (DA11).
///
/// F1 has exactly one version. The type exists now so the object already
/// carries the field when rotation arrives, and so restore can answer 409 for a
/// version it does not have instead of hashing with the wrong key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VersiKunciHmac(i32);

impl VersiKunciHmac {
    /// The only version F1 writes.
    pub const F1: Self = Self(1);

    /// Build a version from the stored integer. Rejects <= 0: the database
    /// column is `>= 1`, and a zero would make "missing" indistinguishable from
    /// "version zero".
    pub fn baru(v: i32) -> Result<Self, VersiKunciHmacTidakValid> {
        if v >= 1 {
            Ok(Self(v))
        } else {
            Err(VersiKunciHmacTidakValid)
        }
    }

    /// The stored integer.
    pub const fn nilai(self) -> i32 {
        self.0
    }
}

/// A key version below 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersiKunciHmacTidakValid;

impl fmt::Display for VersiKunciHmacTidakValid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("versi kunci HMAC harus >= 1")
    }
}

impl std::error::Error for VersiKunciHmacTidakValid {}

/// The closed/open range of `kode_transaksi` in one object, as recorded in
/// [`super::RingkasanObjek`].
///
/// `kode_transaksi` is a UUIDv7 in this product, so ordering the strings is the
/// same as ordering the instants they encode, and a plain lexicographic range
/// answers "did this object cover that window" without a second index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RentangKodeTransaksi {
    /// Smallest `kode_transaksi` in the object.
    pub dari: String,
    /// Largest `kode_transaksi` in the object.
    pub sampai: String,
}

impl RentangKodeTransaksi {
    /// The range of a set of rows, or `None` when there are no rows to bound.
    pub fn dari_baris<'a>(baris: impl IntoIterator<Item = &'a BarisArsip>) -> Option<Self> {
        let mut it = baris.into_iter();
        let first = it.next()?;
        let mut dari = first.kode_transaksi.clone();
        let mut sampai = dari.clone();
        for b in it {
            if b.kode_transaksi < dari {
                dari.clone_from(&b.kode_transaksi);
            }
            if b.kode_transaksi > sampai {
                sampai.clone_from(&b.kode_transaksi);
            }
        }
        Some(Self { dari, sampai })
    }
}

/// One row of an archive object, in the clear.
///
/// `nik` is optional because not every archived table carries one
/// (ADR-0006 section 8: only `transaction_log` does); `payload` is optional for
/// the same reason.
#[derive(Clone, PartialEq, Eq)]
pub struct BarisArsip {
    /// The transaction the row belongs to. Sort tiebreaker.
    pub kode_transaksi: String,
    /// `HMAC-SHA256(nik)` under the key version below. The only column with a
    /// bloom filter.
    pub nik_hmac: NikHmac,
    /// Key version used to derive [`Self::nik_hmac`].
    pub kunci_hmac_versi: VersiKunciHmac,
    /// The plaintext NIK. Sealed on the way to the object; redacted in
    /// `Debug`/`Display`; reachable only through `.expose()`.
    pub nik: Option<NilaiAman<Nik>>,
    /// The free-form payload (request/response JSON in `transaction_log`).
    /// Sealed on the way out and redacted in `Debug`.
    pub payload: Option<NilaiAman<String>>,
    /// When the request was made. Kept in the clear: it is not personal data
    /// and restore filters on it.
    pub terjadi_at: DateTime<Utc>,
}

impl BarisArsip {
    /// A row with a NIK and a payload, the `transaction_log` shape.
    pub fn baru(
        kode_transaksi: impl Into<String>,
        nik_hmac: NikHmac,
        nik: Nik,
        payload: impl Into<String>,
        terjadi_at: DateTime<Utc>,
    ) -> Self {
        Self {
            kode_transaksi: kode_transaksi.into(),
            nik_hmac,
            kunci_hmac_versi: VersiKunciHmac::F1,
            nik: Some(NilaiAman::baru(nik)),
            payload: Some(NilaiAman::baru(payload.into())),
            terjadi_at,
        }
    }

    /// A row without a NIK, for the companion tables that T5 archives.
    pub fn tanpa_nik(
        kode_transaksi: impl Into<String>,
        nik_hmac: NikHmac,
        terjadi_at: DateTime<Utc>,
    ) -> Self {
        Self {
            kode_transaksi: kode_transaksi.into(),
            nik_hmac,
            kunci_hmac_versi: VersiKunciHmac::F1,
            nik: None,
            payload: None,
            terjadi_at,
        }
    }

    /// The sort key the format promises: `nik_hmac`, then `kode_transaksi`
    /// (DA9). Exposed so a caller can sort before it streams and so the writer
    /// can refuse to write out of order.
    pub fn kunci_urut(&self) -> (&str, &str) {
        (self.nik_hmac.as_str(), &self.kode_transaksi)
    }
}

impl fmt::Debug for BarisArsip {
    /// Redacts both personal columns. A test asserts the masked NIK and the
    /// literal `***` appear here while the real values do not (AC-20).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarisArsip")
            .field("kode_transaksi", &self.kode_transaksi)
            .field("nik_hmac", &self.nik_hmac)
            .field("kunci_hmac_versi", &self.kunci_hmac_versi)
            .field("nik", &self.nik) // NilaiAman -> `NilaiAman(***)`
            .field("payload", &self.payload.as_ref().map(|_| "***"))
            .field("terjadi_at", &self.terjadi_at)
            .finish()
    }
}

impl fmt::Display for BarisArsip {
    /// One line for a log or a CLI: transaction, masked NIK, no payload.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.nik {
            Some(nik) => write!(
                f,
                "{} nik={} {}",
                self.kode_transaksi,
                nik.expose(),
                self.terjadi_at
            ),
            None => write!(f, "{} nik=- {}", self.kode_transaksi, self.terjadi_at),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nik() -> Nik {
        Nik::parse("3201234567890001").unwrap()
    }

    fn hmac() -> NikHmac {
        NikHmac::parse("ab".repeat(32)).unwrap()
    }

    #[test]
    fn debug_merahasiakan_nik_dan_payload() {
        let b = BarisArsip::baru(
            "0192f0aa-0000-7000-8000-000000000001",
            hmac(),
            nik(),
            "{\"respons\":\"rahasia-payload\"}",
            Utc::now(),
        );
        let teks = format!("{b:?}");
        assert!(!teks.contains("3201234567890001"), "NIK bocor: {teks}");
        assert!(!teks.contains("rahasia-payload"), "payload bocor: {teks}");
        // `Debug` is for logs: the NIK is fully redacted, not merely masked.
        assert!(
            teks.contains("NilaiAman(***)"),
            "nik tidak diredaksi: {teks}"
        );
    }

    #[test]
    fn display_tersamar_tanpa_payload() {
        let b = BarisArsip::baru(
            "0192f0aa-0000-7000-8000-000000000001",
            hmac(),
            nik(),
            "rahasia-payload",
            Utc::now(),
        );
        let teks = b.to_string();
        assert!(!teks.contains("3201234567890001"));
        assert!(!teks.contains("rahasia-payload"));
        assert!(teks.contains("3201********0001"));
    }

    #[test]
    fn versi_nol_ditolak() {
        assert!(VersiKunciHmac::baru(0).is_err());
        assert!(VersiKunciHmac::baru(-1).is_err());
        assert_eq!(VersiKunciHmac::baru(1).unwrap(), VersiKunciHmac::F1);
    }

    #[test]
    fn rentang_kosong_tanpa_baris() {
        assert_eq!(RentangKodeTransaksi::dari_baris(std::iter::empty()), None);
    }
}
