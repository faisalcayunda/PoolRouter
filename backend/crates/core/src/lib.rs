//! Shared domain types and infrastructure for PoolRouter components.
//!
//! The webserver, controller and worker binaries share this crate but never
//! call each other directly; they talk through PostgreSQL and the queue
//! (ADR-0001 section 2).

pub mod config;
pub mod kuota;
pub mod runtime;
pub mod queue;
