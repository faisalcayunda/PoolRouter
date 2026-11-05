//! Admin API handlers (ADR-0003 rev 5 §3.6). Each part of SHAA-196 adds a
//! module here; the OpenAPI stubs stay in `api::admin` and are the contract.

pub mod gerbang;
pub mod listener;
pub mod login;
pub mod me;
pub mod mfa;
pub mod pembatas;
pub mod secret;
pub mod sesi;
