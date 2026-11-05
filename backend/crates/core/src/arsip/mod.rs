//! Archive format for the F1 retention lane: an encrypted Parquet object plus
//! the storage port that carries it (ADR-0006 section 8; f1-dashboard-arsip
//! T3, DA9, DA10, DA21).
//!
//! This module is deliberately pure. It knows how to turn a stream of
//! [`BarisArsip`] into one Parquet object whose `nik` and payload columns are
//! ciphertext, how to read it back, how to find a NIK through the `nik_hmac`
//! bloom filter without pulling the whole object down, and how to check an
//! object against its [`RingkasanObjek`]. It does **not** touch PostgreSQL and
//! does not know which bucket it lands in — the job that decides that is T4, the
//! bucket and lane are T7. The only seam to the outside world is
//! [`PenyimpanArsip`], a three-method port with two implementations
//! ([`Memori`] for tests and [`object_store::ObjectStore`] for S3).
//!
//! Four decisions are enforced here rather than at the call site:
//!
//! - **Rows are sorted by `nik_hmac` then `kode_transaksi`** before they are
//!   written, and the file records that sort order. The bloom filter per row
//!   group is only useful if a lookup can skip the groups that cannot hold the
//!   key, and Parquet's per-group statistics on the sorted column are what make
//!   the range read cheap (AC-17).
//! - **`nik` and the payload columns are AES-256-GCM ciphertext** under a random
//!   data key per object, and that data key is wrapped by a [`PembungkusDek`]
//!   before it leaves the module (AC-16). No plaintext NIK or payload is ever
//!   written, in any environment.
//! - **The port has no delete.** An archive that could be deleted through this
//!   type would defeat the retention requirement it exists for (AC-19).
//! - **Nothing that carries a NIK or a payload prints it.** `Debug`/`Display`
//!   redact through [`NilaiAman`]; the values are reachable only by
//!   `expose()` (AC-20).
//!
//! Round-trip, ciphertext, bloom-filter, verification and contract-suite tests
//! live in `tests_arsip.rs` so this file stays about behaviour, not assertions.

// The module is a tracer bullet (DA5): T3 lands the format, port and crypto, and
// T4/T7 wire it into the archive job and the bucket lane. Until then several
// items have no caller outside this module, which `dead_code` would flag.
#![allow(dead_code)]

mod baca;
mod baris;
mod galat;
mod kunci;
mod port;
mod tulis;
mod verifikasi;

pub use baca::{BacaArsip, SaringanNik};
pub use baris::{BarisArsip, KolomArsip, RentangKodeTransaksi, VersiKunciHmac};
pub use galat::{GalatArsip, GalatVerifikasi};
pub use kunci::{Dek, DekTerbungkus, KunciObjek, PembungkusDek, PembungkusKekAwal};
pub use port::{Memori, PenyimpanArsip, RingkasanObjek, S3};
pub use tulis::{ParameterTulis, TulisArsip};
pub use verifikasi::Verifikasi;

/// Rows per Parquet row group (DA9: 128.000). AC-15 checks that 300.000 rows
/// land in more than one group, so this constant is the one place the number
/// appears.
pub const BARIS_PER_ROW_GROUP: usize = 128_000;

/// The compression the archive is always written with (DA9). zstd is the
/// format decision, not a caller choice, so it is not a field of
/// [`ParameterTulis`].
pub const KOMPRESI_ARSIP: &str = "zstd";

/// Name of the leaf the bloom filter is built on (DA9: only `nik_hmac`).
pub const KOLOM_BLOOM: &str = "nik_hmac";

#[cfg(test)]
mod tests_arsip;
