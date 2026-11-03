-- request constraints, the NULL list for unparseable bodies, transitions, T-04, log rules, counters.
\set ON_ERROR_STOP on

-- ---- NULL list (AC SHAA-193): downstream_pool_id / prioritas / prioritas_efektif / ttl_expires_at only on rejected validasi_gagal 400
BEGIN;
SELECT t.jalan($$SELECT t.buat_request('{"status":"rejected","reject_reason":"validasi_gagal","reject_http_status":400,
  "completed_at":"2026-11-03T00:00:00Z","downstream_pool_id":null,"prioritas":null,"prioritas_efektif":null,"ttl_expires_at":null}')$$,
  'unparseable body: pool, priority and ttl may be NULL on rejected/validasi_gagal/400');
SAVEPOINT s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"rejected","reject_reason":"validasi_gagal","reject_http_status":422,
  "completed_at":"2026-11-03T00:00:00Z","downstream_pool_id":null,"prioritas":null,"prioritas_efektif":null,"ttl_expires_at":null}')$$,
  '23514', 'NULL pool refused for validasi_gagal 422');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"rejected","reject_reason":"kuota_habis","reject_http_status":429,
  "completed_at":"2026-11-03T00:00:00Z","downstream_pool_id":null,"prioritas":null,"prioritas_efektif":null,"ttl_expires_at":null}')$$,
  '23514', 'NULL pool refused for kuota_habis');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"downstream_pool_id":null,"prioritas":null,"prioritas_efektif":null}')$$,
  '23514', 'NULL pool refused for a queued request');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"rejected","reject_reason":"validasi_gagal","reject_http_status":400,
  "completed_at":"2026-11-03T00:00:00Z","downstream_pool_id":null,"prioritas":null,"prioritas_efektif":null,"ttl_expires_at":null,
  "metadata":{"a":1}}')$$, '23514', 'NULL list requires metadata = {}');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"rejected","reject_reason":"validasi_gagal","reject_http_status":400,
  "completed_at":"2026-11-03T00:00:00Z","downstream_pool_id":null,"prioritas":null,"prioritas_efektif":null,"ttl_expires_at":null,
  "input_payload":{"NIK":"0000000000000001"}}')$$, '23514', 'rejected rows never keep input_payload (P-01)');
ROLLBACK TO s;
-- columns derived from auth / path / config stay NOT NULL even on that row
SELECT t.gagal($$SELECT t.buat_request('{"status":"rejected","reject_reason":"validasi_gagal","reject_http_status":400,
  "completed_at":"2026-11-03T00:00:00Z","credential_id":null}')$$, '23502', 'credential_id stays NOT NULL');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"rejected","reject_reason":"validasi_gagal","reject_http_status":400,
  "completed_at":"2026-11-03T00:00:00Z","received_at":null}')$$, '23502', 'received_at stays NOT NULL');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"rejected","reject_reason":"validasi_gagal","reject_http_status":400,
  "completed_at":"2026-11-03T00:00:00Z","provider_config_version":null}')$$, '23502', 'config version stays NOT NULL');
ROLLBACK TO s;
-- pair must be consistent: pool NULL implies priorities NULL
SELECT t.gagal($$SELECT t.buat_request('{"status":"rejected","reject_reason":"validasi_gagal","reject_http_status":400,
  "completed_at":"2026-11-03T00:00:00Z","downstream_pool_id":null}')$$, '23514', 'pool and priorities go NULL together');
ROLLBACK TO s;
ROLLBACK;

-- ---- reason / state consistency (DQ-08), reject code pairs, attempts (DQ-07)
BEGIN;
SAVEPOINT s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"rejected","completed_at":"2026-11-03T00:00:00Z"}')$$, '23514', 'rejected needs reject_reason');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"rejected","reject_reason":"kuota_habis","reject_http_status":403,"completed_at":"2026-11-03T00:00:00Z"}')$$,
  '23514', 'kuota_habis only with 429');
