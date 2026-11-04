-- F3 access matrix (SHAA-354): delta data contract F3 v0.3.2 BA-01..BA-04 and "role F0 tetap least-privilege untuk tabel baru".
-- Statements run as the role under test (SET LOCAL ROLE), inside transactions that are rolled back.
\set ON_ERROR_STOP on

-- Nobody but the owner holds DELETE, TRUNCATE or any DDL-like privilege on the new tables; PUBLIC holds nothing.
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.role_table_grants
   WHERE table_schema = 'public' AND grantee <> 'poolrouter_owner'
     AND table_name IN ('endpoint_blend', 'endpoint_blend_versi', 'endpoint_blend_komponen', 'blend', 'blend_status_history', 'blend_webhook_log',
                        'provider_penanda_audit', 'gerbang_blend_peristiwa')
     AND privilege_type IN ('DELETE', 'TRUNCATE', 'TRIGGER', 'REFERENCES')$q$) = 0, 'no role but the owner has DELETE / TRUNCATE / TRIGGER / REFERENCES on an F3 table');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.role_table_grants
   WHERE table_schema = 'public' AND grantee = 'PUBLIC' AND table_name IN ('endpoint_blend', 'endpoint_blend_versi', 'endpoint_blend_komponen', 'blend',
     'blend_status_history', 'blend_webhook_log', 'provider_penanda_audit', 'gerbang_blend_peristiwa')$q$) = 0, 'PUBLIC has no privilege on an F3 table');
-- DQ-66: nobody but the owner and the roles of BA-02 can read blend.output_payload
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.role_column_grants
   WHERE table_name = 'blend' AND column_name = 'output_payload' AND privilege_type = 'SELECT'
     AND grantee NOT IN ('poolrouter_owner', 'pr_blend', 'pr_webhook', 'pr_api')$q$) = 0, 'DQ-66: only owner, pr_blend, pr_webhook, pr_api can SELECT blend.output_payload');
SELECT t.ok(NOT has_column_privilege('pr_controller', 'blend', 'output_payload', 'SELECT') AND NOT has_table_privilege('pr_controller', 'blend', 'SELECT'),
  'A-01 / DQ-66: pr_controller cannot read blend.output_payload');
SELECT t.ok(NOT has_column_privilege('pr_blend', 'request', 'input_payload', 'SELECT') AND NOT has_column_privilege('pr_blend', 'request', 'metadata', 'SELECT')
        AND NOT has_table_privilege('pr_blend', 'transaction_log', 'SELECT') AND NOT has_column_privilege('pr_blend', 'transaction_log', 'nik', 'SELECT')
        AND NOT has_any_column_privilege('pr_blend', 'request', 'UPDATE') AND NOT has_any_column_privilege('pr_blend', 'request', 'INSERT'),
  'DQ-66 / BA-01: pr_blend has no input_payload, metadata, transaction_log or NIK, and no UPDATE or INSERT on request');
SELECT t.ok(NOT has_table_privilege('pr_admin_api', 'blend', 'SELECT') AND NOT has_any_column_privilege('pr_admin_api', 'blend', 'SELECT'),
  'BA-02: pr_admin_api cannot read blend');
SELECT t.ok(NOT has_any_column_privilege('pr_worker', 'blend', 'SELECT') AND NOT has_any_column_privilege('pr_worker', 'blend', 'UPDATE'),
  'BA-02: pr_worker gets nothing on blend');
SELECT t.ok(NOT has_column_privilege('pr_webhook', 'request', 'input_payload', 'SELECT') AND NOT has_table_privilege('pr_webhook', 'transaction_log', 'SELECT')
        AND NOT has_column_privilege('pr_webhook', 'blend', 'requested_at', 'UPDATE') AND NOT has_column_privilege('pr_webhook', 'blend', 'status', 'UPDATE'),
  'BA-02: pr_webhook has no input_payload, no transaction_log, and writes only webhook bookkeeping');
