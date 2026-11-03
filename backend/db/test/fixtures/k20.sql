-- Synthetic fixtures for K-20 deactivation and reactivation tests (SHAA-204, addendum v0.7.10 §3, contract v0.7.10 §6.2).
-- Synthetic data only: example.test webhook hosts, 16-digit identifiers with prefix 00, no real person data.
-- Load after 05_fixtures.sql. Deterministic: fixed UUIDs, fixed timestamps, no random values.
--
-- Groups (one route each: client -> downstream endpoint -> downstream pool -> upstream pool -> provider stub):
--   C = client-c   (client_nonaktif)            K = client-x (kredensial_nonaktif, K1/K2 from 05_fixtures)
--   E = client-e   (endpoint_nonaktif)          D = client-d (pool_nonaktif, gate/sweeper)
--   U = client-u   (pool_nonaktif)              P = client-p (provider_nonaktif)
--   Z = client-kedaluwarsa  (TP-K20-7: expired credential, 5 queued requests)
--   B = client-bersama      (TP-K20-8: one provider shared by two upstream pools)
-- Index NN in UUIDs: C=11, K=12, E=13, D=14, U=15, P=16, Z=17, B=18 (second shared pool = 19).
BEGIN;

-- Admin users for the language tests (TP-I18N-10a/10e). bahasa_ui NULL is the case the contract needs.
INSERT INTO admin_user (pengguna_id, username, password_hash, bahasa_ui) VALUES
  ('00000000-0000-7000-8000-000000000005', 'ui-null', '$argon2id$synthetic', NULL),
  ('00000000-0000-7000-8000-000000000006', 'ui-en', '$argon2id$synthetic', 'en');
INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh, disetujui_oleh) VALUES
  ('00000000-0000-7000-8000-000000000005', 'operator', '00000000-0000-7000-8000-000000000001', '00000000-0000-7000-8000-000000000002'),
  ('00000000-0000-7000-8000-000000000006', 'operator', '00000000-0000-7000-8000-000000000001', '00000000-0000-7000-8000-000000000002');

-- Clients for groups C, E, D, U, P, Z, B. Client X (K) already exists in 05_fixtures.sql.
INSERT INTO client (client_id, nama) VALUES
  ('10000000-0000-7000-8000-000000000011', 'client-c'),
  ('10000000-0000-7000-8000-000000000013', 'client-e'),
  ('10000000-0000-7000-8000-000000000014', 'client-d'),
  ('10000000-0000-7000-8000-000000000015', 'client-u'),
  ('10000000-0000-7000-8000-000000000016', 'client-p'),
  ('10000000-0000-7000-8000-000000000017', 'client-kedaluwarsa'),
  ('10000000-0000-7000-8000-000000000018', 'client-bersama');

-- Webhook URL per client (client X already has one in 05_fixtures.sql).
INSERT INTO client_webhook (client_id, webhook_url, aktif, secret_versi, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, diubah_oleh)
SELECT client_id, 'https://hook.example.test/' || nama, true, 1, '\x11', '\x22', '\x33', 'kek-1', '00000000-0000-7000-8000-000000000001'
FROM client WHERE nama IN ('client-c', 'client-e', 'client-d', 'client-u', 'client-p', 'client-kedaluwarsa', 'client-bersama');

-- Expired credential for TP-K20-7. EN-02: kedaluwarsa_at only blocks authentication, not K-20 processing.
INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, kedaluwarsa_at, dibuat_oleh) VALUES
  ('11000000-0000-7000-8000-000000000017', '10000000-0000-7000-8000-000000000017', 'api_key', '\x17', '2026-10-01T00:00:00Z', '00000000-0000-7000-8000-000000000001');

