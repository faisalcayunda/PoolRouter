//! Downstream API runtime (SHAA-198): client auth, probes, shared HTTP layers.
//! Contracts live in `crate::api`; this module implements them.

pub mod auth;
pub mod credential;
pub mod gcra;
pub mod gerbang;
pub mod intake;
pub mod jwt;
pub mod layers;
pub mod probe;
pub mod server;
pub mod token;
