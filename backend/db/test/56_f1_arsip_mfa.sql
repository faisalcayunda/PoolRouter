-- F1 behaviour test, part 2 (SHAA-333): archive manifest and restore (D6), MFA (D7). Synthetic data only.
\set ON_ERROR_STOP on

-- ================================================================ arsip_manifest (6.2, AR-02, DQ-15, DQ-48 a-c)
BEGIN;
SELECT t.jalan($$INSERT INTO arsip_manifest (manifest_id, tabel_sumber, provider_id, tanggal_wib, objek_kunci, format, versi_contract, jumlah_baris,
    kode_transaksi_min, kode_transaksi_maks, sha256, ukuran_byte, kunci_hmac_versi, nik_mode, payload_mode, nik_kek_id, enkripsi_format_versi, dek_terbungkus, status, ditulis_at)
  VALUES ('f6000000-0000-7000-8000-000000000001', 'transaction_log', '20000000-0000-7000-8000-000000000001', '2026-08-01', 'arsip/transaction_log/p1/2026-08-01.parquet',
    'parquet-zstd-1', 'v0.9.5', 100, 'b0000000-0000-7000-8000-000000000001', 'b0000000-0000-7000-8000-0000000000ff', repeat('a', 64), 1024, 1,
    'enkripsi_aplikasi', 'enkripsi_aplikasi', 'kek-arsip-1', 1, '\xdeadbeef', 'ditulis', now())$$, 'encrypted transaction_log manifest accepted');
-- one row per unit; the same unit again is refused, also for audit_akses where provider_id is NULL
SELECT t.gagal($$INSERT INTO arsip_manifest (manifest_id, tabel_sumber, provider_id, tanggal_wib, objek_kunci, format, versi_contract, jumlah_baris,
    kode_transaksi_min, kode_transaksi_maks, sha256, ukuran_byte, kunci_hmac_versi, nik_mode, payload_mode, status)
  VALUES (gen_random_uuid(), 'transaction_log', '20000000-0000-7000-8000-000000000001', '2026-08-01', 'arsip/lain.parquet',
    'parquet-zstd-1', 'v0.9.5', 1, 'b0000000-0000-7000-8000-000000000001', 'b0000000-0000-7000-8000-000000000001', repeat('a', 64), 1, 1, 'plaintext', 'plaintext', 'ditulis')$$,
  '23505', 'AR-03: one manifest per (tabel_sumber, provider_id, tanggal_wib)');
SELECT t.jalan($$INSERT INTO arsip_manifest (manifest_id, tabel_sumber, tanggal_wib, objek_kunci, format, versi_contract, jumlah_baris, sha256, ukuran_byte, status)
  VALUES (gen_random_uuid(), 'audit_akses', '2026-08-01', 'arsip/audit/2026-08-01.parquet', 'parquet-zstd-1', 'v0.9.5', 5, repeat('b', 64), 10, 'ditulis')$$,
  'audit_akses manifest without provider accepted');
SELECT t.gagal($$INSERT INTO arsip_manifest (manifest_id, tabel_sumber, tanggal_wib, objek_kunci, format, versi_contract, jumlah_baris, sha256, ukuran_byte, status)
  VALUES (gen_random_uuid(), 'audit_akses', '2026-08-01', 'arsip/audit/dup.parquet', 'parquet-zstd-1', 'v0.9.5', 5, repeat('b', 64), 10, 'ditulis')$$, '23505',
  'NULL provider_id counts as a value in the unit key (contract 6.2)');
ROLLBACK;

BEGIN;
-- helper: a valid base row, one column changed per case
CREATE TEMP TABLE m_ok (j jsonb);
INSERT INTO m_ok VALUES (jsonb_build_object('manifest_id', gen_random_uuid(), 'tabel_sumber', 'transaction_log', 'provider_id', '20000000-0000-7000-8000-000000000001',
  'tanggal_wib', '2026-08-02', 'objek_kunci', 'k0', 'format', 'parquet-zstd-1', 'versi_contract', 'v0.9.5', 'jumlah_baris', 10, 'jumlah_dikecualikan_h02', 0,
  'kode_transaksi_min', 'b0000000-0000-7000-8000-000000000001', 'kode_transaksi_maks', 'b0000000-0000-7000-8000-000000000002', 'sha256', repeat('c', 64),
  'ukuran_byte', 5, 'kunci_hmac_versi', 1, 'nik_mode', 'plaintext', 'payload_mode', 'plaintext', 'status', 'ditulis', 'legal_hold', false));
