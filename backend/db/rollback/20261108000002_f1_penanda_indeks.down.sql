-- no-transaction
-- Rollback 20261108000002_f1_penanda_indeks.sql (ARC-199). Jalankan di luar transaksi (tanpa `psql -1`),
-- sebelum rollback 20261108000001. Juga dipakai untuk membuang indeks INVALID sisa CONCURRENTLY yang gagal.
DROP INDEX CONCURRENTLY IF EXISTS request_data_balikan_tertunda_idx;
