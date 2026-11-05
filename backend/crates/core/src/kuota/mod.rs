//! Daily quota, rolling window and rate limit (module M-02 of the SHAA-175
//! hand-off).
//!
//! Authority: ADR-0001 rev 15 section 5a.1 (quota, period and bucket),
//! section 5a.2 (rebuild after failover), section 5a.3 (retry budget) and
//! section 5 (fail-closed).
//!
//! Two rules shape this module:
//!
//! - **The period is computed once, in Rust.** ADR-0001 rev 15 section 5a.1
//!   point 1d: the period start, the period end and the 10-minute bucket index
//!   are derived from `requested_at` and sent to Redis as arguments. No script
//!   reads the clock (`TIME`) to pick a key, otherwise an attempt stamped
//!   23:59:59.9 that increments at 00:00:00.1 lands in a different period than
//!   its log row and DQ-05 reports a false positive around the reset.
//! - **Fail-closed.** An unusable configuration, an unknown zone, a missing
//!   argument or an unprepared Redis never counts as "quota available"
//!   (ADR-0001 rev 15 section 5). Every such state is reported to the caller as
//!   an error or as `belum_siap`; the mapping to a problem code stays in one
//!   place outside this module.

pub mod alokasi;
pub mod metrik;
pub mod pemulihan;
pub mod periode;
pub mod rencana;
pub mod store;
pub mod sumber_pg;

pub use alokasi::{Anggaran, GalatAlokasi, JenisAlokasi, NilaiAlokasi};
pub use periode::{
    AturanReset, GalatKuota, JUMLAH_EMBER_HARIAN, KalenderZona, MENIT_EMBER, Periode, ZonaTetap,
    ember_mulai, indeks_ember, nomor_ember, periode_untuk,
};
pub use rencana::{
    PoolPeminjam, RencanaAnggaran, Rute, Sumber, SumberDana, rencana_anggaran, ruang_intake, urutan_sumber,
};
