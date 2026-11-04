-- Rollback of 20261104000001_f1_antrean_alokasi_retry.sql (SHAA-333). Refuses when F1 data exists unless
--   SET poolrouter.izinkan_hapus_data_f1 = 'ya';   (needs Klaus + owner approval)
DO $$
DECLARE
  v_ada bigint;
BEGIN
  SELECT (SELECT count(*) FROM request WHERE aging_ambang_detik IS NOT NULL OR prioritas_diubah_at IS NOT NULL
                                              OR reject_lingkup IS NOT NULL OR webhook_not_before IS NOT NULL)
       + (SELECT count(*) FROM request_status_history WHERE prioritas_dari IS NOT NULL)
       + (SELECT count(*) FROM transaction_log WHERE sumber_anggaran IS NOT NULL OR downstream_pool_id IS NOT NULL)
       + (SELECT count(*) FROM downstream_pool_rute WHERE alokasi_jenis IS NOT NULL OR NOT boleh_pinjam)
    INTO v_ada;
  IF v_ada > 0 AND coalesce(current_setting('poolrouter.izinkan_hapus_data_f1', true), '') <> 'ya' THEN
    RAISE EXCEPTION 'rollback ditolak: % baris data F1 (set poolrouter.izinkan_hapus_data_f1 = ya setelah persetujuan Klaus dan owner)', v_ada;
  END IF;
END $$;

DROP TRIGGER downstream_pool_prioritas ON downstream_pool;
DROP FUNCTION tg_downstream_pool_prioritas();
DROP TRIGGER konfigurasi_platform_f1 ON konfigurasi_platform;
DELETE FROM konfigurasi_platform WHERE kunci IN ('prioritas_maks', 'webhook_batas_percobaan', 'webhook_backoff_base_detik',
  'webhook_backoff_cap_detik', 'webhook_jitter_persen', 'restore_masa_berlaku_hari', 'restore_rentang_dengan_nik_maks_bulan',
  'restore_rentang_tanpa_nik_maks_hari');
DROP FUNCTION tg_konfigurasi_f1(), fn_konfigurasi_int(text, integer);

-- tg_log_update: back to the F0 body.
CREATE OR REPLACE FUNCTION tg_log_update() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.status_akses IS NOT NULL THEN
    IF (to_jsonb(NEW) - 'archived_at') IS DISTINCT FROM (to_jsonb(OLD) - 'archived_at') THEN
      RAISE EXCEPTION 'baris transaction_log yang lengkap tidak berubah (R-03)' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
  ELSIF NEW.kode_transaksi <> OLD.kode_transaksi OR NEW.request_id <> OLD.request_id OR NEW.attempt_no <> OLD.attempt_no
        OR NEW.nik IS DISTINCT FROM OLD.nik OR NEW.requested_at <> OLD.requested_at
        OR NEW.upstream_pool_id <> OLD.upstream_pool_id OR NEW.provider_id <> OLD.provider_id THEN
    RAISE EXCEPTION 'kolom identitas transaction_log tidak boleh diubah (R-03)' USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NEW;
END $$;
DROP TRIGGER transaction_log_f1_insert ON transaction_log;
DROP FUNCTION tg_log_f1_insert();
DROP INDEX transaction_log_sumber_idx;
ALTER TABLE transaction_log DROP COLUMN sumber_anggaran, DROP COLUMN downstream_pool_id;

DROP TRIGGER downstream_pool_rute_alokasi ON downstream_pool_rute;
DROP TRIGGER upstream_pool_alokasi ON upstream_pool;
DROP TRIGGER downstream_pool_alokasi ON downstream_pool;
DROP FUNCTION tg_alokasi_dalam_kuota(), fn_alokasi_kelebihan(uuid);
ALTER TABLE downstream_pool_rute DROP COLUMN alokasi_jenis, DROP COLUMN alokasi_nilai, DROP COLUMN sifat, DROP COLUMN boleh_pinjam;

DROP TRIGGER request_aging_history_wajib ON request;
DROP TRIGGER request_f1_guard ON request;
DROP FUNCTION tg_request_aging_punya_history(), tg_request_f1_guard();
ALTER TABLE request_status_history DROP COLUMN prioritas_dari, DROP COLUMN prioritas_ke;
DROP INDEX request_webhook_retry_idx, request_aging_idx;
ALTER TABLE request DROP CONSTRAINT chk_prioritas_efektif_le_prioritas;
ALTER TABLE request DROP COLUMN aging_ambang_detik, DROP COLUMN prioritas_diubah_at, DROP COLUMN reject_lingkup, DROP COLUMN webhook_not_before;
ALTER TABLE downstream_pool DROP COLUMN aging_ambang_detik;
