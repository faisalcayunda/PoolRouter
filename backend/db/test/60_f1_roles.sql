-- F1 access matrix (SHAA-333): contract F1-5.4, F1-6.5, F1-7.4 MF-09, WR-07, F1-9 summary; DQ-50 (e).
-- Statements run as the role under test (SET LOCAL ROLE), inside transactions that are rolled back.
\set ON_ERROR_STOP on

-- ============ NIK stays out of reach of the webserver and the controller in every F1 object
BEGIN;
SET LOCAL ROLE pr_api;
SELECT t.jalan('SELECT sumber_anggaran, downstream_pool_id FROM transaction_log', 'pr_api: reads the new non-NIK log columns');
SELECT t.gagal('SELECT nik FROM transaction_log', '42501', 'pr_api: still no NIK in transaction_log');
SELECT t.gagal('SELECT * FROM data_balikan_item', '42501', 'pr_api: no access to data_balikan_item (carries NIK)');
SELECT t.gagal('SELECT nik FROM arsip_pulih.transaction_log', '42501', 'pr_api: no access to arsip_pulih');
SELECT t.gagal('SELECT 1 FROM pengguna_mfa', '42501', 'pr_api: no access to MFA tables');
SELECT t.gagal('SELECT 1 FROM arsip_restore_permintaan', '42501', 'pr_api: no access to restore requests');
SELECT t.jalan('SELECT aging_ambang_detik, prioritas_diubah_at, reject_lingkup FROM request', 'pr_api: reads the new request columns');
SELECT t.gagal('SELECT webhook_not_before FROM request', '42501', 'WR-07: pr_api cannot read webhook_not_before');
SELECT t.gagal('SELECT input_payload FROM request', '42501', 'pr_api: still no input_payload');
SELECT t.gagal($$UPDATE request SET webhook_not_before = now()$$, '42501', 'WR-07: pr_api cannot write webhook_not_before');
SELECT t.gagal($$UPDATE request SET prioritas_efektif = 1$$, '42501', 'pr_api cannot change the priority');
ROLLBACK;

BEGIN;
SET LOCAL ROLE pr_controller;
SELECT t.jalan('SELECT aging_ambang_detik, prioritas_diubah_at, reject_lingkup FROM request', 'pr_controller: reads the new request columns');
SELECT t.gagal('SELECT webhook_not_before FROM request', '42501', 'WR-07: pr_controller cannot read webhook_not_before');
SELECT t.jalan('SELECT sumber_anggaran, downstream_pool_id FROM transaction_log', 'pr_controller: reads the new non-NIK log columns');
SELECT t.gagal('SELECT nik FROM transaction_log', '42501', 'pr_controller: still no NIK in transaction_log');
SELECT t.gagal('SELECT nik FROM data_balikan_item', '42501', 'pr_controller: no NIK from data_balikan_item');
SELECT t.gagal('SELECT * FROM arsip_pulih.transaction_log', '42501', 'pr_controller: no access to arsip_pulih');
SELECT t.gagal('SELECT 1 FROM pengguna_mfa', '42501', 'pr_controller: no access to MFA tables');
SELECT t.gagal('SELECT 1 FROM kode_daftar_mfa', '42501', 'pr_controller: no access to enrolment codes');
SELECT t.gagal($$UPDATE transaction_log SET archived_at = now()$$, '42501', 'contract 8.1: archived_at moved from the controller to pr_job_arsip');
SELECT t.gagal($$UPDATE request SET aging_ambang_detik = 1$$, '42501', 'AG-02: the controller cannot change the snapshot');
SELECT t.gagal($$UPDATE request SET webhook_not_before = now()$$, '42501', 'WR-07: pr_controller cannot write webhook_not_before');
SELECT t.gagal('SELECT 1 FROM arsip_manifest', '42501', 'pr_controller: no access to the archive manifest');
ROLLBACK;

-- D1: the controller climbs the priority (one step, with its history row) through its column grants
BEGIN;
SELECT t.buat_request('{"request_id":"f9000000-0000-7000-8000-000000000001","prioritas":3,"prioritas_efektif":3,"aging_ambang_detik":600,
  "prioritas_diubah_at":"2026-11-04T00:00:00Z","received_at":"2026-11-04T00:00:00Z"}');