CREATE OR REPLACE FUNCTION pg_temp.ins(p jsonb) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
  INSERT INTO arsip_manifest SELECT * FROM jsonb_populate_record(NULL::arsip_manifest, (SELECT j FROM m_ok) || p || jsonb_build_object('manifest_id', gen_random_uuid()));
END $$;
SELECT t.jalan($$SELECT pg_temp.ins('{"objek_kunci":"k1"}')$$, 'base row accepted (plaintext modes are allowed by the schema; DQ-48 d is the environment gate)');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k2","kunci_hmac_versi":null}')$$, '23514', 'DQ-48 (c): transaction_log manifest needs kunci_hmac_versi');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k3","nik_mode":null}')$$, '23514', 'DQ-48 (c): transaction_log manifest needs nik_mode');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k4","payload_mode":null}')$$, '23514', 'DQ-48 (c): transaction_log manifest needs payload_mode');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k5","nik_mode":"enkripsi_aplikasi"}')$$, '23514', 'DQ-48 (c): an encrypted manifest needs KEK id, format version and wrapped DEK');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k6","payload_mode":"enkripsi_aplikasi","nik_kek_id":"k","enkripsi_format_versi":1}')$$, '23514', 'DQ-48 (c): wrapped DEK is required too');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k7","nik_mode":"rot13"}')$$, '23514', 'nik_mode domain');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k8","kode_transaksi_min":null,"kode_transaksi_maks":null}')$$, '23514', 'transaction_log manifest needs the kode_transaksi range');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k9","kode_transaksi_min":"b0000000-0000-7000-8000-000000000009"}')$$, '23514', 'range min <= max');
SELECT t.jalan($$SELECT pg_temp.ins('{"objek_kunci":"k10","tabel_sumber":"webhook_log","kode_transaksi_min":null,"kode_transaksi_maks":null,"kunci_hmac_versi":null,"nik_mode":null,"payload_mode":null}')$$,
  'webhook_log manifest needs no NIK or payload mode (outside the R-24 gate)');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k11","tabel_sumber":"request","kode_transaksi_min":null,"kode_transaksi_maks":null,"kunci_hmac_versi":null,"nik_mode":null,"payload_mode":null}')$$, '23514',
  'DQ-48 (c): request manifest needs payload_mode');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k12","tabel_sumber":"webhook_log","provider_id":null,"kode_transaksi_min":null,"kode_transaksi_maks":null,"kunci_hmac_versi":null,"nik_mode":null,"payload_mode":null}')$$,
  '23514', 'provider_id is required except for audit_akses');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k13","tabel_sumber":"audit_akses","kode_transaksi_min":null,"kode_transaksi_maks":null,"kunci_hmac_versi":null,"nik_mode":null,"payload_mode":null}')$$,
  '23514', 'audit_akses manifest has no provider_id');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k14","sha256":"ABCDEF"}')$$, '23514', 'sha256 is 64 lower-case hex characters');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k15","jumlah_dikecualikan_h02":11}')$$, '23514', 'H-02 exclusions cannot exceed the row count');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k16","status":"terverifikasi"}')$$, '23514', 'terverifikasi needs diverifikasi_at');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k17","status":"online_dihapus","diverifikasi_at":"2026-08-03T00:00:00Z"}')$$, '23514', 'online_dihapus needs online_dihapus_at');