ROLLBACK TO s;
SELECT t.jalan($$SELECT t.buat_request('{"status":"rejected","reject_reason":"objek_nonaktif","reject_http_status":503,"completed_at":"2026-11-03T00:00:00Z"}')$$,
  'objek_nonaktif 503 accepted');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"reject_reason":"kuota_habis"}')$$, '23514', 'queued cannot carry reject_reason');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"mode":"sync","status":"queued"}')$$, '23514', 'sync is never queued');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"mode":"sync","ttl_expires_at":"2026-11-04T00:00:00Z","status":"in_flight","attempt_count":1,"dispatch_key":null}')$$,
  '23514', 'sync has no ttl');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"ttl_expires_at":null}')$$, '23514', 'async needs ttl_expires_at');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"cancelled","cancel_reason":"client_nonaktif","cancelled_by":"sistem","attempt_count":1,
  "completed_at":"2026-11-03T00:00:00Z","webhook_status":"menunggu"}')$$, '23514', 'cancelled needs attempt_count = 0 (T-02a)');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"cancelled","cancel_reason":"dibatalkan_client","cancelled_by":"sistem",
  "completed_at":"2026-11-03T00:00:00Z","webhook_status":"menunggu"}')$$, '23514', 'dibatalkan_client requires cancelled_by = client (C-02)');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"cancelled","cancel_reason":"client_nonaktif","cancelled_by":"client",
  "completed_at":"2026-11-03T00:00:00Z","webhook_status":"menunggu"}')$$, '23514', 'K-20 reason cannot be cancelled_by client');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"failed","failure_reason":"retry_habis","attempt_count":0,"completed_at":"2026-11-03T00:00:00Z","webhook_status":"menunggu"}')$$,
  '23514', 'failed needs attempt_count >= 1 (DQ-07)');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"done","attempt_count":1,"completed_at":"2026-11-03T00:00:00Z","webhook_status":"menunggu"}')$$,
  '23514', 'done needs output_payload (or a purge stamp)');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"queued","output_payload":{"a":1}}')$$, '23514', 'output_payload only for done');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"rejected","reject_reason":"kuota_habis","reject_http_status":429,"completed_at":"2026-11-03T00:00:00Z",
  "idempotency_key":"k1","idempotency_fingerprint":"0000000000000000000000000000000000000000000000000000000000000000"}')$$, '23514', 'rejected keeps no idempotency key');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"idempotency_key":"k1"}')$$, '23514', 'idempotency key needs fingerprint');
ROLLBACK TO s;
SELECT t.gagal($$SELECT t.buat_request('{"status":"expired","attempt_count":0,"completed_at":"2026-11-03T00:00:00Z"}')$$, '23514', 'async terminal needs webhook_status');
ROLLBACK TO s;
ROLLBACK;

-- ---- idempotency uniqueness per client
BEGIN;
SELECT t.buat_request('{"idempotency_key":"abc","idempotency_fingerprint":"1111111111111111111111111111111111111111111111111111111111111111"}');
SAVEPOINT s;
SELECT t.gagal($$SELECT t.buat_request('{"idempotency_key":"abc","idempotency_fingerprint":"1111111111111111111111111111111111111111111111111111111111111111"}')$$,
  '23505', 'idempotency key unique per client');
ROLLBACK TO s;
SELECT t.jalan($$SELECT t.buat_request('{"client_id":"10000000-0000-7000-8000-000000000002","credential_id":"11000000-0000-7000-8000-000000000003",
  "idempotency_key":"abc","idempotency_fingerprint":"1111111111111111111111111111111111111111111111111111111111111111"}')$$, 'same key allowed for another client');
ROLLBACK;

-- ---- transitions (3.2 diagram), terminal immutability (T-05), history (T-04)
BEGIN;
CREATE TEMP TABLE _r AS SELECT t.buat_request('{}') AS id;
SAVEPOINT s;
SELECT t.gagal($$UPDATE request SET status = 'done', attempt_count = 1, output_payload = '{}', completed_at = now(), webhook_status = 'menunggu',
  final_kode_transaksi = gen_random_uuid() WHERE request_id = (SELECT id FROM _r)$$, '23514', 'queued -> done is not a legal transition');
ROLLBACK TO s;
UPDATE request SET status = 'dispatched', dispatched_at = now() WHERE request_id = (SELECT id FROM _r);
INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) SELECT id, 2, 'queued', 'dispatched', 'controller' FROM _r;
UPDATE request SET status = 'cancelled', cancel_reason = 'dibatalkan_client', cancelled_by = 'client', completed_at = now(), webhook_status = 'tidak_dikirim'
  WHERE request_id = (SELECT id FROM _r);
INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor, alasan) SELECT id, 3, 'dispatched', 'cancelled', 'client', 'dibatalkan_client' FROM _r;
SELECT t.gagal($$UPDATE request SET status = 'queued', cancel_reason = NULL, cancelled_by = NULL, completed_at = NULL, webhook_status = NULL
  WHERE request_id = (SELECT id FROM _r)$$, '23514', 'terminal status never changes (T-05)');
COMMIT;

-- T-04: a status without a matching history row fails when the deferred constraint fires
DO $x$
BEGIN
  BEGIN
    INSERT INTO request SELECT (jsonb_populate_record(NULL::request, jsonb_build_object(
      'request_id', gen_random_uuid(), 'client_id', '10000000-0000-7000-8000-000000000001',
      'credential_id', '11000000-0000-7000-8000-000000000001', 'downstream_endpoint_id', '50000000-0000-7000-8000-000000000001',
      'downstream_pool_id', '40000000-0000-7000-8000-000000000001', 'provider_id', '20000000-0000-7000-8000-000000000001',
      'provider_config_version', 1, 'endpoint_config_version', 1, 'mode', 'async', 'prioritas', 1, 'prioritas_efektif', 1, 'status', 'queued',
      'metadata', '{}'::jsonb, 'received_at', now(), 'ttl_expires_at', now() + interval '1 day', 'attempt_count', 0, 'webhook_attempt_count', 0))).*;
    SET CONSTRAINTS request_history_wajib IMMEDIATE;
    RAISE EXCEPTION 'FAIL: request without history row accepted (T-04)';
  EXCEPTION WHEN integrity_constraint_violation THEN
    RAISE NOTICE 'ok - request without history row is rejected (T-04)';
  END;
END $x$;

-- ---- transaction_log rules (R-03, R-05, R-07, R-08, nik requirement)
BEGIN;
CREATE TEMP TABLE _q AS SELECT t.buat_request('{"status":"in_flight","attempt_count":1,"dispatched_at":"2026-11-03T00:00:00Z","upstream_pool_id":"30000000-0000-7000-8000-000000000001"}') AS id;
CREATE TEMP TABLE _l AS SELECT gen_random_uuid() AS k;
INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
  provider_config_version, endpoint_config_version, nik, requested_at, log_mode, worker_id)
  SELECT k, id, 1, '20000000-0000-7000-8000-000000000001', '30000000-0000-7000-8000-000000000001', '10000000-0000-7000-8000-000000000001',
         '50000000-0000-7000-8000-000000000001', 1, 1, '0000000000000001', now(), 'metadata_only', 'w1' FROM _l, _q;
SAVEPOINT s;
SELECT t.gagal($$INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
  provider_config_version, endpoint_config_version, nik, requested_at, log_mode, worker_id)
  SELECT gen_random_uuid(), id, 1, '20000000-0000-7000-8000-000000000001', '30000000-0000-7000-8000-000000000001', '10000000-0000-7000-8000-000000000001',
         '50000000-0000-7000-8000-000000000001', 1, 1, '0000000000000002', now(), 'metadata_only', 'w1' FROM _q$$, '23505', '(request_id, attempt_no) unique');
