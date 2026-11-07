-- F0 schema, part 4: grants per component role (SHAA-193).
-- Source: ADR-0001 rev 15 section 7.2, data contract v0.7.10 sections 8.1, 8.1a, 8.1b.
-- Roles (pr_api, pr_admin_api, pr_controller, pr_worker, pr_webhook) come from backend/db/bootstrap.sh.
-- Column grants only on request / transaction_log / client / client_kredensial / client_webhook.
-- An UPDATE that reads a column in WHERE or RETURNING needs SELECT on it, so CAS columns stay readable.

-- All columns of a table except the listed ones, for column-level grants.
CREATE FUNCTION pg_temp.kolom(p_tabel text, p_kecuali text[] DEFAULT ARRAY[]::text[]) RETURNS text
LANGUAGE sql AS $$
  SELECT string_agg(quote_ident(column_name), ', ' ORDER BY ordinal_position)
    FROM information_schema.columns
   WHERE table_schema = 'public' AND table_name = p_tabel AND column_name <> ALL (p_kecuali);
$$;

-- ================================================================ pr_api (webserver: downstream API and sync path)
GRANT SELECT ON downstream_endpoint, downstream_pool, downstream_pool_rute, upstream_pool, provider, provider_config_versi,
  endpoint_config_versi, endpoint_metadata_field, client_endpoint, provider_secret, upstream_attempt_daily TO pr_api;
GRANT SELECT (client_id, nama, aktif, limiter_penolakan_burst, limiter_penolakan_isi_ulang_per_detik) ON client TO pr_api;
GRANT SELECT (credential_id, client_id, jenis, kunci_hash, status, kedaluwarsa_at, dibuat_at) ON client_kredensial TO pr_api;   -- authentication; dibuat_at is the store's ORDER BY
GRANT SELECT (client_id, aktif) ON client_webhook TO pr_api;
DO $$ BEGIN
  EXECUTE format('GRANT SELECT (%s) ON request TO pr_api', pg_temp.kolom('request', ARRAY['input_payload', 'metadata']));
END $$;
GRANT INSERT ON request TO pr_api;
-- Client cancel (K-19) and the sync outcome; payload purge goes through fn_purge_p02.
GRANT UPDATE (status, upstream_pool_id, dispatched_at, first_attempt_at, completed_at, attempt_count, final_kode_transaksi,
  output_payload, sumber_hasil, failure_reason, cancel_reason, cancelled_by, webhook_status, not_before) ON request TO pr_api;
GRANT SELECT, INSERT ON request_status_history TO pr_api;
-- 8.1: the webserver service account may WRITE the NIK and the input payload, not read them. A-01: full NIK and
-- payload are readable only through audited SECURITY DEFINER functions, pr_worker and pr_webhook. So no `nik`,
-- `request_payload` or `response_payload` here, and no table-level SELECT (which would grant every column).
DO $$ BEGIN
  EXECUTE format('GRANT SELECT (%s) ON transaction_log TO pr_api',
                 pg_temp.kolom('transaction_log', ARRAY['nik', 'request_payload', 'response_payload']));
END $$;
GRANT INSERT ON transaction_log TO pr_api;
-- Sync path: write NIK / request payload once, then the outcome. Trigger R-03 limits the update.
GRANT UPDATE (nik, request_payload, response_payload, responded_at, duration_ms, status_akses, provider_http_status,
  provider_response_code, error_class, response_hash) ON transaction_log TO pr_api;
GRANT EXECUTE ON FUNCTION fn_purge_p02(uuid), fn_entitas_nonaktif_pertama(uuid) TO pr_api;

-- ================================================================ pr_worker
GRANT SELECT ON downstream_endpoint, downstream_pool, downstream_pool_rute, upstream_pool, provider, provider_config_versi,
  endpoint_config_versi, endpoint_metadata_field, provider_secret TO pr_worker;
-- 8.1b: status columns only, no credential hash columns
GRANT SELECT (client_id, aktif) ON client TO pr_worker;
GRANT SELECT (credential_id, client_id, status) ON client_kredensial TO pr_worker;
GRANT SELECT (client_id, aktif) ON client_webhook TO pr_worker;
DO $$ BEGIN
  EXECUTE format('GRANT SELECT (%s) ON request TO pr_worker', pg_temp.kolom('request', ARRAY['output_payload']));