SELECT t.ok(NOT has_any_column_privilege('pr_job_balikan', 'blend', 'SELECT') AND NOT has_any_column_privilege('pr_restore', 'blend', 'SELECT')
        AND NOT has_any_column_privilege('pr_job_arsip', 'blend', 'UPDATE'),
  'the F1 job roles keep no write access to the F3 tables');
SELECT t.ok(NOT has_function_privilege('pr_worker', 'fn_catat_gerbang_blend(text, text, text)', 'EXECUTE') AND NOT has_function_privilege('pr_admin_api', 'fn_purge_blend_payload(integer)', 'EXECUTE')
        AND NOT has_function_privilege('PUBLIC', 'fn_purge_blend_payload(integer)', 'EXECUTE') AND NOT has_function_privilege('PUBLIC', 'fn_catat_gerbang_blend(text, text, text)', 'EXECUTE'),
  'F3 functions are executable only by the roles that need them (not PUBLIC)');
SELECT t.ok(has_function_privilege('pr_controller', 'fn_purge_blend_payload(integer)', 'EXECUTE') AND has_function_privilege('pr_api', 'fn_catat_gerbang_blend(text, text, text)', 'EXECUTE'),
  'the purge runs as pr_controller and the gate trail is written by the webserver process');
-- pr_blend can log in and nothing else is granted to it by accident
SELECT t.ok(t.hitung($q$SELECT count(*) FROM pg_roles WHERE rolname = 'pr_blend' AND rolcanlogin AND NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolbypassrls$q$) = 1,
  'pr_blend: login, no superuser / createdb / createrole / bypassrls');

-- ============ the merge job at work (BA-01)
BEGIN;
SELECT t.f3_siapkan();
SET LOCAL ROLE pr_blend;
SELECT t.jalan('SELECT blend_id, status, mode FROM blend', 'pr_blend reads blend');
SELECT t.jalan('SELECT blend_id, downstream_endpoint_id, status, failure_reason, cancel_reason, attempt_count, completed_at, output_payload FROM request', 'pr_blend reads the child columns of BA-01');
SELECT t.jalan('SELECT 1 FROM request_status_history', 'pr_blend reads request_status_history');
SELECT t.jalan('SELECT 1 FROM endpoint_blend_komponen', 'pr_blend reads the configuration');
SELECT t.gagal('SELECT input_payload FROM request', '42501', 'pr_blend: no input_payload');
SELECT t.gagal('SELECT metadata FROM request', '42501', 'pr_blend: no metadata');
SELECT t.gagal('SELECT 1 FROM transaction_log', '42501', 'pr_blend: no transaction_log');
SELECT t.gagal($$UPDATE request SET status = 'cancelled'$$, '42501', 'pr_blend: no UPDATE on request');
SELECT t.gagal('DELETE FROM blend', '42501', 'pr_blend: no DELETE on blend');
SELECT t.gagal($$UPDATE blend SET client_id = client_id$$, '42501', 'pr_blend: cannot touch identity columns of blend');
SELECT t.jalan($$UPDATE blend SET output_payload = NULL WHERE false$$, 'pr_blend: writes the blend result columns');
SELECT t.jalan($$INSERT INTO blend_status_history (blend_id, seq, status_dari, status_ke, aktor) SELECT blend_id, 2, 'berjalan', 'berjalan', 'controller' WHERE false$$, 'pr_blend: writes blend_status_history');
ROLLBACK;

BEGIN;
SELECT t.f3_siapkan();
SET LOCAL ROLE pr_controller;
SELECT t.gagal('SELECT output_payload FROM blend', '42501', 'pr_controller: no blend.output_payload');
SELECT t.gagal('SELECT 1 FROM blend', '42501', 'pr_controller: no table-level SELECT on blend');
SELECT t.jalan('SELECT blend_id FROM request', 'pr_controller reads request.blend_id (N-1 predicate)');
SELECT t.jalan('SELECT fn_purge_blend_payload()', 'pr_controller purges blend payloads through the function');
SELECT t.gagal($$UPDATE blend SET output_payload = NULL$$, '42501', 'pr_controller: no direct UPDATE on blend');
ROLLBACK;

