-- F1 behaviour test (SHAA-333): constraints and triggers of the F1 schema, per contract rule (AG, AL, WR, DB, AR, MF, DQ-40..50).
-- Synthetic data only. Every block runs in a transaction that is rolled back.
\set ON_ERROR_STOP on

-- Run a statement, then force the deferred constraint triggers so a violation shows up in the same call.
CREATE OR REPLACE FUNCTION t.gagal_commit(p_sql text, p_sqlstate text, p_nama text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
  BEGIN
    EXECUTE p_sql;
    EXECUTE 'SET CONSTRAINTS ALL IMMEDIATE';
  EXCEPTION WHEN OTHERS THEN
    IF SQLSTATE = p_sqlstate THEN RAISE NOTICE 'ok - % (%)', p_nama, SQLSTATE; RETURN; END IF;
    RAISE EXCEPTION 'FAIL: % -> expected %, got % (%)', p_nama, p_sqlstate, SQLSTATE, SQLERRM;
  END;
  RAISE EXCEPTION 'FAIL: % -> statement succeeded, expected %', p_nama, p_sqlstate;
END $$;
CREATE OR REPLACE FUNCTION t.jalan_commit(p_sql text, p_nama text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
  EXECUTE p_sql;
  EXECUTE 'SET CONSTRAINTS ALL IMMEDIATE';
  RAISE NOTICE 'ok - %', p_nama;
EXCEPTION WHEN OTHERS THEN
  RAISE EXCEPTION 'FAIL: % -> % (%)', p_nama, SQLERRM, SQLSTATE;
END $$;
-- A pending (in-flight) log row for an existing request; p overrides columns.
CREATE OR REPLACE FUNCTION t.buat_log(p_request uuid, p jsonb DEFAULT '{}') RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
  base jsonb := jsonb_build_object('kode_transaksi', gen_random_uuid(), 'request_id', p_request, 'attempt_no', 1,
    'provider_id', '20000000-0000-7000-8000-000000000001', 'upstream_pool_id', '30000000-0000-7000-8000-000000000001',
    'client_id', '10000000-0000-7000-8000-000000000001', 'downstream_endpoint_id', '50000000-0000-7000-8000-000000000001',
    'provider_config_version', 1, 'endpoint_config_version', 1, 'nik', '0000000000000007', 'requested_at', now(),
    'log_mode', 'metadata_only', 'worker_id', 'w-f1');
  rec transaction_log;
BEGIN
  rec := jsonb_populate_record(NULL::transaction_log, base || p);
  INSERT INTO transaction_log SELECT rec.*;
  RETURN rec.kode_transaksi;
END $$;

-- ================================================================ D1 aging (AG-01..AG-04, DQ-40)
BEGIN;
SELECT t.jalan($$SELECT t.buat_request('{"request_id":"f1000000-0000-7000-8000-000000000001","prioritas":3,"prioritas_efektif":3,
  "aging_ambang_detik":600,"prioritas_diubah_at":"2026-11-04T00:00:00Z","received_at":"2026-11-04T00:00:00Z"}')$$, 'AG-02: async queued request with the aging snapshot accepted');
SELECT t.gagal($$UPDATE request SET aging_ambang_detik = 60 WHERE request_id = 'f1000000-0000-7000-8000-000000000001'$$, '23514',
  'AG-02: the aging snapshot does not change once set');
SELECT t.gagal($$UPDATE request SET prioritas = 5 WHERE request_id = 'f1000000-0000-7000-8000-000000000001'$$, '23514',
  'AG-01: prioritas (origin) never changes');
SELECT t.gagal($$UPDATE request SET prioritas_efektif = 2 WHERE request_id = 'f1000000-0000-7000-8000-000000000001'$$, '23514',
  'AG-03: a step must also set prioritas_diubah_at = now()');
SELECT t.gagal($$UPDATE request SET prioritas_efektif = 1, prioritas_diubah_at = now() WHERE request_id = 'f1000000-0000-7000-8000-000000000001'$$, '23514',
  'AG-03: one step only (3 -> 1 refused)');
SELECT t.gagal($$UPDATE request SET prioritas_efektif = 4 WHERE request_id = 'f1000000-0000-7000-8000-000000000001'$$, '23514',
  'AG-01: efektif cannot exceed prioritas (and never goes up)');
SELECT t.gagal_commit($$UPDATE request SET prioritas_efektif = 2, prioritas_diubah_at = now() WHERE request_id = 'f1000000-0000-7000-8000-000000000001'$$,
  '23000', 'AG-04: a priority step without its aging history row is refused at commit');
-- the history row of a step: queued -> queued, controller, one level, same timestamp
SELECT t.gagal($$INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor, prioritas_dari, prioritas_ke)
  VALUES ('f1000000-0000-7000-8000-000000000001', 2, 'queued', 'queued', 'aging', 'controller', 3, 1)$$, '23514', 'AG-04: aging row steps exactly one level');
SELECT t.gagal($$INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor, prioritas_dari, prioritas_ke)
  VALUES ('f1000000-0000-7000-8000-000000000001', 2, 'queued', 'queued', 'aging', 'worker', 3, 2)$$, '23514', 'AG-04: aging row is written by the controller');
SELECT t.gagal($$INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor, prioritas_dari, prioritas_ke)
  VALUES ('f1000000-0000-7000-8000-000000000001', 2, 'queued', 'dispatched', 'aging', 'controller', 3, 2)$$, '23514', 'AG-04: an aging row is not a status transition');