SET LOCAL ROLE pr_controller;
SELECT t.jalan_commit($$UPDATE request SET prioritas_efektif = 2, prioritas_diubah_at = now() WHERE request_id = 'f9000000-0000-7000-8000-000000000001' AND prioritas_efektif = 3;
  INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor, prioritas_dari, prioritas_ke)
  VALUES ('f9000000-0000-7000-8000-000000000001', 2, 'queued', 'queued', 'aging', 'controller', 3, 2)$$, 'U-57: pr_controller aging step (update + history row) works with its grants');
ROLLBACK;

BEGIN;
SET LOCAL ROLE pr_webhook;
SELECT t.jalan('SELECT request_id, webhook_not_before FROM request', 'WR-07: pr_webhook reads webhook_not_before');
SELECT t.jalan($$UPDATE request SET webhook_not_before = now() WHERE false$$, 'WR-07: pr_webhook updates webhook_not_before');
SELECT t.gagal('SELECT 1 FROM pengguna_mfa', '42501', 'pr_webhook: no MFA tables');
SELECT t.gagal('SELECT * FROM arsip_pulih.transaction_log', '42501', 'pr_webhook: no arsip_pulih');
ROLLBACK;
BEGIN;
SET LOCAL ROLE pr_worker;
SELECT t.gagal($$UPDATE request SET webhook_not_before = now()$$, '42501', 'WR-07: pr_worker cannot write webhook_not_before');
SELECT t.gagal('SELECT webhook_not_before FROM request', '42501', 'WR-07: pr_worker cannot read webhook_not_before');
SELECT t.jalan('SELECT aging_ambang_detik, prioritas_diubah_at, reject_lingkup FROM request', 'pr_worker: reads the new request columns');
SELECT t.gagal('SELECT 1 FROM pengguna_mfa', '42501', 'pr_worker: no MFA tables');
SELECT t.gagal('SELECT 1 FROM data_balikan_item', '42501', 'pr_worker: no data_balikan_item');
ROLLBACK;

-- ============ MF-09: MFA tables belong to pr_admin_api alone
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.role_table_grants
   WHERE table_name IN ('pengguna_mfa', 'kode_pemulihan_mfa', 'kode_daftar_mfa') AND table_schema = 'public'
     AND grantee NOT IN ('pr_admin_api', 'poolrouter_owner')$q$) = 0, 'DQ-50 (e) / MF-09: only pr_admin_api (and the owner) hold privileges on the three MFA tables');
SELECT t.ok(t.hitung($q$
  SELECT count(DISTINCT privilege_type) FROM information_schema.role_table_grants
   WHERE table_name IN ('pengguna_mfa', 'kode_pemulihan_mfa', 'kode_daftar_mfa') AND grantee = 'pr_admin_api'
     AND privilege_type IN ('SELECT', 'INSERT', 'UPDATE', 'DELETE')$q$) = 4, 'MF-09: pr_admin_api has SELECT/INSERT/UPDATE/DELETE on the MFA tables');
BEGIN;
SET LOCAL ROLE pr_admin_api;
SELECT t.jalan('SELECT secret_ciphertext FROM pengguna_mfa', 'MF-09: the admin API reads the ciphertext (verification happens there)');
SELECT t.jalan('SELECT 1 FROM kode_pemulihan_mfa', 'pr_admin_api: recovery codes');
SELECT t.jalan('SELECT 1 FROM kode_daftar_mfa', 'pr_admin_api: enrolment codes');
SELECT t.gagal('SELECT 1 FROM arsip_pulih.transaction_log', '42501', 'pr_admin_api: no arsip_pulih (only the auditor SECURITY DEFINER function reads it)');
SELECT t.jalan('SELECT manifest_id, tabel_sumber, provider_id, tanggal_wib, kunci_hmac_versi, status FROM arsip_manifest', 'pr_admin_api: reads only the columns needed for the HMAC key version of a range');
SELECT t.gagal('SELECT dek_terbungkus FROM arsip_manifest', '42501', 'AR-02c: pr_admin_api cannot read dek_terbungkus');
SELECT t.gagal('SELECT * FROM arsip_manifest', '42501', 'pr_admin_api: no whole-row read of the manifest');
SELECT t.gagal('SELECT objek_kunci, sha256 FROM arsip_manifest', '42501', 'pr_admin_api: no object key or checksum');
SELECT t.gagal($$DELETE FROM arsip_manifest$$, '42501', 'pr_admin_api: cannot delete manifest rows (kept as the deletion record)');
SELECT t.jalan('SELECT 1 FROM arsip_restore_permintaan', 'pr_admin_api: reads restore requests (status visible to the auditor)');
SELECT t.gagal($$UPDATE arsip_restore_permintaan SET status = 'siap'$$, '42501', 'pr_admin_api: restore status is moved by the worker only');
SELECT t.jalan($$UPDATE downstream_pool SET aging_ambang_detik = 600 WHERE false$$, 'pr_admin_api: sets the aging threshold');
SELECT t.jalan('SELECT fn_alokasi_kelebihan(upstream_pool_id) FROM upstream_pool', 'pr_admin_api: executes fn_alokasi_kelebihan');
ROLLBACK;