SELECT t.jalan($$SELECT pg_temp.ins('{"objek_kunci":"k18","status":"online_dihapus","diverifikasi_at":"2026-08-03T00:00:00Z","online_dihapus_at":"2026-08-03T00:01:00Z","tanggal_wib":"2026-08-04"}')$$, 'online_dihapus with both stamps accepted');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k19","status":"hilang"}')$$, '23514', 'manifest status domain');
SELECT t.gagal($$SELECT pg_temp.ins('{"objek_kunci":"k0"}') ; SELECT pg_temp.ins('{"objek_kunci":"k0"}')$$, '23505', 'objek_kunci is unique');
ROLLBACK;

-- ================================================================ arsip_restore_permintaan (6.3, AR-05..AR-07, DQ-49)
BEGIN;
CREATE OR REPLACE FUNCTION pg_temp.minta(p jsonb DEFAULT '{}') RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
  rid uuid := gen_random_uuid();
  aid uuid := gen_random_uuid();
  base jsonb;
  rec arsip_restore_permintaan;
BEGIN
  INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, alasan, hasil, sumber_ip, sesi_id, target_restore_id)
    VALUES (aid, '00000000-0000-7000-8000-000000000003', 'auditor', 'minta_restore', 'tidak_ada', 'periksa kasus uji', 'diizinkan', '10.0.0.1', 's1', rid);
  base := jsonb_build_object('restore_id', rid, 'diminta_oleh', '00000000-0000-7000-8000-000000000003', 'audit_id', aid,
    'provider_id', '20000000-0000-7000-8000-000000000001', 'rentang_mulai', '2026-07-01', 'rentang_selesai', '2026-07-07',
    'alasan', 'periksa kasus uji', 'status', 'antre', 'diminta_at', now());
  rec := jsonb_populate_record(NULL::arsip_restore_permintaan, base || p);
  INSERT INTO arsip_restore_permintaan SELECT rec.*;
  RETURN rid;
END $$;
SELECT t.jalan_commit($$SELECT pg_temp.minta()$$, 'AR-05: 7 days without NIK accepted');
SELECT t.gagal_commit($$SELECT pg_temp.minta('{"rentang_selesai":"2026-07-08"}')$$, '23514', 'AR-05: 8 days without NIK refused (limit 7)');
SELECT t.jalan_commit($$SELECT pg_temp.minta('{"rentang_mulai":"2026-01-01","rentang_selesai":"2026-12-31","nik_hmac_per_versi":[{"versi":1,"hmac":"aa"}],"nik_tersamar":"0021"}')$$,
  'AR-05: a year with a NIK HMAC accepted');
SELECT t.gagal_commit($$SELECT pg_temp.minta('{"rentang_mulai":"2025-01-01","rentang_selesai":"2026-12-31","nik_hmac_per_versi":[{"versi":1,"hmac":"aa"}]}')$$, '23514', 'AR-05: 24 months with NIK refused (limit 12)');
SELECT t.gagal($$SELECT pg_temp.minta('{"nik_hmac_per_versi":{"versi":1}}')$$, '23514', 'nik_hmac_per_versi is a JSON array');
SELECT t.gagal($$SELECT pg_temp.minta('{"nik_tersamar":"12"}')$$, '23514', 'nik_tersamar is 4 digits');
SELECT t.gagal($$SELECT pg_temp.minta('{"alasan":"  "}')$$, '23514', 'DQ-49 (a): a reason is required');
SELECT t.gagal($$SELECT pg_temp.minta('{"rentang_selesai":"2026-06-30"}')$$, '23514', 'range end >= start');
SELECT t.gagal($$SELECT pg_temp.minta('{"status":"siap","mulai_at":"2026-07-20T00:00:00Z"}')$$, '23514', 'siap needs siap_at and kedaluwarsa_at');
SELECT t.gagal($$SELECT pg_temp.minta('{"status":"kedaluwarsa","nik_tersamar":"0021"}')$$, '23514', 'AR-07 / DQ-49 (c): a kedaluwarsa request has no pseudonyms');
SELECT t.gagal($$SELECT pg_temp.minta('{"status":"hilang"}')$$, '23514', 'restore status domain');
-- DQ-49 (a): no matching audit row at commit
SELECT t.gagal_commit($$INSERT INTO arsip_restore_permintaan (restore_id, diminta_oleh, audit_id, provider_id, rentang_mulai, rentang_selesai, alasan, status)
  VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000003', gen_random_uuid(), '20000000-0000-7000-8000-000000000001', '2026-07-01', '2026-07-02', 'tanpa audit', 'antre')$$,
  '23000', 'DQ-49 (a): a request without its minta_restore audit row is refused at commit');
