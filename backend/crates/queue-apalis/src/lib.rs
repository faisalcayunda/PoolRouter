//! Queue adapter (ADR-0001 section 6, ADR-0004 section 4.3).
//!
//! Consume goes through apalis-amqp, publish goes through `lapin` directly.
//! Apalis and lapin types must not leave this crate. The port traits
//! (`JobPublisher` / `JobConsumer`) and the adapter itself arrive in a later
//! sub-issue.

// Keep the pinned dependencies linked so the dependency tree is checked in CI.
use apalis as _;
use apalis_amqp as _;
use lapin as _;
use poolrouter_core as _;