-- ============ pr_job_balikan (F1-5.4)
BEGIN;
SET LOCAL ROLE pr_job_balikan;
SELECT t.jalan('SELECT status, input_payload, metadata, attempt_count FROM request', 'pr_job_balikan: reads request columns the composition needs');
SELECT t.jalan('SELECT nik, kode_transaksi, requested_at FROM transaction_log', 'pr_job_balikan: reads the NIK it must put in items');
SELECT t.gagal('SELECT output_payload FROM request', '42501', 'pr_job_balikan: no output_payload');
SELECT t.gagal('SELECT request_payload FROM transaction_log', '42501', 'pr_job_balikan: no request_payload');
SELECT t.gagal('SELECT response_payload FROM transaction_log', '42501', 'pr_job_balikan: no response_payload');
SELECT t.jalan('SELECT provider_id, data_balikan_aktif, config_versi_aktif FROM provider', 'pr_job_balikan: reads provider settings');
SELECT t.jalan('SELECT dokumen FROM provider_config_versi', 'pr_job_balikan: reads the provider config (data_balikan block)');
SELECT t.jalan('SELECT 1 FROM endpoint_metadata_field', 'pr_job_balikan: reads which metadata fields go to data balikan');
SELECT t.gagal('SELECT 1 FROM audit_akses', '42501', 'pr_job_balikan: no audit_akses');
SELECT t.gagal('SELECT 1 FROM client_webhook', '42501', 'pr_job_balikan: no client_webhook');
SELECT t.gagal('SELECT 1 FROM provider_secret', '42501', 'pr_job_balikan: no provider secrets');
SELECT t.gagal('SELECT 1 FROM pengguna_mfa', '42501', 'pr_job_balikan: no MFA tables');
SELECT t.gagal('SELECT 1 FROM arsip_manifest', '42501', 'pr_job_balikan: no archive objects');
SELECT t.gagal('SELECT 1 FROM arsip_pulih.transaction_log', '42501', 'pr_job_balikan: no arsip_pulih');
SELECT t.jalan('SELECT 1 FROM data_balikan_batch', 'pr_job_balikan: reads batches');
SELECT t.jalan('SELECT 1 FROM data_balikan_item', 'pr_job_balikan: reads items');
SELECT t.jalan($$UPDATE data_balikan_batch SET attempt_count = attempt_count WHERE false$$, 'pr_job_balikan: updates batches');
SELECT t.jalan($$UPDATE data_balikan_item SET aktif = aktif WHERE false$$, 'pr_job_balikan: updates items');
SELECT t.gagal($$UPDATE request SET status = status$$, '42501', 'pr_job_balikan: cannot write request');
SELECT t.gagal($$UPDATE transaction_log SET archived_at = now()$$, '42501', 'pr_job_balikan: cannot write the log');
SELECT t.gagal($$DELETE FROM data_balikan_item$$, '42501', 'pr_job_balikan: no DELETE on items (job does not delete anything)');
SELECT t.gagal($$DELETE FROM data_balikan_batch$$, '42501', 'pr_job_balikan: no DELETE on batches');
ROLLBACK;