SELECT t.gagal_commit($$INSERT INTO arsip_restore_permintaan (restore_id, diminta_oleh, audit_id, provider_id, rentang_mulai, rentang_selesai, alasan, status)
  SELECT gen_random_uuid(), '00000000-0000-7000-8000-000000000003', audit_id, '20000000-0000-7000-8000-000000000001', '2026-07-01', '2026-07-02', 'audit milik orang lain', 'antre'
    FROM audit_akses WHERE aksi = 'minta_restore' LIMIT 1$$, '23000', 'DQ-49 (a): the audit row must name this restore id');
-- identity is frozen; status moves; expiry empties the pseudonyms and the restored rows
CREATE TEMP TABLE r_id AS SELECT pg_temp.minta('{"nik_hmac_per_versi":[{"versi":1,"hmac":"aa"}],"nik_tersamar":"0021"}') AS id;
SELECT t.gagal($$UPDATE arsip_restore_permintaan SET alasan = 'diubah' WHERE restore_id = (SELECT id FROM r_id)$$, '23000', 'request identity (reason) is frozen');
SELECT t.gagal($$UPDATE arsip_restore_permintaan SET rentang_selesai = '2026-12-31' WHERE restore_id = (SELECT id FROM r_id)$$, '23000', 'request range is frozen');
SELECT t.jalan_commit($$UPDATE arsip_restore_permintaan SET status = 'berjalan', mulai_at = now() WHERE restore_id = (SELECT id FROM r_id)$$, 'antre -> berjalan');
SELECT t.jalan_commit($$UPDATE arsip_restore_permintaan SET status = 'siap', siap_at = now(), kedaluwarsa_at = now() - interval '1 minute', jumlah_baris_pulih = 1
  WHERE restore_id = (SELECT id FROM r_id)$$, 'berjalan -> siap');
INSERT INTO arsip_pulih.transaction_log (restore_id, kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
    provider_config_version, endpoint_config_version, nik, requested_at, log_mode, worker_id)
  SELECT id, gen_random_uuid(), 'a0000000-0000-7000-8000-000000000001', 1, '20000000-0000-7000-8000-000000000001', '30000000-0000-7000-8000-000000000001',
         '10000000-0000-7000-8000-000000000001', '50000000-0000-7000-8000-000000000001', 1, 1, '0000000000000021', '2026-07-01', 'metadata_only', 'w-restore' FROM r_id;
SELECT t.ok(t.hitung('SELECT count(*) FROM arsip_pulih.transaction_log') = 1, 'restored row is stored in arsip_pulih');
SELECT t.gagal($$INSERT INTO arsip_pulih.transaction_log (restore_id, kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
    provider_config_version, endpoint_config_version, nik, requested_at, log_mode, worker_id)
  SELECT gen_random_uuid(), gen_random_uuid(), 'a0000000-0000-7000-8000-000000000001', 1, '20000000-0000-7000-8000-000000000001', '30000000-0000-7000-8000-000000000001',
         '10000000-0000-7000-8000-000000000001', '50000000-0000-7000-8000-000000000001', 1, 1, '0000000000000021', '2026-07-01', 'metadata_only', 'w-restore'$$, '23503',
  'arsip_pulih rows belong to a restore request');
SELECT t.gagal($$INSERT INTO arsip_pulih.transaction_log (restore_id, kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
    provider_config_version, endpoint_config_version, nik, requested_at, log_mode, worker_id)
  SELECT id, gen_random_uuid(), 'a0000000-0000-7000-8000-000000000001', 1, '20000000-0000-7000-8000-000000000001', '30000000-0000-7000-8000-000000000001',
         '10000000-0000-7000-8000-000000000001', '50000000-0000-7000-8000-000000000001', 1, 1, '12345', '2026-07-01', 'metadata_only', 'w-restore' FROM r_id$$, '23514',
  'arsip_pulih keeps the transaction_log CHECKs (NIK is 16 digits)');
