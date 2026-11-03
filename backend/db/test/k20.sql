-- K-20 fixture for scenario U-54 (mass cancel of a credential, SHAA-255).
-- Synthetic data only: no NIK or other real identity; payloads use the 0000... prefix (contract 6.1).
-- Deterministic: fixed UUIDs, no random(), no now()-dependent identifiers.
-- Requires migrations and 05_fixtures.sql. NOT matched by run.sh (its globs are 0*.sql and [1-9]*.sql);
-- load it with backend/db/test/k20_load.sh.
--
-- Client K-20 with credentials K1 and K2 (same as U-54 in 40_functions.sql):
--   K1 (11...a1): requests n = 1..10  queued, attempt_count 0
--                 requests n = 11..15 queued, attempt_count 1 (dispatched once, transaction_log row, back in queued)
--   K2 (11...a2): requests n = 16..25 queued, attempt_count 0
-- Every status is written in its own COMMIT with a matching request_status_history row (T-04 on the commit path).
-- terjadi_at takes the transaction time (T-04 requires terjadi_at >= transaction_timestamp()), so it is not set here.
-- Expected preview: fn_pratinjau_nonaktif('kredensial', K1) = 10 cancelled / 5 failed; ('client', K-20) = 20 / 5.

-- T1: client, credentials, endpoint link, and 25 queued requests with their first history row
BEGIN;
INSERT INTO client (client_id, nama) VALUES ('10000000-0000-7000-8000-0000000000b1'::uuid, 'client-k20-sintetis');
INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, dibuat_oleh) VALUES
  ('11000000-0000-7000-8000-0000000000b1', '10000000-0000-7000-8000-0000000000b1'::uuid, 'api_key', '\x01', '00000000-0000-7000-8000-000000000001'),
  ('11000000-0000-7000-8000-0000000000b2', '10000000-0000-7000-8000-0000000000b1'::uuid, 'api_key', '\x02', '00000000-0000-7000-8000-000000000001');
INSERT INTO client_endpoint VALUES ('10000000-0000-7000-8000-0000000000b1'::uuid, '50000000-0000-7000-8000-000000000001'::uuid);

INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, downstream_pool_id, provider_id,
  provider_config_version, endpoint_config_version, mode, prioritas, prioritas_efektif, status, metadata,
  received_at, ttl_expires_at, attempt_count, webhook_attempt_count, dispatch_key, input_payload)
SELECT ('12000000-0000-7000-8000-' || lpad(n::text, 12, '0'))::uuid,
  '10000000-0000-7000-8000-0000000000b1'::uuid,
  CASE WHEN n <= 15 THEN '11000000-0000-7000-8000-0000000000b1'::uuid ELSE '11000000-0000-7000-8000-0000000000b2'::uuid END,
  '50000000-0000-7000-8000-000000000001'::uuid, '40000000-0000-7000-8000-000000000001'::uuid, '20000000-0000-7000-8000-000000000001'::uuid,
  1, 1, 'async', 1, 1, 'queued', '{"status_penerima_manfaat":"1","referensi_client":"k20-sintetis"}'::jsonb,
  '2026-11-03T00:00:00Z', '2026-11-04T00:00:00Z', 0, 0,
  ('13000000-0000-7000-8000-' || lpad(n::text, 12, '0'))::uuid,
  jsonb_build_object('NIK', '00' || lpad(n::text, 14, '0'))
FROM generate_series(1, 25) AS n;

INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor, alasan)
SELECT ('12000000-0000-7000-8000-' || lpad(n::text, 12, '0'))::uuid, 1, NULL, 'queued', 'webserver', NULL
FROM generate_series(1, 25) AS n;
COMMIT;

-- T2: retry requests 11..15 dispatched (queued -> dispatched, attempt 1)
BEGIN;
UPDATE request SET status = 'dispatched', attempt_count = 1, upstream_pool_id = '30000000-0000-7000-8000-000000000001'::uuid
  WHERE request_id IN (SELECT ('12000000-0000-7000-8000-' || lpad(n::text, 12, '0'))::uuid FROM generate_series(11, 15) AS n);
INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor, alasan)
SELECT ('12000000-0000-7000-8000-' || lpad(n::text, 12, '0'))::uuid, 2, 'queued', 'dispatched', 'controller', NULL
FROM generate_series(11, 15) AS n;
COMMIT;

-- T3: dispatched -> in_flight
BEGIN;
UPDATE request SET status = 'in_flight'
  WHERE request_id IN (SELECT ('12000000-0000-7000-8000-' || lpad(n::text, 12, '0'))::uuid FROM generate_series(11, 15) AS n);
INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor, alasan)
SELECT ('12000000-0000-7000-8000-' || lpad(n::text, 12, '0'))::uuid, 3, 'dispatched', 'in_flight', 'worker', NULL
FROM generate_series(11, 15) AS n;
COMMIT;

-- T4: in_flight -> queued (provider error, retry scheduled); one transaction_log row per attempt
BEGIN;
UPDATE request SET status = 'queued'
  WHERE request_id IN (SELECT ('12000000-0000-7000-8000-' || lpad(n::text, 12, '0'))::uuid FROM generate_series(11, 15) AS n);
INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor, alasan)
SELECT ('12000000-0000-7000-8000-' || lpad(n::text, 12, '0'))::uuid, 4, 'in_flight', 'queued', 'worker', 'provider_sementara'
FROM generate_series(11, 15) AS n;
INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
  provider_config_version, endpoint_config_version, nik, requested_at, status_akses, error_class, log_mode, worker_id)
SELECT ('14000000-0000-7000-8000-' || lpad(n::text, 12, '0'))::uuid, ('12000000-0000-7000-8000-' || lpad(n::text, 12, '0'))::uuid,
  1, '20000000-0000-7000-8000-000000000001'::uuid, '30000000-0000-7000-8000-000000000001'::uuid,
  '10000000-0000-7000-8000-0000000000b1'::uuid, '50000000-0000-7000-8000-000000000001'::uuid, 1, 1,
  '0000000000000008', '2026-11-03T00:02:00Z', 'gagal_provider', 'provider_sementara', 'metadata_only', 'w1'
FROM generate_series(11, 15) AS n;
COMMIT;