SELECT t.gagal($$INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor, prioritas_dari)
  VALUES ('f1000000-0000-7000-8000-000000000001', 2, 'queued', 'queued', 'aging', 'controller', 3)$$, '23514', 'AG-04: aging row carries both priorities');
SELECT t.gagal($$INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor, prioritas_dari, prioritas_ke)
  VALUES ('f1000000-0000-7000-8000-000000000001', 2, 'queued', 'dispatched', NULL, 'controller', 3, 2)$$, '23514', 'priorities only on aging rows');
INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor, prioritas_dari, prioritas_ke)
  VALUES ('f1000000-0000-7000-8000-000000000001', 2, 'queued', 'queued', 'aging', 'controller', 3, 2);
SELECT t.jalan_commit($$UPDATE request SET prioritas_efektif = 2, prioritas_diubah_at = now() WHERE request_id = 'f1000000-0000-7000-8000-000000000001'$$,
  'AG-03/AG-04: one step with its aging row commits (same now() for both)');
ROLLBACK;

BEGIN;
SELECT t.gagal($$SELECT t.buat_request('{"mode":"sync","status":"in_flight","attempt_count":1,"ttl_expires_at":null,"aging_ambang_detik":600}')$$, '23514',
  'DQ-40 (c): aging columns on a sync request refused');
SELECT t.gagal($$SELECT t.buat_request('{"status":"rejected","reject_reason":"kuota_habis","reject_http_status":429,"completed_at":"2026-11-03T00:00:00Z","prioritas_diubah_at":"2026-11-03T00:00:00Z"}')$$,
  '23514', 'DQ-40 (c): aging columns on a rejected request refused');
SELECT t.gagal($$SELECT t.buat_request('{"aging_ambang_detik":30}')$$, '23514', 'aging threshold 1..59 refused (2.1)');
SELECT t.gagal($$SELECT t.buat_request('{"aging_ambang_detik":86401}')$$, '23514', 'aging threshold above 86400 refused');
SELECT t.jalan($$SELECT t.buat_request('{"aging_ambang_detik":0,"prioritas_diubah_at":"2099-01-01T00:00:00Z"}')$$, 'threshold 0 (aging off) accepted');
ROLLBACK;
BEGIN;
SELECT t.gagal($$SELECT t.buat_request('{"prioritas":2,"prioritas_efektif":3}')$$, '23514', 'AG-01: prioritas_efektif <= prioritas');
SELECT t.gagal($$SELECT t.buat_request('{"aging_ambang_detik":600,"prioritas_diubah_at":"2000-01-01T00:00:00Z"}')$$, '23514', 'DQ-40 (c): prioritas_diubah_at >= received_at');
SELECT t.gagal($$UPDATE downstream_pool SET aging_ambang_detik = 59$$, '23514', 'downstream_pool.aging_ambang_detik: 0 or 60..86400');
SELECT t.jalan($$UPDATE downstream_pool SET aging_ambang_detik = 0$$, 'downstream_pool.aging_ambang_detik 0 accepted');
SELECT t.ok((SELECT column_default FROM information_schema.columns WHERE table_name = 'downstream_pool' AND column_name = 'aging_ambang_detik') = '600',
  'downstream_pool.aging_ambang_detik defaults to 600');
ROLLBACK;