SELECT t.ok(fn_arsip_pulih_kedaluwarsa((SELECT id FROM r_id)) = 1, 'AR-07: expiry removes the restored rows');
SELECT t.ok((SELECT status = 'kedaluwarsa' AND nik_hmac_per_versi IS NULL AND nik_tersamar IS NULL FROM arsip_restore_permintaan WHERE restore_id = (SELECT id FROM r_id)),
  'AR-07: expired request has status kedaluwarsa and empty pseudonyms');
SELECT t.ok(fn_arsip_pulih_kedaluwarsa((SELECT id FROM r_id)) = 0, 'AR-07: expiry is idempotent');
SELECT t.gagal($$UPDATE arsip_restore_permintaan SET status = 'siap', siap_at = now(), kedaluwarsa_at = now(), mulai_at = now() WHERE restore_id = (SELECT id FROM r_id)$$, '23000',
  'an expired request does not come back');
ROLLBACK;

-- ================================================================ D7 MFA (MF-01..MF-10, DQ-50)
BEGIN;
SELECT t.jalan($$INSERT INTO pengguna_mfa (pengguna_id, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, status)
  VALUES ('00000000-0000-7000-8000-000000000003', '\xaa', '\xbb', '\xcc', 'kek-1', 'menunggu_konfirmasi')$$, 'pengguna_mfa row accepted');
SELECT t.gagal($$INSERT INTO pengguna_mfa (pengguna_id, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, status)
  VALUES ('00000000-0000-7000-8000-000000000003', '\xaa', '\xbb', '\xcc', 'kek-1', 'menunggu_konfirmasi')$$, '23505', 'MF-01: one row per user');
SELECT t.gagal($$INSERT INTO pengguna_mfa (pengguna_id, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, status)
  VALUES ('00000000-0000-7000-8000-000000000001', '\xaa', '\xbb', '\xcc', 'kek-1', 'aktif')$$, '23514', 'status aktif needs dikonfirmasi_at');
SELECT t.gagal($$INSERT INTO pengguna_mfa (pengguna_id, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, status, dikonfirmasi_at)
  VALUES ('00000000-0000-7000-8000-000000000001', '\xaa', '\xbb', '\xcc', 'kek-1', 'menunggu_konfirmasi', now())$$, '23514', 'menunggu_konfirmasi has no dikonfirmasi_at');
SELECT t.gagal($$INSERT INTO pengguna_mfa (pengguna_id, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, status)
  VALUES (gen_random_uuid(), '\xaa', '\xbb', '\xcc', 'kek-1', 'menunggu_konfirmasi')$$, '23503', 'pengguna_mfa references admin_user');
SELECT t.gagal($$INSERT INTO pengguna_mfa (pengguna_id, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, status)
  VALUES ('00000000-0000-7000-8000-000000000001', NULL, '\xbb', '\xcc', 'kek-1', 'menunggu_konfirmasi')$$, '23502', 'the secret is stored as ciphertext only, never NULL');
SELECT t.ok((SELECT langkah_terakhir FROM pengguna_mfa) = 0, 'langkah_terakhir defaults to 0');
SELECT t.jalan($$UPDATE pengguna_mfa SET langkah_terakhir = 59000000 WHERE pengguna_id = '00000000-0000-7000-8000-000000000003'$$, 'MF-03: langkah_terakhir goes up');
SELECT t.gagal($$UPDATE pengguna_mfa SET langkah_terakhir = 59000000 - 1 WHERE pengguna_id = '00000000-0000-7000-8000-000000000003'$$, '23514', 'MF-03: langkah_terakhir never goes down');
SELECT t.ok(t.hitung($$WITH u AS (UPDATE pengguna_mfa SET langkah_terakhir = 59000001 WHERE pengguna_id = '00000000-0000-7000-8000-000000000003' AND langkah_terakhir < 59000001 RETURNING 1) SELECT count(*) FROM u$$) = 1,
  'MF-03: compare-and-set changes exactly one row for a new step');