ROLLBACK TO s;
SELECT t.gagal($$INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
  provider_config_version, endpoint_config_version, nik, requested_at, log_mode, worker_id)
  SELECT gen_random_uuid(), id, 2, '20000000-0000-7000-8000-000000000001', '30000000-0000-7000-8000-000000000001', '10000000-0000-7000-8000-000000000001',
         '50000000-0000-7000-8000-000000000001', 1, 1, NULL, now(), 'metadata_only', 'w1' FROM _q$$, '23502', 'nik required for a provider with wajib_log_nik');
ROLLBACK TO s;
SELECT t.gagal($$UPDATE transaction_log SET status_akses = 'berhasil', error_class = 'timeout' WHERE kode_transaksi = (SELECT k FROM _l)$$, '23514',
  'berhasil cannot carry an error_class');
ROLLBACK TO s;
SELECT t.gagal($$UPDATE transaction_log SET status_akses = 'gagal_jaringan', error_class = 'lainnya' WHERE kode_transaksi = (SELECT k FROM _l)$$, '23514',
  'lainnya with gagal_jaringan is illegal (DQ-31 e)');
ROLLBACK TO s;
SELECT t.gagal($$UPDATE transaction_log SET status_akses = 'gagal_jaringan', error_class = 'tls', responded_at = now() WHERE kode_transaksi = (SELECT k FROM _l)$$, '23514',
  'tls row has no responded_at (R-07)');
ROLLBACK TO s;
SELECT t.gagal($$UPDATE transaction_log SET request_payload = '{"a":1}' WHERE kode_transaksi = (SELECT k FROM _l)$$, '23514', 'metadata_only keeps no payload (R-05)');
ROLLBACK TO s;
SELECT t.gagal($$UPDATE transaction_log SET nik = '0000000000000009' WHERE kode_transaksi = (SELECT k FROM _l)$$, '23000', 'identity columns frozen while pending');
ROLLBACK TO s;
UPDATE transaction_log SET status_akses = 'gagal_jaringan', error_class = 'tls', duration_ms = 5 WHERE kode_transaksi = (SELECT k FROM _l);
SELECT t.gagal($$UPDATE transaction_log SET duration_ms = 6 WHERE kode_transaksi = (SELECT k FROM _l)$$, '23000', 'complete row is immutable (R-03)');
ROLLBACK TO s;
SELECT t.jalan($$UPDATE transaction_log SET archived_at = now() WHERE kode_transaksi = (SELECT k FROM _l)$$, 'archived_at may still be set');
SELECT t.gagal($$DELETE FROM transaction_log WHERE kode_transaksi = (SELECT k FROM _l)$$, '23000', 'transaction_log rows are not deleted');
ROLLBACK;

-- ---- upstream_attempt_daily (KP-06)
BEGIN;
INSERT INTO upstream_attempt_daily VALUES ('30000000-0000-7000-8000-000000000001', '2026-11-03 17:00+00', '2026-11-04 00:00+00', true, '2026-11-04', 5, now());
SAVEPOINT s;
SELECT t.jalan($$INSERT INTO upstream_attempt_daily VALUES ('30000000-0000-7000-8000-000000000001', '2026-11-04 00:00+00', '2026-11-04 17:00+00', true, '2026-11-04', 1, now()),
  ('30000000-0000-7000-8000-000000000001', '2026-11-04 17:00+00', '2026-11-05 17:00+00', false, '2026-11-04', 1, now())$$,
  'two consecutive periods may share the label tanggal');
SELECT t.gagal($$INSERT INTO upstream_attempt_daily VALUES ('30000000-0000-7000-8000-000000000001', '2026-11-04 10:00+00', '2026-11-04 20:00+00', false, '2026-11-04', 1, now())$$,
  '23P01', 'periods of one pool never overlap');
ROLLBACK TO s;
SELECT t.gagal($$INSERT INTO upstream_attempt_daily VALUES ('30000000-0000-7000-8000-000000000001', '2026-11-03 17:00+00', '2026-11-03 18:00+00', false, '2026-11-04', 1, now())$$,
  '23505', 'key is (pool, periode_mulai), not tanggal');
ROLLBACK;