-- ================================================================ D2 allocation (AL-01..AL-07, DQ-41, DQ-44)
BEGIN;
-- up-1: kuota_harian 1000, margin 0.010 -> Q = 990
SELECT t.gagal($$UPDATE downstream_pool_rute SET alokasi_jenis = 'exact'$$, '23514', 'CHECK (i): jenis, nilai and sifat all set or all NULL');
SELECT t.gagal($$UPDATE downstream_pool_rute SET boleh_pinjam = false$$, '23514', 'CHECK (ii): a route without allocation must allow borrowing (U-65)');
SELECT t.gagal($$UPDATE downstream_pool_rute SET alokasi_jenis = 'exact', alokasi_nilai = 1.5, sifat = 'reserved', boleh_pinjam = false$$, '23514', 'CHECK (iii): exact is a whole number');
SELECT t.gagal($$UPDATE downstream_pool_rute SET alokasi_jenis = 'exact', alokasi_nilai = 0, sifat = 'reserved', boleh_pinjam = false$$, '23514', 'CHECK (iii): exact >= 1');
SELECT t.gagal($$UPDATE downstream_pool_rute SET alokasi_jenis = 'persen', alokasi_nilai = 0, sifat = 'reserved', boleh_pinjam = false$$, '23514', 'CHECK (iii): persen >= 0.01');
SELECT t.gagal($$UPDATE downstream_pool_rute SET alokasi_jenis = 'persen', alokasi_nilai = 100.01, sifat = 'reserved', boleh_pinjam = false$$, '23514', 'CHECK (iii): persen <= 100');
SELECT t.gagal($$UPDATE downstream_pool_rute SET alokasi_jenis = 'bagi', alokasi_nilai = 5, sifat = 'reserved', boleh_pinjam = false$$, '23514', 'alokasi_jenis domain');
SELECT t.gagal($$UPDATE downstream_pool_rute SET alokasi_jenis = 'exact', alokasi_nilai = 5, sifat = 'dipinjam', boleh_pinjam = false$$, '23514', 'sifat domain');
SELECT t.jalan_commit($$UPDATE downstream_pool_rute SET alokasi_jenis = 'exact', alokasi_nilai = 990, sifat = 'reserved', boleh_pinjam = false$$,
  'AL-01: allocation equal to Q accepted');
SELECT t.gagal_commit($$UPDATE downstream_pool_rute SET alokasi_nilai = 991$$, '23514', 'AL-01 / U-64: sum above Q refused with the excess (991 > 990)');
SELECT t.jalan_commit($$UPDATE downstream_pool_rute SET alokasi_jenis = 'persen', alokasi_nilai = 100.00$$, 'AL-01: 100 persen = floor(Q) accepted');
INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) VALUES ('f2000000-0000-7000-8000-000000000002', 'down-f1', 1);
SELECT t.gagal_commit($$INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id, alokasi_jenis, alokasi_nilai, sifat, boleh_pinjam)
  VALUES ('f2000000-0000-7000-8000-000000000002', '30000000-0000-7000-8000-000000000001', 'exact', 1, 'shareable', true)$$, '23514',
  'AL-01: a second route pushes the sum above Q');
SELECT t.jalan_commit($$UPDATE downstream_pool_rute SET alokasi_jenis = 'persen', alokasi_nilai = 50.00 WHERE downstream_pool_id = '40000000-0000-7000-8000-000000000001';
  INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id, alokasi_jenis, alokasi_nilai, sifat, boleh_pinjam)
  VALUES ('f2000000-0000-7000-8000-000000000002', '30000000-0000-7000-8000-000000000001', 'persen', 50.00, 'shareable', true)$$,
  'AL-01: 50 + 50 persen = Q accepted');
SELECT t.jalan_commit($$UPDATE downstream_pool_rute SET alokasi_jenis = 'exact', alokasi_nilai = CASE WHEN downstream_pool_id = '40000000-0000-7000-8000-000000000001' THEN 400 ELSE 500 END$$,
  'AL-01: exact 400 + 500 <= 990 accepted');
SELECT t.gagal_commit($$UPDATE upstream_pool SET kuota_harian = 900$$, '23514', 'AL-01: lowering kuota_harian below the allocated sum is refused (U-64)');
SELECT t.gagal_commit($$UPDATE upstream_pool SET margin_kuota = 0.200$$, '23514', 'AL-01: raising margin_kuota below the allocated sum is refused');
UPDATE downstream_pool SET aktif = false WHERE downstream_pool_id = 'f2000000-0000-7000-8000-000000000002';
UPDATE downstream_pool_rute SET alokasi_jenis = 'persen', alokasi_nilai = 60.00 WHERE downstream_pool_id = '40000000-0000-7000-8000-000000000001';
UPDATE downstream_pool_rute SET alokasi_jenis = 'persen', alokasi_nilai = 50.00 WHERE downstream_pool_id = 'f2000000-0000-7000-8000-000000000002';
SELECT t.jalan_commit($$SELECT 1$$, 'deactivated route leaves the sum: 60 persen alone is fine');
SELECT t.gagal_commit($$UPDATE downstream_pool SET aktif = true WHERE downstream_pool_id = 'f2000000-0000-7000-8000-000000000002'$$, '23514',
  'AL-01: re-activating an allocated route that overflows Q is refused (counts as an allocation change)');
