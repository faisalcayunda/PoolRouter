-- Access matrix (contract 8.1, 8.1a, 8.1b; ADR-0001 7.2): U-23, U-55, A-01, DQ-24, DQ-37 (f).
\set ON_ERROR_STOP on

-- Data to read: one done request with a final log (NIK, payload), one terminal async request, one waiting request.
BEGIN;
SELECT t.buat_request('{"request_id":"a0000000-0000-7000-8000-000000000001","status":"in_flight","attempt_count":1,
  "upstream_pool_id":"30000000-0000-7000-8000-000000000001","input_payload":{"NIK":"0000000000001234"},"metadata":{"status_penerima_manfaat":"1"}}');
INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
  provider_config_version, endpoint_config_version, nik, requested_at, status_akses, log_mode, worker_id, response_hash)
VALUES ('b0000000-0000-7000-8000-000000000001', 'a0000000-0000-7000-8000-000000000001', 1, '20000000-0000-7000-8000-000000000001',
  '30000000-0000-7000-8000-000000000001', '10000000-0000-7000-8000-000000000001', '50000000-0000-7000-8000-000000000001', 1, 1,
  '0000000000001234', '2026-11-03 01:00+00', 'berhasil', 'metadata_only', 'w1', repeat('a', 64));
UPDATE request SET status = 'done', completed_at = now(), output_payload = '{"NAMA_LGKP":{"status":"sesuai","skor":90}}',
  final_kode_transaksi = 'b0000000-0000-7000-8000-000000000001', sumber_hasil = 'provider', webhook_status = 'menunggu'
  WHERE request_id = 'a0000000-0000-7000-8000-000000000001';
INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) VALUES ('a0000000-0000-7000-8000-000000000001', 2, 'queued', 'done', 'worker');
-- history above skips in_flight: fix order for realism
COMMIT;

-- ============ pr_controller: no NIK, no payload (ADR-0001 7.2 mandatory test)
SET ROLE pr_controller;
SELECT t.gagal('SELECT nik FROM transaction_log', '42501', 'pr_controller: SELECT nik denied');
SELECT t.gagal('SELECT request_payload FROM transaction_log', '42501', 'pr_controller: SELECT request_payload denied');
SELECT t.jalan($$SELECT fn_ditahan_data_balikan('a0000000-0000-7000-8000-000000000001')$$, 'pr_controller: hold function executable without table SELECT');
SELECT t.gagal('SELECT response_payload FROM transaction_log', '42501', 'pr_controller: SELECT response_payload denied');
SELECT t.gagal('SELECT * FROM transaction_log', '42501', 'pr_controller: SELECT * on transaction_log denied');
SELECT t.gagal('SELECT output_payload FROM request', '42501', 'pr_controller: SELECT output_payload denied');
SELECT t.gagal('SELECT input_payload FROM request', '42501', 'pr_controller: SELECT input_payload denied');
SELECT t.gagal('SELECT metadata FROM request', '42501', 'pr_controller: SELECT metadata denied');
SELECT t.jalan('SELECT status, dispatch_key, not_before, attempt_count, ttl_expires_at FROM request', 'pr_controller: status and dispatch columns readable');
SELECT t.jalan('SELECT kode_transaksi, status_akses, requested_at FROM transaction_log', 'pr_controller: log status columns readable');
SELECT t.gagal('SELECT secret_ciphertext FROM provider_secret', '42501', 'pr_controller: provider secret denied');
SELECT t.gagal('SELECT webhook_url FROM client_webhook', '42501', 'pr_controller: client_webhook.webhook_url denied');
SELECT t.gagal('SELECT secret_ciphertext FROM client_webhook', '42501', 'pr_controller: client_webhook secret denied');
SELECT t.jalan('SELECT client_id, aktif FROM client_webhook', 'pr_controller: client_webhook (client_id, aktif) readable');
SELECT t.gagal('SELECT * FROM audit_akses', '42501', 'pr_controller: audit_akses denied');
SELECT t.gagal('DELETE FROM request', '42501', 'pr_controller: no DELETE');
SELECT t.gagal($$UPDATE client SET aktif = false$$, '42501', 'pr_controller: cannot change entity status (8.1b)');
SELECT t.gagal($$UPDATE downstream_endpoint SET aktif = false$$, '42501', 'pr_controller: cannot change endpoint status');
SELECT t.gagal('SELECT kunci_hash FROM client_kredensial', '42501', 'pr_controller: credential hash denied (8.1b)');
SELECT t.gagal('SELECT kedaluwarsa_at FROM client_kredensial', '42501', 'pr_controller: kedaluwarsa_at denied (8.1b)');
SELECT t.gagal('SELECT dicabut_at FROM client_kredensial', '42501', 'pr_controller: dicabut_at denied (8.1b)');
SELECT t.jalan('SELECT credential_id, client_id, status FROM client_kredensial', 'pr_controller: credential status columns readable');
SELECT t.gagal('SELECT nama FROM client', '42501', 'pr_controller: other client columns denied');
SELECT t.gagal($$UPDATE request SET payload_purged_at = now()$$, '42501', 'pr_controller: payload purge only through the function');
SELECT t.gagal($$UPDATE transaction_log SET status_akses = 'tidak_diketahui'$$, '42501', 'pr_controller: closes logs only through fn_tutup_log_tidak_diketahui');
SELECT t.jalan('SELECT count(*) FROM upstream_attempt_daily', 'pr_controller: upstream_attempt_daily readable');
SELECT t.jalan($$INSERT INTO upstream_attempt_daily VALUES ('30000000-0000-7000-8000-000000000001', '2026-11-01 00:00+00', '2026-11-02 00:00+00', false, '2026-11-01', 3, now())$$,
  'pr_controller: upsert upstream_attempt_daily');