BEGIN;
SELECT t.f3_siapkan();
SET LOCAL ROLE pr_admin_api;
SELECT t.jalan('SELECT 1 FROM endpoint_blend', 'pr_admin_api reads blend configuration');
SELECT t.jalan($$UPDATE endpoint_blend SET diubah_at = now() WHERE false$$, 'pr_admin_api updates blend configuration');
SELECT t.jalan('SELECT butuh_dasar_hukum_blend FROM provider', 'pr_admin_api reads the marker');
SELECT t.gagal('SELECT 1 FROM blend', '42501', 'pr_admin_api: no blend rows');
SELECT t.gagal($$UPDATE provider SET aktif = false$$, '42501', 'pr_admin_api: only the marker column of provider is writable here');
SELECT t.jalan($$UPDATE provider SET butuh_dasar_hukum_blend = butuh_dasar_hukum_blend WHERE false$$, 'pr_admin_api updates the marker (GB-03)');
SELECT t.jalan('SELECT 1 FROM provider_penanda_audit', 'pr_admin_api reads the marker audit');
SELECT t.gagal('DELETE FROM endpoint_blend', '42501', 'pr_admin_api: no DELETE on configuration');
SELECT t.gagal('SELECT fn_catat_gerbang_blend($$true$$, $$X$$, $$i$$)', '42501', 'pr_admin_api: cannot write the gate trail');
ROLLBACK;

BEGIN;
SELECT t.f3_siapkan();
SET LOCAL ROLE pr_api;
SELECT t.jalan('SELECT blend_id, output_payload FROM blend', 'pr_api reads blend (client polling)');
SELECT t.jalan('SELECT blend_id FROM request', 'pr_api reads request.blend_id (BL-04)');
SELECT t.jalan('SELECT butuh_dasar_hukum_blend FROM provider', 'pr_api reads the marker (G-5)');
SELECT t.gagal('SELECT input_payload FROM request', '42501', 'pr_api: still no input_payload');
SELECT t.gagal('DELETE FROM blend', '42501', 'pr_api: no DELETE on blend');
SELECT t.gagal($$UPDATE blend SET client_id = client_id$$, '42501', 'pr_api: cannot touch identity columns of blend');
SELECT t.gagal($$UPDATE endpoint_blend SET status = 'aktif'$$, '42501', 'pr_api: cannot change blend configuration');
SELECT t.gagal($$UPDATE provider SET butuh_dasar_hukum_blend = true$$, '42501', 'pr_api: cannot change the marker');
ROLLBACK;

BEGIN;
SELECT t.f3_siapkan();
SET LOCAL ROLE pr_webhook;
SELECT t.jalan('SELECT blend_id, mode, status, completed_at, webhook_status, webhook_attempt_count, output_payload FROM blend', 'pr_webhook reads what a blend webhook needs');
SELECT t.gagal('SELECT source_ip FROM blend', '42501', 'pr_webhook: no other blend columns');
SELECT t.jalan($$UPDATE blend SET webhook_status = webhook_status WHERE false$$, 'pr_webhook updates webhook bookkeeping');
SELECT t.jalan($$INSERT INTO blend_webhook_log SELECT gen_random_uuid(), 'b3000000-0000-7000-8000-000000000001', '10000000-0000-7000-8000-000000000001', 1, 'done', now(), NULL, 200, 'terkirim', NULL, 5, 1 WHERE false$$,
  'pr_webhook writes blend_webhook_log');
SELECT t.gagal('SELECT 1 FROM blend_webhook_log', '42501', 'pr_webhook: write-only on blend_webhook_log');
ROLLBACK;