ROLLBACK;

BEGIN;
SELECT t.ok(fn_alokasi_kelebihan('30000000-0000-7000-8000-000000000001') = 0, 'fn_alokasi_kelebihan: no allocation, no excess');
UPDATE downstream_pool_rute SET alokasi_jenis = 'exact', alokasi_nilai = 1000, sifat = 'reserved', boleh_pinjam = false;
SELECT t.ok(fn_alokasi_kelebihan('30000000-0000-7000-8000-000000000001') = 10, 'fn_alokasi_kelebihan reports the excess (1000 - 990 = 10)');
ROLLBACK;

-- log columns and reject_lingkup
BEGIN;
SELECT t.buat_request('{"request_id":"f1000000-0000-7000-8000-000000000003","status":"in_flight","attempt_count":1,"upstream_pool_id":"30000000-0000-7000-8000-000000000001"}');
SELECT t.jalan($$SELECT t.buat_log('f1000000-0000-7000-8000-000000000003', '{"sumber_anggaran":"sisa"}')$$, 'sumber_anggaran = sisa accepted');
SELECT t.jalan($$SELECT t.buat_log('f1000000-0000-7000-8000-000000000003', '{"attempt_no":2,"sumber_anggaran":"40000000-0000-7000-8000-000000000001","downstream_pool_id":"40000000-0000-7000-8000-000000000001"}')$$,
  'sumber_anggaran = pool uuid accepted');
SELECT t.jalan($$SELECT t.buat_log('f1000000-0000-7000-8000-000000000003', '{"attempt_no":3}')$$, 'F0-shaped row (both new columns NULL) accepted');
SELECT t.gagal($$SELECT t.buat_log('f1000000-0000-7000-8000-000000000003', '{"attempt_no":4,"sumber_anggaran":"bukan-uuid"}')$$, '23514', 'AL-02: sumber_anggaran is sisa or a uuid');
SELECT t.gagal($$SELECT t.buat_log('f1000000-0000-7000-8000-000000000003', '{"attempt_no":4,"downstream_pool_id":"40000000-0000-7000-8000-000000000009"}')$$, '23514', 'an unknown downstream_pool_id is refused (mismatch trigger runs before the FK)');
INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) VALUES ('f2000000-0000-7000-8000-000000000009', 'down-lain', 1);
SELECT t.gagal($$SELECT t.buat_log('f1000000-0000-7000-8000-000000000003', '{"attempt_no":4,"downstream_pool_id":"f2000000-0000-7000-8000-000000000009"}')$$, '23514',
  'DQ-44 (b): log downstream_pool_id must equal request.downstream_pool_id');
SELECT t.gagal($$UPDATE transaction_log SET sumber_anggaran = 'sisa' WHERE request_id = 'f1000000-0000-7000-8000-000000000003' AND attempt_no = 3$$, '23000',
  'R-01: sumber_anggaran is written at INSERT and never changed (pending row)');
SELECT t.gagal($$UPDATE transaction_log SET downstream_pool_id = '40000000-0000-7000-8000-000000000001' WHERE request_id = 'f1000000-0000-7000-8000-000000000003' AND attempt_no = 3$$, '23000',
  'downstream_pool_id is not changed after INSERT');
SELECT t.jalan($$UPDATE transaction_log SET status_akses = 'berhasil', responded_at = now(), duration_ms = 5, provider_http_status = 200, response_hash = repeat('c', 64)
  WHERE request_id = 'f1000000-0000-7000-8000-000000000003' AND attempt_no = 3$$, 'F0 completion of a pending log row still works');
ROLLBACK;

BEGIN;
SELECT t.jalan($$SELECT t.buat_request('{"status":"rejected","reject_reason":"kuota_habis","reject_http_status":429,"completed_at":"2026-11-03T00:00:00Z","reject_lingkup":"anggaran"}')$$,
  'reject_lingkup = anggaran on kuota_habis accepted');
SELECT t.jalan($$SELECT t.buat_request('{"status":"rejected","reject_reason":"kuota_habis","reject_http_status":429,"completed_at":"2026-11-03T00:00:00Z","reject_lingkup":"upstream_pool"}')$$,
  'reject_lingkup = upstream_pool accepted');
