//! Admin identity primitives shared by the webserver and the CLI (ADR-0003
//! rev 5 section 3.6, §3.6a, §3.6c).
//!
//! Slice 1 of SHAA-196: password hashing, the opaque session token, the session
//! time rules, the `Origin` check, and `NilaiAman` with the NIK helpers. State
//! that needs the database (issuing a session, the login limiter, the CLI
//! commands) builds on these and lives in the component that owns it.
//!
//! The split is deliberate: everything here is a pure function of its inputs, so
//! the security-relevant behaviour — the exact argon2id parameters, the fact
//! that `perpanjangSesi` cannot move the 8-hour cap, the masking form — is
//! testable without a Postgres instance and cannot drift into a different
//! implementation between the webserver and the CLI.
//!
//! Not here, on purpose:
//! - session *storage* (issued/revoked rows): webserver, with `sqlx`;
//! - the login rate limiter (5 per 15 minutes): webserver, needs shared state;
//! - the one-time secret issuance (§3.6a): slice 3.
//!
//! Layanan pengguna dan peran (§3.6c) ada di [`kelola_pengguna`]; antarmuka baris perintahnya di
//! crate `poolrouter-cli` (ARC-55).

pub mod aman;
pub mod kelola_pengguna;
pub mod mfa;
pub mod origin;
pub mod password;
pub mod pengguna;
pub mod pra_sesi;
pub mod secret;
pub mod seed_dukcapil;
pub mod sesi;
pub mod sesi_store;
pub mod token;

pub use aman::{Nik, NilaiAman, mengandung_potongan_nik, tersamar as tersamar_nik, tersamar_audit};
pub use origin::{AsalUi, OriginDitolak};
pub use password::{KebijakanPassword, ParameterArgon2};
pub use pra_sesi::{BATAS_GAGAL, GalatPraSesi, JenisPraSesi, PraSesi, PraSesiStore, UMUR_DETIK};
pub use secret::{PANJANG_SECRET, SecretSekali};
pub use sesi::{BatasSesi, HasilPerpanjang, KeadaanSesi, SesiKedaluwarsa, StatusSesi};
pub use sesi_store::{GalatSesi, SesiDitolak, SesiStore, SesiTersimpan};
pub use token::TokenSesi;