END $$;
GRANT UPDATE (status, upstream_pool_id, first_attempt_at, completed_at, attempt_count, final_kode_transaksi, output_payload,
  sumber_hasil, failure_reason, cancel_reason, cancelled_by, not_before, webhook_status) ON request TO pr_worker;
GRANT SELECT, INSERT ON request_status_history TO pr_worker;
GRANT SELECT, INSERT, UPDATE ON transaction_log TO pr_worker;
GRANT SELECT ON upstream_attempt_daily TO pr_worker;
GRANT EXECUTE ON FUNCTION fn_purge_p02(uuid), fn_entitas_nonaktif_pertama(uuid) TO pr_worker;   -- gate to cancelled (8.1b)

-- ================================================================ pr_controller (dispatcher, sweepers, reconciler, jobs)
GRANT SELECT ON downstream_endpoint, downstream_pool, downstream_pool_rute, upstream_pool, provider TO pr_controller;
GRANT SELECT (client_id, aktif) ON client TO pr_controller;
GRANT SELECT (credential_id, client_id, status) ON client_kredensial TO pr_controller;
GRANT SELECT (client_id, aktif) ON client_webhook TO pr_controller;
DO $$ BEGIN
  EXECUTE format('GRANT SELECT (%s) ON request TO pr_controller', pg_temp.kolom('request', ARRAY['input_payload', 'output_payload', 'metadata']));
END $$;
GRANT UPDATE (status, upstream_pool_id, prioritas_efektif, dispatched_at, not_before, completed_at, attempt_count,
  final_kode_transaksi, failure_reason, cancel_reason, cancelled_by, webhook_status) ON request TO pr_controller;
GRANT SELECT, INSERT ON request_status_history TO pr_controller;
DO $$ BEGIN
  EXECUTE format('GRANT SELECT (%s) ON transaction_log TO pr_controller', pg_temp.kolom('transaction_log', ARRAY['nik', 'request_payload', 'response_payload']));
END $$;
GRANT UPDATE (archived_at) ON transaction_log TO pr_controller;
GRANT SELECT, INSERT, UPDATE ON upstream_attempt_daily TO pr_controller;
GRANT EXECUTE ON FUNCTION fn_purge_p02(uuid), fn_purge_p03(integer), fn_tutup_log_tidak_diketahui(uuid, integer),
  fn_entitas_nonaktif_pertama(uuid), fn_ditahan_data_balikan(uuid) TO pr_controller;

-- ================================================================ pr_webhook (webhook job in the controller)
GRANT SELECT (request_id, webhook_status, webhook_attempt_count) ON request TO pr_webhook;   -- outbox scan + CAS claim 0 -> 1
GRANT SELECT (client_id, status, attempt_count) ON request TO pr_webhook;                    -- body: owner, terminal status, attempt_no
GRANT UPDATE (webhook_status, webhook_attempt_count, webhook_last_at, webhook_last_http_status, webhook_last_error) ON request TO pr_webhook;
GRANT SELECT ON v_request_webhook TO pr_webhook;                       -- terminal async rows, output_payload, no input / metadata
GRANT INSERT ON webhook_log TO pr_webhook;
-- W-02: exactly eight columns of client_webhook, plus the allowlist
GRANT SELECT (client_id, webhook_url, aktif, secret_versi, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id) ON client_webhook TO pr_webhook;
GRANT SELECT ON webhook_domain_allowlist TO pr_webhook;
GRANT SELECT (kunci, nilai) ON konfigurasi_platform TO pr_webhook;  -- WB2a retry policy