-- the composer runs end to end on its own grants: batch -> item -> gagal_permanen -> items inactive
BEGIN;
SET LOCAL ROLE pr_job_balikan;
SELECT t.jalan($$INSERT INTO data_balikan_batch (batch_id, provider_id, pemicu, periode_mulai_at, periode_selesai_at, jumlah_item, status, jadwal_kirim_at, config_ref_version)
  VALUES ('f9100000-0000-7000-8000-000000000001', '20000000-0000-7000-8000-000000000001', 'periode', now(), now(), 1, 'disusun', now(), 1)$$, 'pr_job_balikan inserts a batch');
SELECT t.jalan($$INSERT INTO data_balikan_item (batch_id, request_id, kode_transaksi, nik, field_input, field_metadata, requested_at)
  VALUES ('f9100000-0000-7000-8000-000000000001', 'a0000000-0000-7000-8000-000000000001', 'b0000000-0000-7000-8000-000000000001', '0000000000000021', '{}', '{}', now())$$, 'pr_job_balikan inserts an item');
SELECT t.jalan($$UPDATE data_balikan_batch SET status = 'gagal_permanen' WHERE batch_id = 'f9100000-0000-7000-8000-000000000001'$$, 'pr_job_balikan moves the batch to gagal_permanen');
SELECT t.ok(t.hitung($$SELECT count(*) FROM data_balikan_item WHERE batch_id = 'f9100000-0000-7000-8000-000000000001' AND NOT aktif$$) = 1, 'the trigger deactivated the item with the job role''s own grants');
ROLLBACK;

-- ============ pr_job_arsip (F1-6.5)
BEGIN;
SET LOCAL ROLE pr_job_arsip;
SELECT t.jalan('SELECT nik, request_payload, response_payload FROM transaction_log', 'pr_job_arsip: reads the partitions it archives, NIK and payload included');
SELECT t.jalan('SELECT input_payload, output_payload, metadata FROM request', 'pr_job_arsip: reads request');
SELECT t.jalan('SELECT 1 FROM request_status_history', 'pr_job_arsip: reads status history');
SELECT t.jalan('SELECT 1 FROM webhook_log', 'pr_job_arsip: reads webhook_log');
SELECT t.jalan('SELECT 1 FROM audit_akses', 'pr_job_arsip: reads audit_akses (the partition being archived)');
SELECT t.jalan('SELECT 1 FROM arsip_manifest', 'pr_job_arsip: reads the manifest');
SELECT t.jalan($$INSERT INTO arsip_manifest (manifest_id, tabel_sumber, provider_id, tanggal_wib, objek_kunci, format, versi_contract, jumlah_baris, sha256, ukuran_byte, status)
  VALUES (gen_random_uuid(), 'webhook_log', '20000000-0000-7000-8000-000000000001', '2026-08-09', 'arsip/webhook/2026-08-09', 'parquet-zstd-1', 'v0.9.5', 1, repeat('d', 64), 1, 'ditulis')$$,
  'pr_job_arsip: writes the manifest');
SELECT t.jalan($$UPDATE arsip_manifest SET status = 'gagal_verifikasi'$$, 'pr_job_arsip: updates the manifest');
SELECT t.jalan($$UPDATE transaction_log SET archived_at = now() WHERE false$$, 'pr_job_arsip: sets archived_at');
SELECT t.gagal($$UPDATE transaction_log SET worker_id = 'x'$$, '42501', 'pr_job_arsip: archived_at only');
SELECT t.gagal($$UPDATE request SET status = status$$, '42501', 'pr_job_arsip: cannot write request');
SELECT t.gagal($$DELETE FROM transaction_log$$, '42501', 'AR-04: pr_job_arsip has no direct DELETE on transaction_log');
SELECT t.gagal($$DELETE FROM request$$, '42501', 'AR-04: no direct DELETE on request');
SELECT t.gagal($$DELETE FROM webhook_log$$, '42501', 'AR-04: no direct DELETE on webhook_log');
SELECT t.gagal($$DELETE FROM audit_akses$$, '42501', 'AR-04: no direct DELETE on audit_akses');
SELECT t.gagal($$DELETE FROM arsip_manifest$$, '42501', 'pr_job_arsip: manifest rows are never deleted');
SELECT t.gagal('SELECT 1 FROM arsip_restore_permintaan', '42501', 'pr_job_arsip: no restore requests');
SELECT t.gagal('SELECT 1 FROM arsip_pulih.transaction_log', '42501', 'pr_job_arsip: no arsip_pulih');
SELECT t.gagal('SELECT 1 FROM client_webhook', '42501', 'pr_job_arsip: no client_webhook');
SELECT t.gagal('SELECT 1 FROM provider_secret', '42501', 'pr_job_arsip: no provider secrets');
SELECT t.gagal('SELECT 1 FROM pengguna_mfa', '42501', 'pr_job_arsip: no MFA tables');
SELECT t.gagal($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, hasil, sumber_ip, sesi_id) VALUES (gen_random_uuid(), gen_random_uuid(), 'x', 'lihat_audit', 'tidak_ada', 'diizinkan', 'x', 'x')$$,
  '42501', 'pr_job_arsip cannot write audit_akses');
