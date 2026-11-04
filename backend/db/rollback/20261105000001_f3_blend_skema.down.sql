-- Rollback of 20261105000001_f3_blend_skema.sql (SHAA-354). Run as poolrouter_owner, after the grants rollback.
-- Refuses when any F3 data exists unless
--   SET poolrouter.izinkan_hapus_data_f3 = 'ya';   (deleting production data needs Klaus + owner approval)
-- The migration entries of provider_penanda_audit (aktor = migrasi) are not data in this sense. Children of a blend keep their
-- request rows: they are ordinary F0 requests once blend_id is dropped.
DO $$
BEGIN
  IF (SELECT count(*) FROM blend) + (SELECT count(*) FROM endpoint_blend) + (SELECT count(*) FROM gerbang_blend_peristiwa)
     + (SELECT count(*) FROM provider_penanda_audit WHERE aktor <> 'migrasi') + (SELECT count(*) FROM request WHERE blend_id IS NOT NULL)
     + (SELECT count(*) FROM audit_akses WHERE target_blend_id IS NOT NULL) + (SELECT count(*) FROM audit_konfigurasi WHERE entitas_jenis = 'endpoint_blend') > 0
     AND coalesce(current_setting('poolrouter.izinkan_hapus_data_f3', true), '') <> 'ya' THEN
    RAISE EXCEPTION 'rollback ditolak: objek F3 berisi data (set poolrouter.izinkan_hapus_data_f3 = ya setelah persetujuan Klaus dan owner)';
  END IF;
END $$;

-- back to the F0/F1 definition of P-03 (no BP-05 predicate)
CREATE OR REPLACE FUNCTION fn_purge_p03(p_batas integer DEFAULT 1000) RETURNS integer
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_n integer;
BEGIN
  WITH kandidat AS (
    SELECT r.request_id
      FROM request r
      JOIN transaction_log l ON l.kode_transaksi = r.final_kode_transaksi
      LEFT JOIN downstream_pool d ON d.downstream_pool_id = r.downstream_pool_id
      WHERE r.status IN ('done', 'failed') AND r.output_purged_at IS NULL AND l.log_mode = 'metadata_only'
       AND r.completed_at + CASE WHEN r.mode = 'sync' THEN interval '0' ELSE make_interval(secs => coalesce(d.ttl_antrean_detik, 0)) END <= now()
       AND NOT fn_ditahan_data_balikan(r.request_id)
     ORDER BY r.completed_at
     LIMIT p_batas
     FOR UPDATE OF r SKIP LOCKED)
  UPDATE request r
     SET input_payload = NULL, output_payload = NULL, output_purged_at = now(),
         metadata = fn_metadata_tanpa_rahasia(r.downstream_endpoint_id, r.metadata),
         payload_purged_at = now()
    FROM kandidat k WHERE r.request_id = k.request_id;
  GET DIAGNOSTICS v_n = ROW_COUNT;
  RETURN v_n;
END $$;
-- CREATE OR REPLACE keeps the grants of the function.

DROP TRIGGER request_blend_guard ON request;
DROP TRIGGER request_anak_blend ON request;
DROP TRIGGER webhook_log_bukan_anak ON webhook_log;
DROP FUNCTION tg_request_blend_guard(), tg_request_anak_blend(), tg_webhook_log_bukan_anak();

ALTER TABLE request DROP CONSTRAINT chk_anak_blend_idem, DROP CONSTRAINT chk_anak_blend_webhook;
DROP INDEX request_blend_idx, request_blend_endpoint_uq;
ALTER TABLE request DROP COLUMN blend_id;
ALTER TABLE audit_akses DROP COLUMN target_blend_id;

ALTER TABLE audit_konfigurasi DROP CONSTRAINT audit_konfigurasi_entitas_jenis_check;
ALTER TABLE audit_konfigurasi ADD CONSTRAINT audit_konfigurasi_entitas_jenis_check CHECK (entitas_jenis IN (
  'client', 'kredensial', 'downstream_endpoint', 'downstream_pool', 'upstream_pool', 'provider',
  'client_webhook', 'webhook_domain_allowlist', 'provider_config', 'endpoint_config',
  'endpoint_metadata_field', 'konfigurasi_platform', 'admin_user'));

DROP TABLE blend_webhook_log, blend_status_history, blend;
DROP TABLE gerbang_blend_peristiwa;
DROP TRIGGER provider_penanda_guard ON provider;
DROP TABLE provider_penanda_audit;
ALTER TABLE provider DROP COLUMN butuh_dasar_hukum_blend;
ALTER TABLE endpoint_blend DROP CONSTRAINT endpoint_blend_versi_fk;
DROP TABLE endpoint_blend_komponen, endpoint_blend_versi, endpoint_blend;
DROP FUNCTION tg_blend_transisi(), tg_blend_punya_history(), tg_blend_jumlah_anak(), tg_blend_terminal_konsisten(),
  tg_gerbang_blend_peristiwa(), fn_catat_gerbang_blend(text, text, text), fn_purge_blend_payload(integer),
  tg_provider_penanda_guard(), tg_endpoint_blend_aktivasi(), tg_blend_komponen_beku(), tg_blend_versi_min_komponen();
