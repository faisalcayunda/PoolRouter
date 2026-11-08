-- F1 data balikan: penanda request.data_balikan_tersusun untuk penyusun batch (ARC-199, risiko R1 T5b).
-- Sumber: products/poolrouter/specs/f1-webhook-retry-data-balikan/t5b-penyusun.md (R1) dan keputusan CEO
-- di ARC-199 (opsi 1, jalur penanda).
--
-- Kenapa penanda dan bukan indeks biasa: Q4/Q5/Q9 penyusun menyaring "request done/provider yang belum
-- punya item". Predikat done/provider mencakup hampir seluruh tabel milik provider dan request tidak
-- pernah dihapus, sedangkan yang selektif adalah NOT EXISTS data_balikan_item, kondisi di tabel lain.
-- Indeks pada request saja tidak bisa memfilternya (EXPLAIN 1,17 juta request di PR ARC-199). Penanda
-- ini membuat himpunan tertunda bisa diindeks; indeks parsialnya ada di 20261108000002 (CONCURRENTLY,
-- berkas sendiri). SQL penyusun yang menulis dan membaca penanda adalah milik ARC-181.
--
-- Semantik: true = request punya data_balikan_item aktif. Penyusun (Q8) menyetel true di transaksi yang
-- sama dengan INSERT item; susun ulang (T7) menyetelnya kembali false saat item dinonaktifkan.
--
-- Berjalan sebagai poolrouter_owner. Rollback: backend/db/rollback/20261108000001_f1_penanda_data_balikan.down.sql
-- (setelah rollback 20261108000002).

-- DEFAULT konstan: PostgreSQL 11+ tidak menulis ulang tabel.
ALTER TABLE request ADD COLUMN data_balikan_tersusun boolean NOT NULL DEFAULT false;

-- Backfill satu UPDATE: belum ada data produksi (asumsi kontrak ARC-199). Kalau data produksi sudah ada
-- sebelum migrasi ini berjalan, ganti dengan backfill bertahap per rentang request_id.
-- Trigger request_transisi mengizinkannya: kolom ini bukan kolom hasil yang dikunci T-05.
UPDATE request r SET data_balikan_tersusun = true
 WHERE EXISTS (SELECT 1 FROM data_balikan_item i WHERE i.request_id = r.request_id AND i.aktif);

-- Statistik gabungan: tanpa ini planner mengalikan selektivitas provider/status/sumber_hasil/penanda seolah
-- independen dan menaksir ~143 ribu baris tertunda untuk 2 ribu baris nyata (seed 1,17 juta). Taksiran itu
-- membuat Q4/Q5 memilih seq scan transaction_log + hash join (~110 ms); dengan MCV gabungan planner memakai
-- nested loop lewat PK transaction_log (~9 ms). Lihat EXPLAIN di PR ARC-199.
CREATE STATISTICS request_data_balikan_stat (mcv) ON provider_id, status, sumber_hasil, data_balikan_tersusun FROM request;
ANALYZE request;

-- Hanya kolom penanda. pr_job_balikan tetap tidak bisa menulis kolom request lain (60_f1_roles.sql).
-- SELECT perlu eksplisit: grant SELECT kolom di 20261104000004 dihitung dari kolom yang ada saat itu.
GRANT SELECT (data_balikan_tersusun), UPDATE (data_balikan_tersusun) ON request TO pr_job_balikan;