-- ---- configuration ranges (KP-01, L-04, EN-03)
BEGIN;
SAVEPOINT s;
SELECT t.gagal($$UPDATE upstream_pool SET margin_kuota = 0 WHERE nama = 'up-1'$$, '23514', 'margin_kuota 0 refused (min 0.001)');
ROLLBACK TO s;
SELECT t.gagal($$UPDATE upstream_pool SET margin_kuota = 0.201 WHERE nama = 'up-1'$$, '23514', 'margin_kuota 0.201 refused (max 0.200)');
ROLLBACK TO s;
SELECT t.jalan($$UPDATE upstream_pool SET margin_kuota = 0.001 WHERE nama = 'up-1'$$, 'margin_kuota 0.001 accepted');
ROLLBACK TO s;
SELECT t.gagal($$UPDATE upstream_pool SET batas_percobaan = 6 WHERE nama = 'up-1'$$, '23514', 'batas_percobaan max 5');
ROLLBACK TO s;
SELECT t.gagal($$UPDATE upstream_pool SET retry_backoff_cap_detik = 4 WHERE nama = 'up-1'$$, '23514', 'cap >= base');
ROLLBACK TO s;
SELECT t.gagal($$UPDATE upstream_pool SET reset_kuota_zona = '+07:00' WHERE nama = 'up-1'$$, '23514', 'fixed offset zone refused');
ROLLBACK TO s;
SELECT t.jalan($$UPDATE upstream_pool SET reset_kuota_zona = 'Asia/Jakarta', reset_kuota_jam = '17:00' WHERE nama = 'up-1'$$, 'IANA zone accepted');
ROLLBACK TO s;
SELECT t.gagal($$UPDATE upstream_pool SET jendela_kuota = 'harian' WHERE nama = 'up-1'$$, '23514', 'jendela_kuota domain');
ROLLBACK TO s;
SELECT t.gagal($$UPDATE client SET limiter_penolakan_burst = 1001 WHERE nama = 'client-x'$$, '23514', 'limiter burst max 1000');
ROLLBACK TO s;
SELECT t.gagal($$UPDATE client SET limiter_penolakan_isi_ulang_per_detik = 0.05 WHERE nama = 'client-x'$$, '23514', 'limiter refill min 0.10');
ROLLBACK TO s;
SELECT t.gagal($$INSERT INTO provider_config_versi VALUES ('20000000-0000-7000-8000-000000000001', 2, 2, 'metadata_only', '{}', now(), '00000000-0000-7000-8000-000000000001')$$,
  '23514', 'config_schema_version other than 1 refused (V-05)');
ROLLBACK TO s;
SELECT t.gagal($$UPDATE provider_config_versi SET dokumen = '{"x":1}'$$, '23000', 'config versions are immutable (V-03)');
ROLLBACK TO s;
SELECT t.gagal($$INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, dibuat_oleh)
  VALUES (gen_random_uuid(), '10000000-0000-7000-8000-000000000001', 'api_key', '\x09', '00000000-0000-7000-8000-000000000001')$$,
  '23514', 'at most 2 active credentials per client');
ROLLBACK TO s;
-- A-02 separation of duties
SELECT t.gagal($$INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh, disetujui_oleh)
  VALUES ('00000000-0000-7000-8000-000000000001', 'auditor', '00000000-0000-7000-8000-000000000001', '00000000-0000-7000-8000-000000000002')$$,
  '23514', 'platform admin cannot grant auditor to themselves (A-02)');
ROLLBACK TO s;
SELECT t.gagal($$INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh, disetujui_oleh)
  VALUES ('00000000-0000-7000-8000-000000000002', 'operator', '00000000-0000-7000-8000-000000000001', '00000000-0000-7000-8000-000000000001')$$,
  '23514', 'approver differs from proposer');
ROLLBACK TO s;
SELECT t.gagal($$INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh, disetujui_oleh)
  VALUES ('00000000-0000-7000-8000-000000000002', 'client_owner', '00000000-0000-7000-8000-000000000001', NULL)$$,
  '23514', 'client_owner needs a client on the user');
ROLLBACK;

