-- Rollback 20261108000001_f1_penanda_data_balikan.sql (ARC-199). Jalankan setelah rollback 20261108000002.
-- Penanda bisa dihitung ulang dari data_balikan_item aktif, jadi kolomnya boleh dibuang tanpa kehilangan data.
-- Penyusun yang sudah memakai penanda (ARC-181) harus di-rollback lebih dulu.
REVOKE SELECT (data_balikan_tersusun), UPDATE (data_balikan_tersusun) ON request FROM pr_job_balikan;
DROP STATISTICS IF EXISTS request_data_balikan_stat;
ALTER TABLE request DROP COLUMN data_balikan_tersusun;