RESET ROLE;

-- ============ pr_worker: reads input_payload, no webhook config, no hash columns (8.1b)
SET ROLE pr_worker;
SELECT t.jalan('SELECT input_payload, metadata FROM request', 'pr_worker: reads input_payload of requests');
SELECT t.gagal('SELECT webhook_url FROM client_webhook', '42501', 'pr_worker: client_webhook.webhook_url denied (U-23)');
SELECT t.gagal('SELECT secret_versi FROM client_webhook', '42501', 'pr_worker: client_webhook.secret_versi denied');
SELECT t.gagal('SELECT kek_id FROM client_webhook', '42501', 'pr_worker: client_webhook.kek_id denied');
SELECT t.gagal('SELECT secret_nonce FROM client_webhook', '42501', 'pr_worker: client_webhook secret denied');
SELECT t.jalan('SELECT secret_ciphertext FROM provider_secret', 'pr_worker: reads provider secret ciphertext');
SELECT t.gagal('SELECT kunci_hash FROM client_kredensial', '42501', 'pr_worker: credential hash denied (8.1b)');
SELECT t.jalan('SELECT credential_id, client_id, status FROM client_kredensial', 'pr_worker: credential status readable (8.1b)');
SELECT t.jalan('SELECT client_id, aktif FROM client', 'pr_worker: client (id, aktif) readable (8.1b)');
SELECT t.gagal('SELECT nama FROM client', '42501', 'pr_worker: other client columns denied');
SELECT t.gagal($$UPDATE client SET aktif = false$$, '42501', 'pr_worker: cannot change client status');
SELECT t.gagal($$UPDATE client_kredensial SET status = 'dicabut'$$, '42501', 'pr_worker: cannot change credential status');
SELECT t.gagal($$UPDATE upstream_pool SET aktif = false$$, '42501', 'pr_worker: cannot change pool status');
SELECT t.gagal('SELECT output_payload FROM request', '42501', 'pr_worker: output_payload not readable');
SELECT t.gagal('SELECT * FROM audit_akses', '42501', 'pr_worker: audit_akses denied');
SELECT t.gagal('DELETE FROM transaction_log', '42501', 'pr_worker: no DELETE');
SELECT t.jalan('SELECT nik FROM transaction_log', 'pr_worker: reads nik (writes the log)');
RESET ROLE;

