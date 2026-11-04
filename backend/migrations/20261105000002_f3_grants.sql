-- F3 schema, part 2: grants for the blend objects and the new role pr_blend (SHAA-354).
-- Source: delta data contract F3 v0.3.2 BA-01..BA-04, section 7. Role pr_blend comes from backend/db/bootstrap.sql
-- (cluster-wide, not a migration): re-run the bootstrap before this migration.
-- Least privilege: nobody gets DELETE on a new table; pr_controller still cannot read blend.output_payload (A-01);
-- pr_admin_api cannot read blend; pr_blend never sees input_payload, metadata, transaction_log or NIK and cannot UPDATE request.

-- request.blend_id is a plain identifier. The existing roles read request through column lists made at F0, so it needs an explicit grant.
-- pr_worker and pr_controller need it for the N-1 predicate (blend_id IS NULL when a webhook is planned); pr_api for polling (BL-04).
GRANT SELECT (blend_id) ON request TO pr_api, pr_worker, pr_controller, pr_blend;

-- ================================================================ pr_api (intake, sync path, client polling and cancel)
GRANT SELECT ON endpoint_blend, endpoint_blend_versi, endpoint_blend_komponen TO pr_api;
GRANT SELECT, INSERT ON blend, blend_status_history TO pr_api;
-- sync outcome and the atomic cancel (BL-06); identity columns stay untouchable (trigger as well)
GRANT UPDATE (status, reject_reason, reject_http_status, failure_reason, anak_penentu_request_id, cancel_reason, cancelled_by,
  ringkasan_komponen, output_payload, completed_at, webhook_status) ON blend TO pr_api;
GRANT EXECUTE ON FUNCTION fn_catat_gerbang_blend(text, text, text) TO pr_api;

-- ================================================================ pr_blend (merge job, runs inside the controller process, own pool)
GRANT SELECT ON blend, endpoint_blend, endpoint_blend_versi, endpoint_blend_komponen TO pr_blend;
GRANT SELECT (blend_id, downstream_endpoint_id, status, failure_reason, cancel_reason, attempt_count, completed_at, output_payload) ON request TO pr_blend;
GRANT SELECT ON request_status_history TO pr_blend;
GRANT UPDATE (status, failure_reason, anak_penentu_request_id, cancel_reason, cancelled_by, ringkasan_komponen, output_payload, completed_at,
  webhook_status) ON blend TO pr_blend;
GRANT INSERT ON blend_status_history TO pr_blend;
GRANT SELECT (blend_id, seq) ON blend_status_history TO pr_blend;   -- next seq; the table has no payload column

-- ================================================================ pr_controller (purge only)
GRANT EXECUTE ON FUNCTION fn_purge_blend_payload(integer) TO pr_controller;

-- ================================================================ pr_webhook (blend webhook, same shape as for request)
GRANT SELECT (blend_id, client_id, mode, status, completed_at, webhook_status, webhook_attempt_count, webhook_not_before, output_payload, ringkasan_komponen)
  ON blend TO pr_webhook;
GRANT UPDATE (webhook_status, webhook_attempt_count, webhook_last_at, webhook_last_http_status, webhook_last_error, webhook_not_before) ON blend TO pr_webhook;
GRANT INSERT ON blend_webhook_log TO pr_webhook;

-- ================================================================ pr_admin_api (configuration and the marker)
GRANT SELECT, INSERT ON endpoint_blend_versi, endpoint_blend_komponen TO pr_admin_api;
GRANT SELECT, INSERT ON endpoint_blend TO pr_admin_api;
GRANT UPDATE (status, config_version, diubah_at) ON endpoint_blend TO pr_admin_api;
GRANT UPDATE (butuh_dasar_hukum_blend) ON provider TO pr_admin_api;
GRANT SELECT, INSERT ON provider_penanda_audit TO pr_admin_api;