ROLLBACK;

-- ============ pr_restore (F1-6.5)
BEGIN;
SET LOCAL ROLE pr_restore;
SELECT t.jalan('SELECT 1 FROM arsip_manifest', 'pr_restore: reads the manifest');
SELECT t.jalan('SELECT 1 FROM arsip_restore_permintaan', 'pr_restore: reads restore requests');
SELECT t.jalan($$UPDATE arsip_restore_permintaan SET status = 'berjalan', mulai_at = now(), jumlah_baris_pulih = 0, error_terakhir = NULL WHERE false$$, 'pr_restore: updates status and time columns');
SELECT t.gagal($$UPDATE arsip_restore_permintaan SET alasan = 'x'$$, '42501', 'pr_restore: cannot change the request identity (reason)');
SELECT t.gagal($$UPDATE arsip_restore_permintaan SET nik_hmac_per_versi = NULL$$, '42501', 'pr_restore: pseudonyms are emptied by the expiry function only');
SELECT t.gagal($$INSERT INTO arsip_restore_permintaan (restore_id) VALUES (gen_random_uuid())$$, '42501', 'pr_restore: cannot create requests');
SELECT t.gagal($$UPDATE arsip_manifest SET status = 'objek_dihapus'$$, '42501', 'pr_restore: manifest is read-only');
SELECT t.gagal('SELECT 1 FROM arsip_pulih.transaction_log', '42501', 'pr_restore: INSERT/DELETE only, no SELECT on arsip_pulih (AR-02c / DQ-50 e)');
SELECT t.gagal($$DELETE FROM arsip_pulih.transaction_log WHERE restore_id = gen_random_uuid()$$, '42501', 'pr_restore: a filtered DELETE would read arsip_pulih, so expiry goes through the function');
SELECT t.jalan($$SELECT fn_arsip_pulih_kedaluwarsa(gen_random_uuid())$$, 'pr_restore: executes the expiry function');
SELECT t.gagal('SELECT nik FROM transaction_log', '42501', 'pr_restore: no access to the online log');
SELECT t.gagal('SELECT 1 FROM request', '42501', 'pr_restore: no access to request');
SELECT t.gagal('SELECT 1 FROM audit_akses', '42501', 'pr_restore: audit_akses is INSERT only (the restore_arsip row)');
SELECT t.jalan($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, alasan, hasil, sumber_ip, sesi_id, target_restore_id)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000003', 'auditor', 'restore_arsip', 'tidak_ada', 'periksa', 'diizinkan', 'worker', 'job', gen_random_uuid())$$,
  'pr_restore: writes the restore_arsip audit row (sumber online, restore id set)');
SELECT t.gagal('SELECT 1 FROM pengguna_mfa', '42501', 'pr_restore: no MFA tables');
SELECT t.gagal('SELECT 1 FROM client_webhook', '42501', 'pr_restore: no client_webhook');
ROLLBACK;

