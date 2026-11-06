//! Shared domain types and infrastructure for PoolRouter components.
//!
//! The webserver, controller and worker binaries share this crate but never
//! call each other directly; they talk through PostgreSQL and the queue
//! (ADR-0001 section 2).

pub mod admin;
pub mod arsip;
pub mod audit;
pub mod config;
pub mod data_balikan;
pub mod kredensial;
pub mod kuota;
pub mod queue;
pub mod respons;
pub mod retry;
pub mod runtime;
pub mod secret;
pub mod status_api;