-- ============ pr_api
SET ROLE pr_api;
SELECT t.gagal('SELECT webhook_url FROM client_webhook', '42501', 'pr_api: client_webhook.webhook_url denied (U-23)');
SELECT t.gagal('SELECT secret_ciphertext FROM client_webhook', '42501', 'pr_api: client_webhook secret denied');
SELECT t.jalan('SELECT client_id, aktif FROM client_webhook', 'pr_api: client_webhook (client_id, aktif) readable');
SELECT t.jalan('SELECT secret_ciphertext FROM provider_secret', 'pr_api: reads provider secret ciphertext');
SELECT t.jalan('SELECT kunci_hash, status FROM client_kredensial', 'pr_api: authenticates against credential hash');
SELECT t.gagal('SELECT * FROM audit_akses', '42501', 'pr_api: audit_akses denied');
SELECT t.gagal('DELETE FROM request', '42501', 'pr_api: no DELETE');
SELECT t.gagal('SELECT input_payload FROM request', '42501', 'pr_api: input_payload not readable');
SELECT t.jalan('SELECT output_payload FROM request', 'pr_api: output_payload readable (polling)');
SELECT t.jalan('SELECT nik FROM transaction_log', 'pr_api: NIK readable for sync transaction path');
SELECT t.gagal($$UPDATE client SET aktif = false$$, '42501', 'pr_api: cannot change client status');
RESET ROLE;

-- ============ pr_webhook (U-23): positive control and denials
SET ROLE pr_webhook;
SELECT t.jalan('SELECT client_id, webhook_url, aktif, secret_versi, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id FROM client_webhook',
  'pr_webhook: eight W-02 columns readable (positive control)');
SELECT t.gagal('SELECT diubah_at FROM client_webhook', '42501', 'pr_webhook: client_webhook.diubah_at denied');
SELECT t.gagal('SELECT * FROM client_webhook', '42501', 'pr_webhook: SELECT * on client_webhook denied');
SELECT t.jalan('SELECT domain, aktif FROM webhook_domain_allowlist', 'pr_webhook: allowlist readable');
SELECT t.gagal('SELECT secret_ciphertext FROM provider_secret', '42501', 'pr_webhook: provider secret denied');
SELECT t.gagal('SELECT nama FROM client', '42501', 'pr_webhook: client table denied');
SELECT t.gagal('SELECT nik FROM transaction_log', '42501', 'pr_webhook: SELECT nik denied');
SELECT t.gagal('SELECT 1 FROM transaction_log', '42501', 'pr_webhook: transaction_log denied');
SELECT t.gagal('SELECT input_payload FROM request', '42501', 'pr_webhook: input_payload denied');
SELECT t.gagal('SELECT metadata FROM request', '42501', 'pr_webhook: metadata denied');
SELECT t.gagal('SELECT output_payload FROM request', '42501', 'pr_webhook: output_payload only through the terminal view');
SELECT t.ok(t.hitung('SELECT count(*) FROM v_request_webhook WHERE status NOT IN (''done'',''failed'',''expired'',''cancelled'')') = 0,
  'pr_webhook: view exposes terminal requests only');
