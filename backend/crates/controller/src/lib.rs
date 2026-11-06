//! Controller process of PoolRouter (ARC-69, R2 T2 of `f0-controller-worker`).
//!
//! Exposed as a library so the integration tests in `tests/` can drive the
//! leader lock, the scheduler and the jobs against a real PostgreSQL and Redis
//! (CW10 seams a, b and c). The binary in `src/main.rs` only wires the modules
//! together.

pub mod balikan;
pub mod data_balikan;
pub mod job_balikan;
pub mod jadwal;
pub mod leader;
pub mod rebuild;
pub mod snapshot;
pub mod webhook;
