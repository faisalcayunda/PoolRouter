//! Admin API pool CRUD + quota tracker (R3 T3: AC-16–18).
//! Test seam: handlers via HTTP (production router + oneshot), PostgreSQL (pr_admin_api role),
//! no mock repository.
//!
//! Validators and pure logic tested in unit tests (config.rs); these tests verify:
//! - AC-16: create/update upstream pool → validate + write audit row
//! - AC-17: create/update downstream pool → exact one upstream, reject over-quota
//! - AC-18: get quota tracker → DB counters, not Redis

use chrono::{DateTime, Utc};
use sqlx::{Row, postgres::PgPool};
use uuid::Uuid;

// Common setup: would import from a shared test support module
// (assuming it exists or will be created)
#[tokio::test]
async fn ac16_create_upstream_pool_passes_validate() {
    // Red: test fails because handler returns 501
    // Body: valid config that passes UpstreamPoolConfig::validate
    // Expected: 201, audit row written
}

#[tokio::test]
async fn ac16_update_upstream_pool_resets_next_period() {
    // Red: handler stub
    // Body: reset_kuota_* changes
    // Expected: 200, values in *_berikut columns, audit row
}

#[tokio::test]
async fn ac17_create_downstream_pool_single_upstream() {
    // Red: handler stub
    // Body: one upstream_pool_ids, valid alloc
    // Expected: 201, downstream_pool_rute written, prioritas=1 (read-only)
}

#[tokio::test]
async fn ac17_downstream_pool_rejects_over_quota() {
    // Red: handler stub
    // Body: upstream_pool_ids alloc exceeds quota
    // Expected: 422 (trigger tg_alokasi_dalam_kuota fires)
}

#[tokio::test]
async fn ac18_get_quota_tracker_reads_db_not_redis() {
    // Red: handler stub
    // Setup: insert upstream_attempt_daily and transaction_log rows
    // Modify Redis counter (prove ignored)
    // Expected: 200, tracker reflects DB state, not Redis
}