SELECT t.ok(t.hitung($$WITH u AS (UPDATE pengguna_mfa SET langkah_terakhir = 59000001 WHERE pengguna_id = '00000000-0000-7000-8000-000000000003' AND langkah_terakhir < 59000001 RETURNING 1) SELECT count(*) FROM u$$) = 0,
  'MF-03: replaying the same step changes no row');
ROLLBACK;

BEGIN;
SELECT t.gagal($$INSERT INTO kode_pemulihan_mfa (kode_id, pengguna_id, kode_hash) VALUES (gen_random_uuid(), '00000000-0000-7000-8000-000000000003', 'plain-code')$$, '23514', 'recovery code is stored as an argon2id hash');
INSERT INTO kode_pemulihan_mfa (kode_id, pengguna_id, kode_hash) VALUES ('f7000000-0000-7000-8000-000000000001', '00000000-0000-7000-8000-000000000003', '$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA');
SELECT t.ok(t.hitung($$WITH u AS (UPDATE kode_pemulihan_mfa SET dipakai_at = now() WHERE kode_id = 'f7000000-0000-7000-8000-000000000001' AND dipakai_at IS NULL RETURNING 1) SELECT count(*) FROM u$$) = 1,
  'MF-02: first use changes exactly one row');
SELECT t.ok(t.hitung($$WITH u AS (UPDATE kode_pemulihan_mfa SET dipakai_at = now() WHERE kode_id = 'f7000000-0000-7000-8000-000000000001' AND dipakai_at IS NULL RETURNING 1) SELECT count(*) FROM u$$) = 0,
  'MF-02: a second use changes no row');
SELECT t.gagal($$UPDATE kode_pemulihan_mfa SET dipakai_at = NULL$$, '23000', 'MF-02: a used code cannot be re-opened');
SELECT t.gagal($$UPDATE kode_pemulihan_mfa SET kode_hash = '$argon2id$v=19$m=1,t=1,p=1$x$y'$$, '23000', 'MF-02: the hash never changes');
ROLLBACK;

BEGIN;
CREATE OR REPLACE FUNCTION pg_temp.kode(p jsonb DEFAULT '{}') RETURNS void LANGUAGE plpgsql AS $$
DECLARE
  base jsonb := jsonb_build_object('kode_id', gen_random_uuid(), 'pengguna_id', '00000000-0000-7000-8000-000000000003', 'kode_hash', repeat('a', 64),
    'diterbitkan_oleh', '00000000-0000-7000-8000-000000000001', 'diterbitkan_at', now(), 'kedaluwarsa_at', now() + interval '24 hours', 'gagal_count', 0);
BEGIN
  INSERT INTO kode_daftar_mfa SELECT * FROM jsonb_populate_record(NULL::kode_daftar_mfa, base || p);