SELECT t.gagal($$SELECT t.buat_request('{"status":"rejected","reject_reason":"porsi_sync_habis","reject_http_status":429,"completed_at":"2026-11-03T00:00:00Z","reject_lingkup":"anggaran"}')$$,
  '23514', 'DQ-44 (a): reject_lingkup only with kuota_habis');
SELECT t.gagal($$SELECT t.buat_request('{"status":"rejected","reject_reason":"kuota_habis","reject_http_status":429,"completed_at":"2026-11-03T00:00:00Z","reject_lingkup":"global"}')$$,
  '23514', 'DQ-44 (a): reject_lingkup domain');
SELECT t.gagal($$SELECT t.buat_request('{"reject_lingkup":"anggaran"}')$$, '23514', 'reject_lingkup NULL on a non-rejected request');
ROLLBACK;

-- ================================================================ D3 webhook retry (WR-01..WR-05, DQ-46)
-- A cancelled async request (terminal) with the given webhook bookkeeping.
CREATE OR REPLACE FUNCTION t.buat_batal(p_status text, p_percobaan integer, p_not_before timestamptz, p_error text DEFAULT NULL) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
  rid uuid := t.buat_request('{}');
BEGIN
  UPDATE request SET status = 'cancelled', cancel_reason = 'dibatalkan_client', cancelled_by = 'client', completed_at = now(),
         webhook_status = p_status, webhook_attempt_count = p_percobaan, webhook_not_before = p_not_before, webhook_last_error = p_error
   WHERE request_id = rid;
  INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor)
    VALUES (rid, 2, 'queued', 'cancelled', 'dibatalkan_client', 'client');
  RETURN rid;
END $$;
BEGIN;
SELECT t.jalan($$SELECT t.buat_batal('menunggu', 1, now() + interval '10 minutes')$$, 'webhook_not_before accepted while menunggu');
SELECT t.gagal($$SELECT t.buat_batal('terkirim', 1, now() + interval '10 minutes')$$, '23514', 'DQ-46 (c): webhook_not_before NULL once the status is final');
SELECT t.gagal($$SELECT t.buat_batal('gagal', 6, now() + interval '10 minutes')$$, '23514', 'DQ-46 (c): webhook_not_before NULL on gagal');
SELECT t.gagal($$UPDATE request SET webhook_not_before = now() WHERE status = 'queued'$$, '23514', 'webhook_not_before needs webhook_status = menunggu (NULL status too)');
SELECT t.jalan($$SELECT t.buat_batal('gagal', 6, NULL, 'hasil_kedaluwarsa')$$, 'WR-04: hasil_kedaluwarsa fits the open webhook_last_error domain');
SELECT t.jalan($$SELECT t.buat_batal('menunggu', 2, now() + interval '2 minutes', 'klaim_kedaluwarsa')$$, 'WR-05: klaim_kedaluwarsa fits the open webhook_last_error domain');
ROLLBACK;

