//! Shared domain types and infrastructure for PoolRouter components.
//!
//! The webserver, controller and worker binaries share this crate but never
//! call each other directly; they talk through PostgreSQL and the queue
//! (ADR-0001 section 2).

pub mod admin;
pub mod audit;
pub mod config;
pub mod kredensial;
pub mod kuota;
pub mod queue;
pub mod retry;
pub mod runtime;