-- ================================================================ pr_admin_api (admin API for the Web UI)
-- Entity status flags (aktif) change only through fn_nonaktifkan_entitas / fn_aktifkan_entitas (audit + mass cancel).
GRANT SELECT, INSERT ON client, provider, upstream_pool, downstream_pool, downstream_endpoint TO pr_admin_api;
DO $$ BEGIN
  EXECUTE format('GRANT UPDATE (%s) ON client TO pr_admin_api', pg_temp.kolom('client', ARRAY['client_id', 'aktif']));
  EXECUTE format('GRANT UPDATE (%s) ON provider TO pr_admin_api', pg_temp.kolom('provider', ARRAY['provider_id', 'aktif']));
  EXECUTE format('GRANT UPDATE (%s) ON upstream_pool TO pr_admin_api', pg_temp.kolom('upstream_pool', ARRAY['upstream_pool_id', 'aktif']));
  EXECUTE format('GRANT UPDATE (%s) ON downstream_pool TO pr_admin_api', pg_temp.kolom('downstream_pool', ARRAY['downstream_pool_id', 'aktif']));
  EXECUTE format('GRANT UPDATE (%s) ON downstream_endpoint TO pr_admin_api', pg_temp.kolom('downstream_endpoint', ARRAY['downstream_endpoint_id', 'aktif']));
END $$;
GRANT SELECT, INSERT, UPDATE, DELETE ON downstream_pool_rute, client_endpoint, endpoint_metadata_field TO pr_admin_api;
GRANT SELECT, INSERT ON provider_config_versi, endpoint_config_versi TO pr_admin_api;
-- Credentials: hash is write-only; status / dicabut_* only through fn_set_status_kredensial (EN-03).
-- fn_set_status_kredensial is NOT granted to pr_admin_api: the only sanctioned path is
-- fn_nonaktifkan_entitas('kredensial', ...), which checks client ownership, writes audit_konfigurasi and cancels
-- waiting requests (EN-03, EN-08, DQ-37 e-f). A direct call would skip all three, so the grant is revoked.
GRANT SELECT (credential_id, client_id, jenis, status, dicabut_at, dicabut_oleh, kedaluwarsa_at, dibuat_at, dibuat_oleh) ON client_kredensial TO pr_admin_api;
GRANT INSERT ON client_kredensial TO pr_admin_api;
GRANT UPDATE (kunci_hash, kedaluwarsa_at) ON client_kredensial TO pr_admin_api;
-- Webhook config: secret columns write-only (W-03)
GRANT SELECT (client_id, webhook_url, aktif, secret_versi, kek_id, diubah_at, diubah_oleh) ON client_webhook TO pr_admin_api;
GRANT INSERT ON client_webhook TO pr_admin_api;
GRANT UPDATE (webhook_url, aktif, secret_versi, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, diubah_at, diubah_oleh) ON client_webhook TO pr_admin_api;
-- Provider secrets: write-only
GRANT SELECT (provider_id, nama_secret, secret_versi, diubah_at, diubah_oleh) ON provider_secret TO pr_admin_api;
GRANT INSERT ON provider_secret TO pr_admin_api;
GRANT UPDATE (secret_versi, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, diubah_at, diubah_oleh) ON provider_secret TO pr_admin_api;
GRANT SELECT, INSERT, UPDATE ON webhook_domain_allowlist, konfigurasi_platform TO pr_admin_api;
GRANT SELECT, INSERT, UPDATE ON admin_user, admin_user_peran TO pr_admin_api;
GRANT SELECT, INSERT, UPDATE, DELETE ON admin_session TO pr_admin_api;
GRANT SELECT, INSERT ON audit_konfigurasi TO pr_admin_api;
-- audit_akses: INSERT only; reading goes through fn_lihat_audit (E-06)
GRANT INSERT ON audit_akses TO pr_admin_api;
-- audit_login (ADR-0003 3.6): written through fn_catat_login (SECURITY DEFINER) so the id is a UUIDv7; the table
-- itself is not writable by the role and not readable by anyone.
GRANT EXECUTE ON FUNCTION fn_catat_login(uuid, text, text, text, text, text) TO pr_admin_api;
GRANT SELECT ON upstream_attempt_daily TO pr_admin_api;               -- usage and quota aggregates, no NIK
GRANT SELECT ON v_request_tersamar, v_transaction_log_tersamar TO pr_admin_api;
GRANT EXECUTE ON FUNCTION fn_nonaktifkan_entitas(text, uuid, uuid, text),
  fn_aktifkan_entitas(text, uuid, uuid, text), fn_pratinjau_nonaktif(text, uuid), fn_request_milik_client(uuid),
  fn_webhook_log_milik_client(uuid), fn_auditor_lihat_transaksi(uuid, text, uuid, text, text),
  fn_auditor_cari_nik(uuid, text, text, timestamptz, timestamptz, text, text, text),
  fn_ekspor_log_auditor(uuid, uuid, text, text, text, timestamptz, timestamptz, uuid, text, text, text),
  fn_lihat_audit(uuid, timestamptz, timestamptz, integer, text, text), fn_punya_peran(uuid, text)
  TO pr_admin_api;