-- platform settings (F1-2.1, F1-4.2, F1-6.3)
BEGIN;
SELECT t.gagal($$UPDATE konfigurasi_platform SET nilai = '11'::jsonb WHERE kunci = 'webhook_batas_percobaan'$$, '23514', 'webhook_batas_percobaan 1..10');
SELECT t.gagal($$UPDATE konfigurasi_platform SET nilai = '0'::jsonb WHERE kunci = 'webhook_batas_percobaan'$$, '23514', 'webhook_batas_percobaan >= 1');
SELECT t.gagal($$UPDATE konfigurasi_platform SET nilai = '"enam"'::jsonb WHERE kunci = 'webhook_batas_percobaan'$$, '23514', 'settings are whole numbers');
SELECT t.gagal($$UPDATE konfigurasi_platform SET nilai = '601'::jsonb WHERE kunci = 'webhook_backoff_base_detik'$$, '23514', 'webhook_backoff_base_detik 1..600');
SELECT t.gagal($$UPDATE konfigurasi_platform SET nilai = '20'::jsonb WHERE kunci = 'webhook_backoff_cap_detik'$$, '23514', 'cap must be >= base (20 < 30)');
SELECT t.gagal($$UPDATE konfigurasi_platform SET nilai = '21601'::jsonb WHERE kunci = 'webhook_backoff_cap_detik'$$, '23514', 'cap <= 21600');
SELECT t.gagal($$UPDATE konfigurasi_platform SET nilai = '51'::jsonb WHERE kunci = 'webhook_jitter_persen'$$, '23514', 'webhook_jitter_persen 0..50');
SELECT t.jalan($$UPDATE konfigurasi_platform SET nilai = '0'::jsonb WHERE kunci = 'webhook_jitter_persen'$$, 'jitter 0 accepted');
SELECT t.gagal($$UPDATE konfigurasi_platform SET nilai = '1'::jsonb WHERE kunci = 'prioritas_maks'$$, '23514', 'prioritas_maks 2..10');
SELECT t.gagal($$UPDATE konfigurasi_platform SET nilai = '31'::jsonb WHERE kunci = 'restore_masa_berlaku_hari'$$, '23514', 'restore_masa_berlaku_hari 1..30');
SELECT t.gagal($$UPDATE konfigurasi_platform SET nilai = '37'::jsonb WHERE kunci = 'restore_rentang_dengan_nik_maks_bulan'$$, '23514', 'restore range with NIK 1..36 months');
SELECT t.gagal($$UPDATE konfigurasi_platform SET nilai = '32'::jsonb WHERE kunci = 'restore_rentang_tanpa_nik_maks_hari'$$, '23514', 'restore range without NIK 1..31 days');
SELECT t.jalan($$UPDATE konfigurasi_platform SET nilai = '9'::jsonb WHERE kunci = 'prioritas_maks'$$, 'prioritas_maks 9 accepted');
SELECT t.jalan($$UPDATE downstream_pool SET prioritas = 9$$, 'pool priority up to prioritas_maks accepted');
SELECT t.gagal($$UPDATE downstream_pool SET prioritas = 10$$, '23514', 'pool priority above prioritas_maks refused');
SELECT t.gagal($$UPDATE konfigurasi_platform SET nilai = '8'::jsonb WHERE kunci = 'prioritas_maks'$$, '23514', 'lowering prioritas_maks below a pool priority is refused (422)');
SELECT t.jalan($$INSERT INTO konfigurasi_platform (kunci, nilai) VALUES ('ekspor_catatan', '"bukan F1"'::jsonb)$$, 'F0 keys are not validated by the F1 trigger');
ROLLBACK;

-- ================================================================ D4 data balikan (DB-01, DQ-47)
BEGIN;
SELECT t.jalan($$INSERT INTO data_balikan_batch (batch_id, provider_id, pemicu, periode_mulai_at, periode_selesai_at, jumlah_item, status, jadwal_kirim_at, config_ref_version)
  VALUES ('f4000000-0000-7000-8000-000000000001', '20000000-0000-7000-8000-000000000001', 'jumlah_n', now(), now(), 1, 'disusun', now(), 2)$$, 'batch with config_ref_version accepted');
SELECT t.gagal($$INSERT INTO data_balikan_batch (batch_id, provider_id, pemicu, periode_mulai_at, periode_selesai_at, jumlah_item, status, jadwal_kirim_at)
  VALUES ('f4000000-0000-7000-8000-000000000002', '20000000-0000-7000-8000-000000000001', 'jumlah_n', now(), now(), 1, 'disusun', now())$$, '23502', 'config_ref_version is required');
SELECT t.gagal($$INSERT INTO data_balikan_batch (batch_id, provider_id, pemicu, periode_mulai_at, periode_selesai_at, jumlah_item, status, jadwal_kirim_at, config_ref_version, kirim_berikutnya_at)
  VALUES ('f4000000-0000-7000-8000-000000000002', '20000000-0000-7000-8000-000000000001', 'jumlah_n', now(), now(), 1, 'disusun', now(), 1, now())$$, '23514',
  'DQ-47 (b): kirim_berikutnya_at only on status gagal');
SELECT t.gagal($$INSERT INTO data_balikan_batch (batch_id, provider_id, pemicu, periode_mulai_at, periode_selesai_at, jumlah_item, status, jadwal_kirim_at, config_ref_version)
  VALUES ('f4000000-0000-7000-8000-000000000002', '20000000-0000-7000-8000-000000000001', 'jumlah_n', now(), now(), 1, 'gagal', now(), 1)$$, '23514',
  'DQ-47 (b): status gagal needs kirim_berikutnya_at');
SELECT t.jalan($$INSERT INTO data_balikan_batch (batch_id, provider_id, pemicu, periode_mulai_at, periode_selesai_at, jumlah_item, status, jadwal_kirim_at, config_ref_version, kirim_berikutnya_at)
  VALUES ('f4000000-0000-7000-8000-000000000002', '20000000-0000-7000-8000-000000000001', 'jumlah_n', now(), now(), 1, 'gagal', now(), 1, now() + interval '5 minutes')$$,
  'gagal batch with kirim_berikutnya_at accepted');