END $$;
SELECT t.jalan($$SELECT pg_temp.kode('{"kode_id":"f8000000-0000-7000-8000-000000000001"}')$$, 'enrolment code accepted');
SELECT t.gagal($$SELECT pg_temp.kode()$$, '23505', 'MF-10 (i) / U-83: one active code per user, enforced by the unique partial index');
SELECT t.jalan($$SELECT pg_temp.kode('{"pengguna_id":"00000000-0000-7000-8000-000000000004"}')$$, 'another user can have an active code');
-- a new issue retires the old code ("diganti") in the same transaction and then inserts
UPDATE kode_daftar_mfa SET gugur_at = now(), gugur_alasan = 'diganti' WHERE kode_id = 'f8000000-0000-7000-8000-000000000001';
SELECT t.jalan($$SELECT pg_temp.kode('{"kode_id":"f8000000-0000-7000-8000-000000000002"}')$$, 'after "diganti" a new code is accepted');
SELECT t.gagal($$SELECT pg_temp.kode('{"pengguna_id":"00000000-0000-7000-8000-000000000001","diterbitkan_oleh":"00000000-0000-7000-8000-000000000001"}')$$, '23514', 'MF-10 (ii): issuer is never the target');
SELECT t.gagal($$SELECT pg_temp.kode('{"pengguna_id":"00000000-0000-7000-8000-000000000002","kedaluwarsa_at":"2099-01-01T00:00:00Z"}')$$, '23514', 'DQ-50 (g2): lifetime is exactly 24 hours');
SELECT t.gagal($$SELECT pg_temp.kode('{"pengguna_id":"00000000-0000-7000-8000-000000000002","kode_hash":"ABC"}')$$, '23514', 'code hash is SHA-256 hex (64 lower-case characters)');
SELECT t.gagal($$SELECT pg_temp.kode('{"pengguna_id":"00000000-0000-7000-8000-000000000002","gagal_count":6}')$$, '23514', 'DQ-50 (g2): gagal_count 0..5');
SELECT t.gagal($$SELECT pg_temp.kode('{"pengguna_id":"00000000-0000-7000-8000-000000000002","gagal_count":5}')$$, '23514', 'DQ-50 (g2): five failures without gugur_at refused');
SELECT t.jalan($$SELECT pg_temp.kode('{"pengguna_id":"00000000-0000-7000-8000-000000000002","gagal_count":5,"gugur_at":"2026-11-04T00:00:00Z","gugur_alasan":"gagal_5x"}')$$, 'five failures with gugur gagal_5x accepted');
SELECT t.gagal($$SELECT pg_temp.kode('{"pengguna_id":"00000000-0000-7000-8000-000000000002","kode_id":"f8000000-0000-7000-8000-000000000009","gagal_count":2,"gugur_at":"2026-11-04T00:00:00Z","gugur_alasan":"gagal_5x"}')$$, '23514', 'gagal_5x only with five failures');
SELECT t.gagal($$SELECT pg_temp.kode('{"pengguna_id":"00000000-0000-7000-8000-000000000002","kode_id":"f8000000-0000-7000-8000-000000000009","gugur_at":"2026-11-04T00:00:00Z"}')$$, '23514', 'gugur_at and gugur_alasan come together');
SELECT t.gagal($$SELECT pg_temp.kode('{"pengguna_id":"00000000-0000-7000-8000-000000000002","kode_id":"f8000000-0000-7000-8000-000000000009","dipakai_at":"2026-11-04T00:00:00Z","gugur_at":"2026-11-04T00:00:00Z","gugur_alasan":"diganti"}')$$, '23514', 'a code is used or retired, not both');
SELECT t.ok(t.hitung($$WITH u AS (UPDATE kode_daftar_mfa SET dipakai_at = now() WHERE kode_id = 'f8000000-0000-7000-8000-000000000002' AND dipakai_at IS NULL AND gugur_at IS NULL RETURNING 1) SELECT count(*) FROM u$$) = 1,
  'confirmation: dipakai_at set by compare-and-set');
SELECT t.ok(t.hitung($$WITH u AS (UPDATE kode_daftar_mfa SET dipakai_at = now() WHERE kode_id = 'f8000000-0000-7000-8000-000000000002' AND dipakai_at IS NULL AND gugur_at IS NULL RETURNING 1) SELECT count(*) FROM u$$) = 0,
  'MF-10: a used code cannot be used again');
SELECT t.gagal($$UPDATE kode_daftar_mfa SET kode_hash = repeat('b', 64) WHERE kode_id = 'f8000000-0000-7000-8000-000000000002'$$, '23000', 'the code hash never changes');
SELECT t.jalan($$UPDATE kode_daftar_mfa SET gagal_count = 2 WHERE kode_id = 'f8000000-0000-7000-8000-000000000001'$$, 'gagal_count goes up');
SELECT t.gagal($$UPDATE kode_daftar_mfa SET gagal_count = 1 WHERE kode_id = 'f8000000-0000-7000-8000-000000000001'$$, '23000', 'gagal_count never goes down');
ROLLBACK;