SELECT t.ok(t.hitung('SELECT count(*) FROM v_request_webhook WHERE output_payload IS NOT NULL') >= 1, 'pr_webhook: reads output_payload of terminal request');
SELECT t.jalan('SELECT request_id, webhook_status, webhook_attempt_count FROM request', 'pr_webhook: CAS columns readable');
SELECT t.jalan($$UPDATE request SET webhook_attempt_count = 1 WHERE webhook_status = 'menunggu' AND webhook_attempt_count = 0$$, 'pr_webhook: webhook claim');
SELECT t.gagal($$UPDATE request SET status = 'failed'$$, '42501', 'pr_webhook: cannot change status');
SELECT t.gagal($$UPDATE client_webhook SET aktif = false$$, '42501', 'pr_webhook: no UPDATE on client_webhook');
SELECT t.gagal($$INSERT INTO webhook_domain_allowlist VALUES ('x.example', true, now(), '00000000-0000-7000-8000-000000000001')$$, '42501', 'pr_webhook: allowlist read-only');
SELECT t.gagal('DELETE FROM webhook_log', '42501', 'pr_webhook: no DELETE');
SELECT t.gagal('SELECT * FROM audit_akses', '42501', 'pr_webhook: audit_akses denied');
SELECT t.gagal('SELECT fn_purge_p02(gen_random_uuid())', '42501', 'pr_webhook: no EXECUTE on purge function');
RESET ROLE;

-- ============ pr_admin_api
SET ROLE pr_admin_api;
SELECT t.gagal('SELECT nik FROM transaction_log', '42501', 'pr_admin_api: SELECT nik denied');
SELECT t.gagal('SELECT input_payload FROM request', '42501', 'pr_admin_api: payload denied');
SELECT t.gagal('SELECT output_payload FROM request', '42501', 'pr_admin_api: output payload denied');
SELECT t.gagal('SELECT * FROM transaction_log', '42501', 'pr_admin_api: transaction_log denied');
SELECT t.gagal('SELECT secret_ciphertext FROM client_webhook', '42501', 'pr_admin_api: webhook secret write-only (W-03)');
SELECT t.gagal('SELECT secret_dek_terbungkus FROM client_webhook', '42501', 'pr_admin_api: webhook DEK write-only');
SELECT t.gagal('SELECT secret_ciphertext FROM provider_secret', '42501', 'pr_admin_api: provider secret write-only');
SELECT t.gagal('SELECT kunci_hash FROM client_kredensial', '42501', 'pr_admin_api: credential hash write-only');
SELECT t.jalan('SELECT webhook_url, secret_versi, kek_id FROM client_webhook', 'pr_admin_api: non-secret webhook columns readable');
SELECT t.jalan($$UPDATE client_webhook SET secret_ciphertext = '\x99', secret_versi = secret_versi WHERE false$$, 'pr_admin_api: webhook secret writable');
SELECT t.gagal('SELECT * FROM audit_akses', '42501', 'pr_admin_api: audit_akses unreadable except via fn_lihat_audit');
SELECT t.gagal('UPDATE audit_akses SET peran = peran', '42501', 'pr_admin_api: audit_akses not updatable');
SELECT t.gagal('DELETE FROM audit_akses', '42501', 'pr_admin_api: audit_akses not deletable');
SELECT t.gagal('DELETE FROM audit_konfigurasi', '42501', 'pr_admin_api: audit_konfigurasi not deletable');
SELECT t.gagal('DELETE FROM request', '42501', 'pr_admin_api: request not deletable');
-- masked views
SELECT t.ok(t.hitung('SELECT count(*) FROM v_transaction_log_tersamar WHERE nik_tersamar = ''************1234''') = 1, 'masked view shows only the last 4 digits');
SELECT t.ok(t.hitung($$SELECT count(*) FROM v_transaction_log_tersamar l WHERE l::text ~ '[0-9]{16}'$$) = 0, 'masked log view has no 16-digit value anywhere');
SELECT t.ok(t.hitung($$SELECT count(*) FROM v_request_tersamar l WHERE l::text ~ '[0-9]{16}'$$) = 0, 'masked request view has no 16-digit value anywhere');
SELECT t.gagal($$SELECT nik FROM v_transaction_log_tersamar$$, '42703', 'masked view has no nik column');
-- U-54c: direct UPDATE of the credential status, outside the function
SELECT t.gagal($$UPDATE client_kredensial SET status = 'aktif'$$, '42501', 'U-54c: direct UPDATE of credential status denied');
SELECT t.gagal($$UPDATE client_kredensial SET dicabut_at = NULL$$, '42501', 'U-54c: direct UPDATE of dicabut_at denied');
SELECT t.gagal($$UPDATE client_kredensial SET dicabut_oleh = NULL$$, '42501', 'U-54c: direct UPDATE of dicabut_oleh denied');
SELECT t.gagal($$UPDATE client SET aktif = false$$, '42501', 'direct status flip denied: use fn_nonaktifkan_entitas');
SELECT t.gagal($$UPDATE provider SET aktif = false$$, '42501', 'direct provider status flip denied');
RESET ROLE;

