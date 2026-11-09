//! Admin API handlers (ADR-0003 rev 5 §3.6). Each part of SHAA-196 adds a
//! module here; the OpenAPI stubs stay in `api::admin` and are the contract.

pub mod allowlist;
pub mod endpoint;
pub mod gerbang;
pub mod k20;
pub mod k20_entitas;
pub mod klien;
pub mod kredensial;
pub mod listener;
pub mod log;
pub mod login;
pub mod me;
pub mod mfa;
pub mod pembatas;
pub mod pool;
pub mod provider;
pub mod secret;
pub mod sesi;
pub mod webhook;