-- Explicit: no direct credential-revocation path. Reached only from fn_nonaktifkan_entitas (SECURITY DEFINER).
REVOKE ALL ON FUNCTION fn_set_status_kredensial(uuid, text, uuid, text) FROM PUBLIC;

-- ================================================================ defaults for later migrations
-- Every function above revokes EXECUTE from PUBLIC by hand. Make that the default instead of a habit: a function
-- added by a later migration (created by poolrouter_owner) must not be executable by PUBLIC either. Grants stay
-- explicit and per role, as for the functions above.
ALTER DEFAULT PRIVILEGES FOR ROLE poolrouter_owner IN SCHEMA public REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;

-- ---------------------------------------------------------------- webhook claim (ARC-73, AC-26)
-- `pr_webhook` owns the five webhook_* columns of `request`, but a plain column UPDATE still fails: the
-- BEFORE UPDATE triggers (request_transisi, request_history_wajib, ...) read columns the role is deliberately
-- not granted (W-02). The claim therefore goes through one SECURITY DEFINER function that touches only those
-- five columns; EXECUTE is granted to pr_webhook alone right here.
CREATE FUNCTION fn_klaim_webhook(
  p_request_id uuid,
  p_sekarang timestamptz,
  p_http_status integer,
  p_galat text,
  p_diharapkan integer DEFAULT 0)
RETURNS integer
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
  n integer;
BEGIN
  UPDATE public.request
     SET webhook_attempt_count = p_diharapkan + 1,
         webhook_last_at = p_sekarang,
         webhook_not_before = NULL,
         webhook_status = CASE WHEN p_galat IS NULL THEN webhook_status ELSE 'gagal' END,
         webhook_last_http_status = p_http_status,
         webhook_last_error = p_galat
   WHERE request_id = p_request_id
     AND webhook_status = 'menunggu'
     AND webhook_attempt_count = p_diharapkan
     AND (webhook_not_before IS NULL OR webhook_not_before <= p_sekarang)
     AND (p_galat IS NULL OR webhook_last_at < p_sekarang - interval '5 minutes');
  GET DIAGNOSTICS n = ROW_COUNT;
  RETURN n;
END $$;
REVOKE ALL ON FUNCTION fn_klaim_webhook(uuid, timestamptz, integer, text, integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION fn_klaim_webhook(uuid, timestamptz, integer, text, integer) TO pr_webhook;

-- AC-26: a claim older than the given limit is closed `gagal` without a second send. The age test lives
-- inside the function: the outbox scan only runs on columns pr_webhook may read, and comparing
-- webhook_last_at to a bound timestamp in the scan fails with "permission denied for table request"
-- (most likely a plan-time permission check on the column's index) even though the column is granted.
CREATE FUNCTION fn_tutup_klaim_macet(p_sekarang timestamptz, p_detik integer)
RETURNS bigint
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
  n bigint;
BEGIN
  UPDATE public.request
     SET webhook_status = 'gagal',
         webhook_last_error = 'klaim_kedaluwarsa'
   WHERE webhook_status = 'menunggu'
     AND webhook_attempt_count >= 1
     AND webhook_not_before IS NULL
     AND webhook_last_at < p_sekarang - make_interval(secs => p_detik);
  GET DIAGNOSTICS n = ROW_COUNT;
  RETURN n;
END $$;
REVOKE ALL ON FUNCTION fn_tutup_klaim_macet(timestamptz, integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION fn_tutup_klaim_macet(timestamptz, integer) TO pr_webhook;