-- ============ catalog checks: DQ-24 (a) and (d), U-55
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.column_privileges
   WHERE table_name = 'client_webhook' AND column_name IN ('secret_ciphertext', 'secret_dek_terbungkus', 'secret_nonce')
     AND privilege_type = 'SELECT' AND grantee NOT IN ('poolrouter_owner', 'pr_webhook', 'postgres')$q$) = 0, 'DQ-24 (a): only owner and pr_webhook can select webhook secrets');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.table_privileges WHERE grantee = 'pr_webhook' AND table_name IN ('provider_secret', 'client')$q$) = 0
  AND t.hitung($q$ SELECT count(*) FROM information_schema.column_privileges WHERE grantee = 'pr_webhook' AND table_name IN ('provider_secret', 'client')$q$) = 0,
  'DQ-24 (a): pr_webhook holds no privilege on provider secrets or client');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.column_privileges
   WHERE grantee IN ('pr_worker', 'pr_controller') AND table_name = 'client_kredensial' AND column_name NOT IN ('credential_id', 'client_id', 'status')$q$) = 0,
  'DQ-24 (d): worker/controller see only id, client_id, status of credentials');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.column_privileges
   WHERE grantee IN ('pr_worker', 'pr_controller') AND table_name = 'client' AND column_name NOT IN ('client_id', 'aktif')$q$) = 0,
  'DQ-24 (d): worker/controller see only id and aktif of client');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.column_privileges
   WHERE grantee IN ('pr_worker', 'pr_controller') AND privilege_type = 'UPDATE'
     AND ((table_name IN ('client', 'downstream_endpoint', 'downstream_pool', 'upstream_pool', 'provider') AND column_name = 'aktif')
          OR (table_name = 'client_kredensial'))$q$) = 0,
  'DQ-24 (d): worker/controller cannot UPDATE any entity status column');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.table_privileges
   WHERE grantee IN ('pr_worker', 'pr_controller') AND table_name IN ('client', 'client_kredensial') AND privilege_type <> 'SELECT'$q$) = 0,
  'U-55: no write privilege on client / credential tables');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.role_table_grants
   WHERE grantee IN ('pr_api','pr_admin_api','pr_controller','pr_worker','pr_webhook') AND privilege_type = 'DELETE'
     AND table_name NOT IN ('downstream_pool_rute', 'client_endpoint', 'endpoint_metadata_field', 'admin_session')$q$) = 0,
  'no application role can DELETE outside the allowed configuration tables');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.role_table_grants
   WHERE grantee = 'pr_controller' AND table_name = 'transaction_log' AND privilege_type IN ('SELECT','UPDATE') $q$) = 0,
  'pr_controller has no whole-table grant on transaction_log (column grants only)');

-- DQ-37 (f) on the test data
SELECT t.ok(t.hitung($q$ SELECT count(*) FROM client_kredensial WHERE (status = 'aktif' AND dicabut_at IS NOT NULL) OR (status = 'dicabut' AND (dicabut_at IS NULL OR dicabut_oleh IS NULL))$q$) = 0,
  'DQ-37 (f): no active credential with dicabut_at, no revoked credential without stamps');
