//! Admin API handlers (ADR-0003 rev 5 §3.6). Each part of SHAA-196 adds a
//! module here; the OpenAPI stubs stay in `api::admin` and are the contract.

pub mod allowlist;
pub mod gerbang;
pub mod klien;
pub mod kredensial;
pub mod listener;
pub mod log;
pub mod login;
pub mod me;
pub mod pembatas;
pub mod provider;
pub mod secret;
pub mod sesi;
pub mod webhook;
