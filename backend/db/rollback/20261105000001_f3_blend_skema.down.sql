-- Rollback of 20261105000001_f3_blend_skema.sql (SHAA-354). Run as poolrouter_owner, after 20261105000002.down.
-- Refuses while F3 data exists: any blend, blend configuration, gate event, or a child request.
-- The original F0 provider markers and the 'migrasi' audit rows are not F3 data (they are produced by the migration itself).
DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM blend) OR EXISTS (SELECT 1 FROM endpoint_blend) OR EXISTS (SELECT 1 FROM audit_gerbang_blend)
     OR EXISTS (SELECT 1 FROM request WHERE blend_id IS NOT NULL) THEN
    RAISE EXCEPTION 'rollback F3 ditolak: data blend, konfigurasi blend, peristiwa gerbang atau anak blend masih ada';
  END IF;
  IF EXISTS (SELECT 1 FROM audit_konfigurasi WHERE aktor = 'pengguna' AND entitas_jenis IN ('endpoint_blend', 'provider_penanda_blend'))
     OR EXISTS (SELECT 1 FROM audit_akses WHERE target_blend_id IS NOT NULL) THEN
    RAISE EXCEPTION 'rollback F3 ditolak: audit F3 yang ditulis pengguna masih ada (append-only, tidak boleh dihapus)';
  END IF;
END $$;
DROP FUNCTION fn_blend_hapus_online(timestamptz), fn_gerbang_blend_catat(boolean, text, text, text);
DROP TABLE audit_gerbang_blend, blend_webhook_log, blend_status_history;
ALTER TABLE request DROP CONSTRAINT chk_request_anak_blend, DROP CONSTRAINT fk_request_blend;
DROP INDEX request_blend_komponen_uq;
DROP TRIGGER request_f3_guard ON request;
DROP TRIGGER request_anak_blend ON request;
ALTER TABLE request DROP COLUMN blend_id;
DROP TABLE blend;
DROP TABLE endpoint_blend_komponen, endpoint_blend_versi CASCADE;
DROP TABLE endpoint_blend;
DROP FUNCTION fn_ringkasan_komponen_valid(jsonb), tg_endpoint_blend_baru_nonaktif(), tg_blend_versi_min_dua_komponen(),
  tg_blend_transisi(), tg_blend_append_only(), tg_blend_punya_history(), tg_request_f3_guard(), tg_request_anak_blend(),
  tg_blend_anak_lengkap(), tg_blend_terminal_konsisten();
ALTER TABLE audit_akses DROP COLUMN target_blend_id;
-- audit_konfigurasi: the append-only trigger refuses DELETE, so disable it for the removal of the migration's own rows only.
ALTER TABLE audit_konfigurasi DISABLE TRIGGER audit_konfigurasi_append_only;
DELETE FROM audit_konfigurasi WHERE aktor = 'migrasi';
ALTER TABLE audit_konfigurasi ENABLE TRIGGER audit_konfigurasi_append_only;
ALTER TABLE audit_konfigurasi
  DROP CONSTRAINT chk_audit_penanda_naik, DROP CONSTRAINT chk_audit_penanda_turun, DROP CONSTRAINT chk_audit_konfig_gerbang_entitas,
  DROP CONSTRAINT chk_audit_konfig_gerbang_ditolak, DROP CONSTRAINT chk_audit_konfig_aktor_pengguna, DROP CONSTRAINT chk_audit_konfig_entitas_jenis,
  DROP COLUMN referensi_persetujuan_tanggal, DROP COLUMN referensi_persetujuan_nomor, DROP COLUMN blend_terdampak,
  DROP COLUMN gerbang, DROP COLUMN aktor;
ALTER TABLE audit_konfigurasi ADD CONSTRAINT audit_konfigurasi_entitas_jenis_check CHECK (entitas_jenis IN (
  'client', 'kredensial', 'downstream_endpoint', 'downstream_pool', 'upstream_pool', 'provider',
  'client_webhook', 'webhook_domain_allowlist', 'provider_config', 'endpoint_config',
  'endpoint_metadata_field', 'konfigurasi_platform', 'admin_user'));
ALTER TABLE audit_konfigurasi ALTER COLUMN pengguna_id SET NOT NULL;
ALTER TABLE provider DROP COLUMN butuh_dasar_hukum_blend;