-- DB-01: one active item per request
SELECT t.jalan($$INSERT INTO data_balikan_item (batch_id, request_id, kode_transaksi, nik, field_input, field_metadata, requested_at)
  VALUES ('f4000000-0000-7000-8000-000000000001', 'a0000000-0000-7000-8000-000000000001', 'b0000000-0000-7000-8000-000000000001', '0000000000000021', '{}', '{}', now())$$,
  'first active item accepted (aktif defaults to true)');
SELECT t.gagal($$INSERT INTO data_balikan_item (batch_id, request_id, kode_transaksi, nik, field_input, field_metadata, requested_at)
  VALUES ('f4000000-0000-7000-8000-000000000002', 'a0000000-0000-7000-8000-000000000001', 'b0000000-0000-7000-8000-000000000001', '0000000000000021', '{}', '{}', now())$$, '23505',
  'DB-01 / U-71: a second active item for the same request is refused by the database');
SELECT t.gagal($$INSERT INTO data_balikan_item (batch_id, request_id, kode_transaksi, nik, field_input, field_metadata, requested_at, aktif)
  VALUES ('f4000000-0000-7000-8000-000000000002', 'a0000000-0000-7000-8000-000000000001', 'b0000000-0000-7000-8000-000000000001', '0000000000000021', '{}', '{}', now(), false)$$, '23514',
  'DQ-47 (a): an inactive item belongs to a gagal_permanen batch only');
-- U-73: batch -> gagal_permanen deactivates its items; the request can join a new batch; the old batch stays whole
UPDATE data_balikan_batch SET status = 'gagal_permanen' WHERE batch_id = 'f4000000-0000-7000-8000-000000000001';
SELECT t.ok((SELECT NOT aktif FROM data_balikan_item WHERE batch_id = 'f4000000-0000-7000-8000-000000000001'), 'U-73: gagal_permanen deactivates the items of that batch in the same transaction');
SELECT t.ok(t.hitung($$SELECT count(*) FROM data_balikan_item WHERE batch_id = 'f4000000-0000-7000-8000-000000000001'$$) = 1, 'U-73: the old batch keeps its item rows');
SELECT t.jalan($$INSERT INTO data_balikan_item (batch_id, request_id, kode_transaksi, nik, field_input, field_metadata, requested_at)
  VALUES ('f4000000-0000-7000-8000-000000000002', 'a0000000-0000-7000-8000-000000000001', 'b0000000-0000-7000-8000-000000000001', '0000000000000021', '{}', '{}', now())$$,
  'U-73: the request joins a new batch once the old one is gagal_permanen');
SELECT t.gagal($$INSERT INTO data_balikan_item (batch_id, request_id, kode_transaksi, nik, field_input, field_metadata, requested_at, aktif)
  SELECT 'f4000000-0000-7000-8000-000000000001', request_id, kode_transaksi, nik, field_input, field_metadata, requested_at, true FROM data_balikan_item WHERE batch_id = 'f4000000-0000-7000-8000-000000000002'$$,
  '23514', 'DQ-47 (a): an active item cannot sit in a gagal_permanen batch');
ROLLBACK;

-- ================================================================ audit_akses (F1-6.4, MF-10, DQ-49, DQ-50 d)
BEGIN;
CREATE TEMP TABLE aa (sql text);
SELECT t.gagal($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, alasan, hasil, sumber_ip, sesi_id)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000003', 'auditor', 'minta_restore', 'tidak_ada', 'periksa', 'diizinkan', '10.0.0.1', 's1')$$, '23514',
  'minta_restore needs target_restore_id');
SELECT t.gagal($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, hasil, sumber_ip, sesi_id, target_restore_id)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000003', 'auditor', 'minta_restore', 'tidak_ada', 'diizinkan', '10.0.0.1', 's1', gen_random_uuid())$$, '23514',
  'minta_restore needs a reason');
SELECT t.jalan($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, alasan, hasil, sumber_ip, sesi_id, target_restore_id)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000003', 'auditor', 'minta_restore', 'tidak_ada', 'periksa', 'diizinkan', '10.0.0.1', 's1', gen_random_uuid())$$, 'minta_restore with reason and restore id accepted');
SELECT t.gagal($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, hasil, sumber_ip, sesi_id, sumber)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000003', 'auditor', 'cari_nik', 'tersamar', 'diizinkan', '10.0.0.1', 's1', 'arsip')$$, '23514',
  'DQ-49 (d): sumber = arsip needs the restore id');