-- ============ catalog checks (DQ-50 e): nobody reads arsip_pulih except the owner; no DELETE for the new roles
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM (
    SELECT (aclexplode(c.relacl)).grantee::regrole::text AS g, (aclexplode(c.relacl)).privilege_type AS p
      FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'arsip_pulih' AND c.relkind IN ('r', 'v', 'p')
    UNION ALL
    SELECT (aclexplode(a.attacl)).grantee::regrole::text, (aclexplode(a.attacl)).privilege_type
      FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace
     WHERE n.nspname = 'arsip_pulih' AND a.attacl IS NOT NULL) x
   WHERE g NOT IN ('poolrouter_owner') AND p IN ('SELECT', 'REFERENCES')$q$) = 0, 'DQ-50 (e): no role but the owner has SELECT on any arsip_pulih object (tables or columns)');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace, aclexplode(c.relacl) x
   WHERE n.nspname = 'arsip_pulih' AND c.relkind = 'r' AND x.grantee = 0$q$) = 0, 'arsip_pulih tables carry no PUBLIC grant');
SELECT t.ok(NOT has_schema_privilege('pr_api', 'arsip_pulih', 'USAGE') AND NOT has_schema_privilege('pr_controller', 'arsip_pulih', 'USAGE')
            AND NOT has_schema_privilege('pr_worker', 'arsip_pulih', 'USAGE') AND NOT has_schema_privilege('pr_webhook', 'arsip_pulih', 'USAGE')
            AND NOT has_schema_privilege('pr_admin_api', 'arsip_pulih', 'USAGE') AND NOT has_schema_privilege('pr_job_arsip', 'arsip_pulih', 'USAGE')
            AND NOT has_schema_privilege('pr_job_balikan', 'arsip_pulih', 'USAGE') AND has_schema_privilege('pr_restore', 'arsip_pulih', 'USAGE'),
  'only pr_restore may even enter schema arsip_pulih');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.role_table_grants
   WHERE grantee IN ('pr_job_balikan', 'pr_job_arsip') AND privilege_type IN ('DELETE', 'TRUNCATE')$q$) = 0, 'job roles pr_job_balikan / pr_job_arsip hold no DELETE or TRUNCATE anywhere');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.role_table_grants
   WHERE grantee = 'pr_restore' AND privilege_type IN ('DELETE', 'TRUNCATE') AND NOT (table_schema = 'arsip_pulih' AND privilege_type = 'DELETE')$q$) = 0,
  'pr_restore may DELETE only in arsip_pulih');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.role_table_grants
   WHERE grantee IN ('pr_api', 'pr_worker', 'pr_controller', 'pr_webhook', 'pr_job_balikan', 'pr_job_arsip', 'pr_restore')
     AND table_name IN ('pengguna_mfa', 'kode_pemulihan_mfa', 'kode_daftar_mfa')$q$) = 0, 'DQ-50 (e): no component or job role has any right on the MFA tables');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.role_table_grants
   WHERE grantee IN ('pr_api', 'pr_worker', 'pr_controller', 'pr_webhook', 'pr_admin_api') AND table_name IN ('arsip_manifest', 'arsip_restore_permintaan')
     AND privilege_type IN ('INSERT', 'UPDATE', 'DELETE', 'TRUNCATE') AND NOT (grantee = 'pr_admin_api' AND table_name = 'arsip_restore_permintaan' AND privilege_type = 'INSERT')$q$) = 0,
  'only the admin API creates restore requests; nobody else writes the manifest or the requests among the F0 roles');
SELECT t.ok(NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname IN ('operator', 'pr_operator')), 'AR-08: the operator has no database role, hence no archive access');

-- every new role can log in only with its own secret and keeps the log-parameter protection (R-04)
SELECT t.ok(t.hitung($q$SELECT count(*) FROM pg_roles r WHERE r.rolname IN ('pr_job_balikan', 'pr_job_arsip', 'pr_restore')
  AND r.rolcanlogin AND NOT r.rolsuper AND NOT r.rolcreatedb AND NOT r.rolcreaterole AND NOT r.rolbypassrls AND NOT r.rolreplication$q$) = 3,
  'new job roles: LOGIN, not superuser, no CREATEDB / CREATEROLE / BYPASSRLS / REPLICATION');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM pg_db_role_setting s JOIN pg_roles r ON r.oid = s.setrole
  WHERE r.rolname IN ('pr_job_balikan', 'pr_job_arsip', 'pr_restore')
    AND s.setconfig @> ARRAY['log_parameter_max_length=0'] AND s.setconfig @> ARRAY['log_parameter_max_length_on_error=0']$q$) = 3,
  'new job roles never log statement parameters (R-04)');