-- Providers (stub). Group K uses its own stub provider; the shared provider for TP-K20-8 is 18.
INSERT INTO provider (provider_id, nama, data_balikan_aktif) VALUES
  ('20000000-0000-7000-8000-000000000011', 'stub-c', false),
  ('20000000-0000-7000-8000-000000000012', 'stub-k', false),
  ('20000000-0000-7000-8000-000000000013', 'stub-e', false),
  ('20000000-0000-7000-8000-000000000014', 'stub-d', false),
  ('20000000-0000-7000-8000-000000000015', 'stub-u', false),
  ('20000000-0000-7000-8000-000000000016', 'stub-p', false),
  ('20000000-0000-7000-8000-000000000017', 'stub-z', false),
  ('20000000-0000-7000-8000-000000000018', 'stub-bersama', false);
INSERT INTO provider_config_versi (provider_id, versi, config_schema_version, log_mode, dokumen, dibuat_oleh)
SELECT provider_id, 1, 1, 'metadata_only', '{}', '00000000-0000-7000-8000-000000000001'
FROM provider WHERE nama LIKE 'stub-%';

-- Upstream pools: batas_percobaan 3 and backoff 1 s for the fixture (addendum §3). Two pools share provider 18 (TP-K20-8).
INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, kuota_harian, batas_percobaan, retry_backoff_base_detik) VALUES
  ('30000000-0000-7000-8000-000000000011', '20000000-0000-7000-8000-000000000011', 'up-c', 1000, 3, 1),
  ('30000000-0000-7000-8000-000000000012', '20000000-0000-7000-8000-000000000012', 'up-k', 1000, 3, 1),
  ('30000000-0000-7000-8000-000000000013', '20000000-0000-7000-8000-000000000013', 'up-e', 1000, 3, 1),
  ('30000000-0000-7000-8000-000000000014', '20000000-0000-7000-8000-000000000014', 'up-d', 1000, 3, 1),
  ('30000000-0000-7000-8000-000000000015', '20000000-0000-7000-8000-000000000015', 'up-u', 1000, 3, 1),
  ('30000000-0000-7000-8000-000000000016', '20000000-0000-7000-8000-000000000016', 'up-p', 1000, 3, 1),
  ('30000000-0000-7000-8000-000000000017', '20000000-0000-7000-8000-000000000017', 'up-z', 1000, 3, 1),
  ('30000000-0000-7000-8000-000000000018', '20000000-0000-7000-8000-000000000018', 'up-bersama-1', 1000, 3, 1),
  ('30000000-0000-7000-8000-000000000019', '20000000-0000-7000-8000-000000000018', 'up-bersama-2', 1000, 3, 1);

-- Downstream pools, each routing to its own upstream pool(s).
INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) VALUES
  ('40000000-0000-7000-8000-000000000011', 'down-c', 1),
  ('40000000-0000-7000-8000-000000000012', 'down-k', 1),
  ('40000000-0000-7000-8000-000000000013', 'down-e', 1),
  ('40000000-0000-7000-8000-000000000014', 'down-d', 1),
  ('40000000-0000-7000-8000-000000000015', 'down-u', 1),
  ('40000000-0000-7000-8000-000000000016', 'down-p', 1),
  ('40000000-0000-7000-8000-000000000017', 'down-z', 1),
  ('40000000-0000-7000-8000-000000000018', 'down-bersama', 1);
INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id) VALUES
  ('40000000-0000-7000-8000-000000000011', '30000000-0000-7000-8000-000000000011'),
  ('40000000-0000-7000-8000-000000000012', '30000000-0000-7000-8000-000000000012'),
  ('40000000-0000-7000-8000-000000000013', '30000000-0000-7000-8000-000000000013'),
  ('40000000-0000-7000-8000-000000000014', '30000000-0000-7000-8000-000000000014'),
  ('40000000-0000-7000-8000-000000000015', '30000000-0000-7000-8000-000000000015'),
  ('40000000-0000-7000-8000-000000000016', '30000000-0000-7000-8000-000000000016'),
  ('40000000-0000-7000-8000-000000000017', '30000000-0000-7000-8000-000000000017'),
  ('40000000-0000-7000-8000-000000000018', '30000000-0000-7000-8000-000000000018'),
  ('40000000-0000-7000-8000-000000000018', '30000000-0000-7000-8000-000000000019');

