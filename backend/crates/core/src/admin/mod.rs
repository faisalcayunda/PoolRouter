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
//! - the one-time secret issuance (§3.6a): slice 3;
//! - the user/role CLI (§3.6c): slice 4.

pub mod aman;
pub mod origin;
pub mod password;
pub mod secret;
pub mod sesi;
pub mod token;

pub use aman::{Nik, NilaiAman, tersamar as tersamar_nik};
pub use origin::{AsalUi, OriginDitolak};
pub use password::{KebijakanPassword, ParameterArgon2};
pub use secret::{SecretSekali, PANJANG_SECRET};
pub use sesi::{BatasSesi, HasilPerpanjang, KeadaanSesi, SesiKedaluwarsa, StatusSesi};
pub use token::TokenSesi;
