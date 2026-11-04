-- Rollback of 20261104000002_f1_audit_balikan_arsip.sql (SHAA-333). Refuses when F1 data exists unless
--   SET poolrouter.izinkan_hapus_data_f1 = 'ya';   (needs Klaus + owner approval)
DO $$
DECLARE
  v_ada bigint;
BEGIN
  SELECT (SELECT count(*) FROM arsip_manifest) + (SELECT count(*) FROM arsip_restore_permintaan)
       + (SELECT count(*) FROM arsip_pulih.transaction_log)
       + (SELECT count(*) FROM audit_akses WHERE aksi IN ('minta_restore', 'terbit_kode_daftar', 'daftar_mfa', 'bootstrap_mfa',
            'pakai_kode_pemulihan', 'reset_mfa', 'reset_mfa_darurat') OR sumber <> 'online' OR target_pengguna_id IS NOT NULL
            OR pelaku_kedua_id IS NOT NULL OR target_restore_id IS NOT NULL OR kunci_hmac_versi IS NOT NULL)
       + (SELECT count(*) FROM provider_config_versi WHERE config_schema_version <> 1)
       + (SELECT count(*) FROM data_balikan_item WHERE NOT aktif)
       + (SELECT count(*) FROM data_balikan_batch WHERE kirim_berikutnya_at IS NOT NULL)
    INTO v_ada;
  IF v_ada > 0 AND coalesce(current_setting('poolrouter.izinkan_hapus_data_f1', true), '') <> 'ya' THEN
    RAISE EXCEPTION 'rollback ditolak: % baris data F1 (set poolrouter.izinkan_hapus_data_f1 = ya setelah persetujuan Klaus dan owner)', v_ada;
  END IF;
END $$;

DROP FUNCTION fn_arsip_pulih_kedaluwarsa(uuid);
DROP SCHEMA arsip_pulih CASCADE;
DROP TABLE arsip_restore_permintaan;
DROP FUNCTION tg_restore_rentang(), tg_restore_punya_audit(), tg_restore_identitas();
DROP TABLE arsip_manifest;

DROP TRIGGER data_balikan_item_sesuai_batch ON data_balikan_item;
DROP TRIGGER data_balikan_batch_gagal_permanen ON data_balikan_batch;
DROP FUNCTION tg_balikan_item_sesuai_batch(), tg_balikan_batch_gagal_permanen();
DROP INDEX data_balikan_item_aktif_uq;
ALTER TABLE data_balikan_item DROP COLUMN aktif;
DROP INDEX data_balikan_batch_kirim_idx;
ALTER TABLE data_balikan_batch DROP COLUMN kirim_berikutnya_at, DROP COLUMN config_ref_version;   -- takes chk_* with them
ALTER TABLE provider_config_versi DROP CONSTRAINT chk_config_schema_version;
ALTER TABLE provider_config_versi ADD CONSTRAINT provider_config_versi_config_schema_version_check CHECK (config_schema_version = 1);

DROP INDEX audit_akses_kode_idx, audit_akses_target_pengguna_idx;
ALTER TABLE audit_akses DROP CONSTRAINT chk_aksi, DROP CONSTRAINT chk_alasan_wajib, DROP CONSTRAINT chk_audit_kode_id;
ALTER TABLE audit_akses DROP COLUMN sumber, DROP COLUMN target_pengguna_id, DROP COLUMN pelaku_kedua_id,
  DROP COLUMN target_restore_id, DROP COLUMN kunci_hmac_versi;                       -- takes the F1 chk_audit_* with them
ALTER TABLE audit_akses ADD CONSTRAINT audit_akses_aksi_check CHECK (aksi IN ('lihat_log', 'cari_log', 'cari_nik', 'lihat_payload', 'ekspor',
  'lihat_audit', 'restore_arsip', 'ubah_peran', 'akses_ditolak'));
ALTER TABLE audit_akses ADD CONSTRAINT chk_alasan_wajib CHECK (
  (tampilan_nik <> 'utuh' AND aksi NOT IN ('lihat_payload', 'ekspor', 'restore_arsip')) OR length(btrim(coalesce(alasan, ''))) > 0);
