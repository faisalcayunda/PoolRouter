-- Rollback of 20261104000003_f1_mfa.sql (SHAA-333). Drops the three MFA tables: refuses when they hold rows unless
--   SET poolrouter.izinkan_hapus_data_f1 = 'ya';   (deletion of production data needs Klaus + owner approval)
-- Sessions revoked by the up migration (MF-07) are not restored; those users simply log in again.
DO $$
BEGIN
  IF (SELECT count(*) FROM pengguna_mfa) + (SELECT count(*) FROM kode_pemulihan_mfa) + (SELECT count(*) FROM kode_daftar_mfa) > 0
     AND coalesce(current_setting('poolrouter.izinkan_hapus_data_f1', true), '') <> 'ya' THEN
    RAISE EXCEPTION 'rollback ditolak: tabel MFA F1 berisi data (set poolrouter.izinkan_hapus_data_f1 = ya setelah persetujuan Klaus dan owner)';
  END IF;
END $$;
DROP TABLE kode_daftar_mfa, kode_pemulihan_mfa, pengguna_mfa;
DROP FUNCTION tg_mfa_langkah_naik(), tg_kode_pemulihan_sekali(), tg_kode_daftar_beku();
ALTER TABLE admin_session DROP COLUMN mfa_terverifikasi_at;