SELECT t.gagal($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, hasil, sumber_ip, sesi_id, sumber)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000003', 'auditor', 'cari_nik', 'tersamar', 'diizinkan', '10.0.0.1', 's1', 'cloud')$$, '23514', 'sumber domain');
SELECT t.gagal($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, hasil, sumber_ip, sesi_id)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000003', 'auditor', 'hapus_semua', 'tidak_ada', 'diizinkan', '10.0.0.1', 's1')$$, '23514', 'unknown aksi refused');
SELECT t.gagal($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, alasan, hasil, sumber_ip, sesi_id, filter_kueri)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000001', 'platform_admin', 'terbit_kode_daftar', 'tidak_ada', 'peran baru', 'diizinkan', 'cli', 'cli', '{"kode_id":"x"}')$$, '23514',
  'terbit_kode_daftar needs target_pengguna_id');
SELECT t.gagal($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, alasan, hasil, sumber_ip, sesi_id, target_pengguna_id)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000001', 'platform_admin', 'terbit_kode_daftar', 'tidak_ada', 'peran baru', 'diizinkan', 'cli', 'cli', '00000000-0000-7000-8000-000000000003')$$, '23514',
  'MF-10: terbit_kode_daftar carries filter_kueri.kode_id');
SELECT t.gagal($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, alasan, hasil, sumber_ip, sesi_id, target_pengguna_id, filter_kueri)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000001', 'platform_admin', 'terbit_kode_daftar', 'tidak_ada', 'peran baru', 'diizinkan', 'cli', 'cli', '00000000-0000-7000-8000-000000000001', '{"kode_id":"x"}')$$, '23514',
  'DQ-50 (d): the issuer is never the target');
SELECT t.jalan($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, alasan, hasil, sumber_ip, sesi_id, target_pengguna_id, filter_kueri)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000001', 'platform_admin', 'terbit_kode_daftar', 'tidak_ada', 'peran baru', 'diizinkan', 'cli', 'cli', '00000000-0000-7000-8000-000000000003', '{"kode_id":"x"}')$$,
  'terbit_kode_daftar with target, reason and kode_id accepted');
SELECT t.gagal($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, alasan, hasil, sumber_ip, sesi_id, target_pengguna_id)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000001', 'platform_admin', 'reset_mfa', 'tidak_ada', 'hilang', 'diizinkan', 'cli', 'cli', '00000000-0000-7000-8000-000000000001')$$, '23514',
  'DQ-50 (d): reset_mfa with actor = target refused');
SELECT t.gagal($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, alasan, hasil, sumber_ip, sesi_id, target_pengguna_id)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000001', 'platform_admin', 'reset_mfa_darurat', 'tidak_ada', 'INS-1', 'diizinkan', 'cli', 'cli', '00000000-0000-7000-8000-000000000003')$$, '23514',
  'DQ-50 (d): reset_mfa_darurat needs pelaku_kedua_id');
SELECT t.gagal($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, alasan, hasil, sumber_ip, sesi_id, target_pengguna_id, pelaku_kedua_id)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000001', 'platform_admin', 'reset_mfa_darurat', 'tidak_ada', 'INS-1', 'diizinkan', 'cli', 'cli', '00000000-0000-7000-8000-000000000003', '00000000-0000-7000-8000-000000000001')$$, '23514',
  'DQ-50 (d): the second actor differs from the first');
-- MF-06 / U-86: the target may be the first actor, the second actor or a third admin
SELECT t.jalan($$INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, alasan, hasil, sumber_ip, sesi_id, target_pengguna_id, pelaku_kedua_id)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000001', 'platform_admin', 'reset_mfa_darurat', 'tidak_ada', 'INS-1', 'diizinkan', 'cli', 'cli', '00000000-0000-7000-8000-000000000003', '00000000-0000-7000-8000-000000000002'),
  (gen_random_uuid(), '00000000-0000-7000-8000-000000000001', 'platform_admin', 'reset_mfa_darurat', 'tidak_ada', 'INS-1', 'diizinkan', 'cli', 'cli', '00000000-0000-7000-8000-000000000001', '00000000-0000-7000-8000-000000000002'),
  (gen_random_uuid(), '00000000-0000-7000-8000-000000000001', 'platform_admin', 'reset_mfa_darurat', 'tidak_ada', 'INS-1', 'diizinkan', 'cli', 'cli', '00000000-0000-7000-8000-000000000002', '00000000-0000-7000-8000-000000000002')$$,
  'U-86: break-glass accepts any admin as target (incl. an actor); no row-level restriction on the target') ;
ROLLBACK;
