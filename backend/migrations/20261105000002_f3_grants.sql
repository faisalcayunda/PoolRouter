-- F3 schema, part 2: grants for the blend objects and the new role pr_blend (SHAA-354).
-- Source: delta data contract F3 v0.3.2 section 7 (BA-01..BA-04). Role pr_blend comes from backend/db/bootstrap.sql
-- (cluster-wide; re-run the bootstrap before this migration). Nobody gets DELETE on a new table (retention goes through
-- fn_blend_hapus_online). F0 column grants were generated from column lists, so request.blend_id needs explicit grants.

-- ================================================================ pr_api (BA-02)
GRANT SELECT ON endpoint_blend, endpoint_blend_versi, endpoint_blend_komponen TO pr_api;      -- provider is already table-readable
GRANT SELECT, INSERT ON blend, blend_status_history TO pr_api;
GRANT SELECT (blend_id) ON request TO pr_api;
-- sync outcome and BL-06 cancellation (terminal columns), BP-02 purge after the response
GRANT UPDATE (status, failure_reason, anak_penentu_request_id, cancel_reason, cancelled_by, ringkasan_komponen, output_payload,
  payload_purged_at, completed_at, webhook_status) ON blend TO pr_api;
GRANT EXECUTE ON FUNCTION fn_gerbang_blend_catat(boolean, text, text, text) TO pr_api;

-- ================================================================ pr_blend (BA-01): the merge job
-- request: only what BL-05 needs. request_id and cancelled_by are additions to the BA-01 list (the summary carries
-- request_id; the blend takes cancelled_by from the earliest cancelled child). No input_payload, metadata, nik, UPDATE.
GRANT SELECT ON blend, endpoint_blend, endpoint_blend_versi, endpoint_blend_komponen, request_status_history TO pr_blend;
GRANT SELECT (request_id, blend_id, downstream_endpoint_id, status, failure_reason, cancel_reason, cancelled_by, attempt_count,
  completed_at, output_payload) ON request TO pr_blend;
GRANT UPDATE (status, failure_reason, anak_penentu_request_id, cancel_reason, cancelled_by, ringkasan_komponen, output_payload,
  completed_at, webhook_status) ON blend TO pr_blend;
GRANT INSERT ON blend_status_history TO pr_blend;

-- ================================================================ pr_controller (BA-02): purge only, never reads output_payload
DO $$ BEGIN
  EXECUTE format('GRANT SELECT (%s) ON blend TO pr_controller', (
    SELECT string_agg(quote_ident(column_name), ', ' ORDER BY ordinal_position) FROM information_schema.columns
     WHERE table_schema = 'public' AND table_name = 'blend' AND column_name <> 'output_payload'));
END $$;
GRANT UPDATE (output_payload, payload_purged_at) ON blend TO pr_controller;
GRANT EXECUTE ON FUNCTION fn_blend_hapus_online(timestamptz) TO pr_controller;
GRANT EXECUTE ON FUNCTION fn_gerbang_blend_catat(boolean, text, text, text) TO pr_controller;

-- ================================================================ pr_webhook (BA-02)
GRANT SELECT ON blend TO pr_webhook;
GRANT UPDATE (webhook_status, webhook_attempt_count, webhook_last_at, webhook_last_http_status, webhook_last_error, webhook_not_before)
  ON blend TO pr_webhook;
GRANT SELECT, INSERT ON blend_webhook_log TO pr_webhook;

-- ================================================================ pr_admin_api (BA-02): configuration and the marker, no blend rows
GRANT SELECT, INSERT, UPDATE ON endpoint_blend TO pr_admin_api;
GRANT SELECT, INSERT ON endpoint_blend_versi, endpoint_blend_komponen TO pr_admin_api;
GRANT UPDATE (butuh_dasar_hukum_blend) ON provider TO pr_admin_api;

-- pr_worker: nothing (children are ordinary requests). Function execution stays closed to PUBLIC.
REVOKE ALL ON FUNCTION fn_ringkasan_komponen_valid(jsonb), tg_endpoint_blend_baru_nonaktif(), tg_blend_versi_min_dua_komponen(),
  tg_blend_transisi(), tg_blend_append_only(), tg_blend_punya_history(), tg_request_f3_guard(), tg_request_anak_blend(),
  tg_blend_anak_lengkap(), tg_blend_terminal_konsisten() FROM PUBLIC;
