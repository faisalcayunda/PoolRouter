-- no-transaction
-- F1 data balikan: indeks parsial request tertunda untuk penyusun (ARC-199, R1).
--
-- Baris pertama `-- no-transaction` wajib: CREATE INDEX CONCURRENTLY tidak bisa berjalan di dalam blok
-- transaksi, termasuk blok implisit dari kueri multi-statement. Karena itu berkas ini berisi tepat satu
-- statement. Penerap uji (backend/db/test/run.sh, uji-pg.sh, fixtures/k20-load.sh) menerapkannya di luar
-- `psql -1`.
--
-- Isi indeks hanya request done/provider yang belum tersusun, sehingga ukurannya mengikuti antrean
-- tertunda, bukan total historis. Kolom kunci mengikuti Q4/Q5 (ORDER BY completed_at, request_id per
-- provider) dan memberi Q9 index-only scan.
--
-- Kalau CONCURRENTLY gagal di tengah jalan, PostgreSQL meninggalkan indeks INVALID. Jalankan rollback
-- berkas ini (DROP INDEX CONCURRENTLY IF EXISTS), lalu terapkan ulang.
-- Rollback: backend/db/rollback/20261108000002_f1_penanda_indeks.down.sql.
CREATE INDEX CONCURRENTLY request_data_balikan_tertunda_idx ON request (provider_id, completed_at, request_id)
  WHERE status = 'done' AND sumber_hasil = 'provider' AND NOT data_balikan_tersusun;
