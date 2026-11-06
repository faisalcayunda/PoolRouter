//! Downstream API runtime (SHAA-198): client auth, probes, shared HTTP layers.
//! Contracts live in `crate::api`; this module implements them.

pub mod auth;
pub mod credential;
pub mod downstream_sesi;
pub mod gcra;
pub mod jwt;
pub mod layers;
pub mod pengenal;
pub mod probe;
pub mod server;
pub mod token;
