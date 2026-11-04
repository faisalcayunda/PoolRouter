-- F1 schema, part 4: grants for the new objects and the three new job roles (SHAA-333).
-- Source: data contract v0.9.5-final F1-5.4, F1-6.5, F1-7.4 MF-09, F1-9 summary table, WR-07.
-- Roles pr_job_balikan, pr_job_arsip and pr_restore come from backend/db/bootstrap.sql (cluster-wide, not a migration;
-- it also gives them CONNECT and USAGE on schema public, so re-run the bootstrap before this migration).
-- F0 column grants were generated from the F0 column list, so new columns on request / transaction_log need explicit grants.
-- Nobody gets DELETE on a new table. Operator (a person role) has no database role at all (AR-08).

-- Temporary helper (session-local, as in the F0 grants migration): all columns of a table except the listed ones.
CREATE OR REPLACE FUNCTION pg_temp.kolom(p_tabel text, p_kecuali text[] DEFAULT ARRAY[]::text[]) RETURNS text
LANGUAGE sql AS $$
  SELECT string_agg(quote_ident(column_name), ', ' ORDER BY ordinal_position)
    FROM information_schema.columns
   WHERE table_schema = 'public' AND table_name = p_tabel AND column_name <> ALL (p_kecuali);
$$;

-- ================================================================ existing roles, new columns
-- webhook_not_before is deliberately absent here: WR-07 / WR-08 (only pr_webhook reads it, never the client owner).
GRANT SELECT (aging_ambang_detik, prioritas_diubah_at, reject_lingkup) ON request TO pr_api, pr_worker, pr_controller;
GRANT SELECT (sumber_anggaran, downstream_pool_id) ON transaction_log TO pr_api, pr_controller;   -- pr_worker has table-level SELECT
-- D1: the controller climbs priority and writes the aging history row (history INSERT is already granted).
GRANT UPDATE (prioritas_efektif, prioritas_diubah_at) ON request TO pr_controller;
-- D3 / WR-07: only pr_webhook reads and writes webhook_not_before.
GRANT SELECT (webhook_not_before) ON request TO pr_webhook;
GRANT UPDATE (webhook_not_before) ON request TO pr_webhook;
-- Admin API changes the per-pool aging threshold (audited by the API).
GRANT UPDATE (aging_ambang_detik) ON downstream_pool TO pr_admin_api;

-- D6: job arsip is a separate role now (contract 8.1 row replaced); the controller no longer sets archived_at.
REVOKE UPDATE (archived_at) ON transaction_log FROM pr_controller;

-- ================================================================ D7 / MF-09: only the admin API (the CLI runs through it, ADR-0003 3.6c)
GRANT SELECT, INSERT, UPDATE, DELETE ON pengguna_mfa, kode_pemulihan_mfa, kode_daftar_mfa TO pr_admin_api;

-- ================================================================ pr_job_balikan (D4, F1-5.4)
-- Reads what composing a batch needs, including the NIK (items carry it); no payload outputs, no secrets, no
-- audit_akses, no client_webhook, no DELETE. Provider secrets are resolved by the worker connection, not here.
DO $$ BEGIN
  EXECUTE format('GRANT SELECT (%s) ON request TO pr_job_balikan', pg_temp.kolom('request', ARRAY['output_payload']));
  EXECUTE format('GRANT SELECT (%s) ON transaction_log TO pr_job_balikan', pg_temp.kolom('transaction_log', ARRAY['request_payload', 'response_payload']));
END $$;
GRANT SELECT ON provider, provider_config_versi, endpoint_metadata_field, downstream_endpoint TO pr_job_balikan;
GRANT SELECT, INSERT, UPDATE ON data_balikan_batch, data_balikan_item TO pr_job_balikan;

-- ================================================================ pr_job_arsip (D6, F1-6.5, AR-02c)
-- Reads the partitions it archives (Rahasia columns included), writes the manifest and archived_at. No DELETE: the
-- online delete is a SECURITY DEFINER function owned by the D6 implementation issue (AR-04).
GRANT SELECT ON request, request_status_history, transaction_log, webhook_log, audit_akses TO pr_job_arsip;
GRANT SELECT, INSERT, UPDATE ON arsip_manifest TO pr_job_arsip;
GRANT UPDATE (archived_at) ON transaction_log TO pr_job_arsip;

-- ================================================================ pr_restore (D6, F1-6.5, AR-02c)
-- Reads manifests and requests, moves a request through its states, fills arsip_pulih. No SELECT on arsip_pulih:
-- expiry goes through fn_arsip_pulih_kedaluwarsa. Audit: INSERT only (the restore_arsip row).
GRANT USAGE ON SCHEMA arsip_pulih TO pr_restore;
GRANT SELECT ON arsip_manifest, arsip_restore_permintaan TO pr_restore;
GRANT UPDATE (status, mulai_at, siap_at, kedaluwarsa_at, jumlah_baris_pulih, error_terakhir) ON arsip_restore_permintaan TO pr_restore;
GRANT INSERT, DELETE ON arsip_pulih.transaction_log TO pr_restore;
GRANT INSERT ON audit_akses TO pr_restore;
GRANT EXECUTE ON FUNCTION fn_arsip_pulih_kedaluwarsa(uuid) TO pr_restore;

-- The admin API creates restore requests (together with the minta_restore audit row, same transaction).
-- Column grant only: enough to find the HMAC key version of the objects in a range (nik_hmac_per_versi); never dek_terbungkus (Rahasia, AR-02c).
GRANT SELECT (manifest_id, tabel_sumber, provider_id, tanggal_wib, kunci_hmac_versi, status) ON arsip_manifest TO pr_admin_api;
GRANT SELECT, INSERT ON arsip_restore_permintaan TO pr_admin_api;
GRANT EXECUTE ON FUNCTION fn_alokasi_kelebihan(uuid), fn_konfigurasi_int(text, integer) TO pr_admin_api;

-- ================================================================ lock down function execution (again)
-- The F0 migration ends with ALTER DEFAULT PRIVILEGES ... IN SCHEMA public REVOKE EXECUTE ... FROM PUBLIC. A
-- schema-level default cannot subtract the built-in global "EXECUTE to PUBLIC", so it does not take effect; functions
-- added after F0 stayed executable by PUBLIC (found while testing F1, reported in the PR). Fix it here for good:
-- the global default for poolrouter_owner, then revoke on every function that exists now. F0 files are not edited.
ALTER DEFAULT PRIVILEGES FOR ROLE poolrouter_owner REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;
DO $$
DECLARE
  f record;
BEGIN
  FOR f IN SELECT p.oid::regprocedure AS sig FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
            WHERE n.nspname IN ('public', 'arsip_pulih') AND p.prokind = 'f'
              AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.objid = p.oid AND d.deptype = 'e')
  LOOP
    EXECUTE format('REVOKE ALL ON FUNCTION %s FROM PUBLIC', f.sig);
  END LOOP;
END $$;