-- Downstream endpoints, one per group.
INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, downstream_pool_id) VALUES
  ('50000000-0000-7000-8000-000000000011', 'ep-c', '20000000-0000-7000-8000-000000000011', '40000000-0000-7000-8000-000000000011'),
  ('50000000-0000-7000-8000-000000000012', 'ep-k', '20000000-0000-7000-8000-000000000012', '40000000-0000-7000-8000-000000000012'),
  ('50000000-0000-7000-8000-000000000013', 'ep-e', '20000000-0000-7000-8000-000000000013', '40000000-0000-7000-8000-000000000013'),
  ('50000000-0000-7000-8000-000000000014', 'ep-d', '20000000-0000-7000-8000-000000000014', '40000000-0000-7000-8000-000000000014'),
  ('50000000-0000-7000-8000-000000000015', 'ep-u', '20000000-0000-7000-8000-000000000015', '40000000-0000-7000-8000-000000000015'),
  ('50000000-0000-7000-8000-000000000016', 'ep-p', '20000000-0000-7000-8000-000000000016', '40000000-0000-7000-8000-000000000016'),
  ('50000000-0000-7000-8000-000000000017', 'ep-z', '20000000-0000-7000-8000-000000000017', '40000000-0000-7000-8000-000000000017'),
  ('50000000-0000-7000-8000-000000000018', 'ep-bersama', '20000000-0000-7000-8000-000000000018', '40000000-0000-7000-8000-000000000018');
INSERT INTO endpoint_config_versi (downstream_endpoint_id, versi, config_schema_version, dokumen, dibuat_oleh)
SELECT downstream_endpoint_id, 1, 1, '{}', '00000000-0000-7000-8000-000000000001'
FROM downstream_endpoint WHERE nama LIKE 'ep-%' AND nama <> 'ep-1';

-- Routes: client -> endpoint. Group K adds a second route for client X (its existing route is ep-1).
INSERT INTO client_endpoint (client_id, downstream_endpoint_id) VALUES
  ('10000000-0000-7000-8000-000000000011', '50000000-0000-7000-8000-000000000011'),
  ('10000000-0000-7000-8000-000000000001', '50000000-0000-7000-8000-000000000012'),
  ('10000000-0000-7000-8000-000000000013', '50000000-0000-7000-8000-000000000013'),
  ('10000000-0000-7000-8000-000000000014', '50000000-0000-7000-8000-000000000014'),
  ('10000000-0000-7000-8000-000000000015', '50000000-0000-7000-8000-000000000015'),
  ('10000000-0000-7000-8000-000000000016', '50000000-0000-7000-8000-000000000016'),
  ('10000000-0000-7000-8000-000000000017', '50000000-0000-7000-8000-000000000017'),
  ('10000000-0000-7000-8000-000000000018', '50000000-0000-7000-8000-000000000018');

-- Five queued async requests for TP-K20-7 on the expired credential (EN-02: still queued and processed).
INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, downstream_pool_id, provider_id,
  provider_config_version, endpoint_config_version, mode, prioritas, prioritas_efektif, status, input_payload, metadata,
  referensi_client, received_at, ttl_expires_at, attempt_count)
SELECT ('60000000-0000-7000-8000-00000000000' || n)::uuid,
  '10000000-0000-7000-8000-000000000017', '11000000-0000-7000-8000-000000000017',
  '50000000-0000-7000-8000-000000000017', '40000000-0000-7000-8000-000000000017', '20000000-0000-7000-8000-000000000017',
  1, 1, 'async', 1, 1, 'queued',
  jsonb_build_object('NIK', '000000000000000' || n), '{}'::jsonb,
  'SINTETIS-K20-7-' || n, '2026-11-03T01:00:00Z', '2026-11-04T01:00:00Z', 0
FROM generate_series(1, 5) AS n;

-- T-04: every request status needs a matching history row (initial queued entry, written by the webserver).
INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor)
SELECT request_id, 1, NULL, 'queued', 'webserver' FROM request WHERE referensi_client LIKE 'SINTETIS-K20-7-%';

COMMIT;
