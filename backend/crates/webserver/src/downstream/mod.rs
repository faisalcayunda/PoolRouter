//! Downstream API runtime (SHAA-198): client auth, probes, shared HTTP layers.
//! Contracts live in `crate::api`; this module implements them.

pub mod gcra;
pub mod layers;
pub mod probe;
