-- F1 schema test (SHAA-333): every object of data contract v0.9.5-final F1-2..F1-7 exists with the contract's type
-- and nullability. "N" = NOT NULL, "Y" = nullable. Conditional "wajib untuk ..." columns are nullable here and guarded by CHECKs (55_f1_constraints.sql).
\set ON_ERROR_STOP on

CREATE TEMP TABLE f1_kolom (skema text, tabel text, kolom text, tipe text, nullable text);
INSERT INTO f1_kolom VALUES
 -- D1
 ('public','downstream_pool','aging_ambang_detik','integer','N'),
 ('public','request','aging_ambang_detik','integer','Y'),
 ('public','request','prioritas_diubah_at','timestamp with time zone','Y'),
 ('public','request_status_history','prioritas_dari','integer','Y'),
 ('public','request_status_history','prioritas_ke','integer','Y'),
 -- D2
 ('public','downstream_pool_rute','alokasi_jenis','text','Y'),
 ('public','downstream_pool_rute','alokasi_nilai','numeric(12,2)','Y'),
 ('public','downstream_pool_rute','sifat','text','Y'),
 ('public','downstream_pool_rute','boleh_pinjam','boolean','N'),
 ('public','transaction_log','sumber_anggaran','character varying(36)','Y'),
 ('public','transaction_log','downstream_pool_id','uuid','Y'),
 ('public','request','reject_lingkup','text','Y'),
 -- D3
 ('public','request','webhook_not_before','timestamp with time zone','Y'),
 -- D4
 ('public','data_balikan_batch','kirim_berikutnya_at','timestamp with time zone','Y'),
 ('public','data_balikan_batch','config_ref_version','integer','N'),
 ('public','data_balikan_item','aktif','boolean','N'),
 -- audit_akses (6.4)
 ('public','audit_akses','sumber','text','N'),
 ('public','audit_akses','target_pengguna_id','uuid','Y'),
 ('public','audit_akses','pelaku_kedua_id','uuid','Y'),
 ('public','audit_akses','target_restore_id','uuid','Y'),
 ('public','audit_akses','kunci_hmac_versi','integer','Y'),
 -- D6 arsip_manifest (6.2)
 ('public','arsip_manifest','manifest_id','uuid','N'),
 ('public','arsip_manifest','tabel_sumber','text','N'),
 ('public','arsip_manifest','provider_id','uuid','Y'),
 ('public','arsip_manifest','tanggal_wib','date','N'),
 ('public','arsip_manifest','objek_kunci','character varying(512)','N'),
 ('public','arsip_manifest','format','character varying(32)','N'),
 ('public','arsip_manifest','versi_contract','character varying(16)','N'),
 ('public','arsip_manifest','jumlah_baris','bigint','N'),
 ('public','arsip_manifest','jumlah_dikecualikan_h02','integer','N'),
 ('public','arsip_manifest','kode_transaksi_min','uuid','Y'),
 ('public','arsip_manifest','kode_transaksi_maks','uuid','Y'),
 ('public','arsip_manifest','sha256','character(64)','N'),
 ('public','arsip_manifest','ukuran_byte','bigint','N'),
 ('public','arsip_manifest','kunci_hmac_versi','integer','Y'),
 ('public','arsip_manifest','nik_mode','text','Y'),
 ('public','arsip_manifest','payload_mode','text','Y'),
 ('public','arsip_manifest','nik_kek_id','character varying(100)','Y'),
 ('public','arsip_manifest','enkripsi_format_versi','integer','Y'),
 ('public','arsip_manifest','dek_terbungkus','bytea','Y'),
 ('public','arsip_manifest','status','text','N'),
 ('public','arsip_manifest','legal_hold','boolean','N'),
 ('public','arsip_manifest','ditulis_at','timestamp with time zone','Y'),
 ('public','arsip_manifest','diverifikasi_at','timestamp with time zone','Y'),
 ('public','arsip_manifest','online_dihapus_at','timestamp with time zone','Y'),
 ('public','arsip_manifest','objek_dihapus_at','timestamp with time zone','Y'),
 -- D6 arsip_restore_permintaan (6.3)
 ('public','arsip_restore_permintaan','restore_id','uuid','N'),
 ('public','arsip_restore_permintaan','diminta_oleh','uuid','N'),
 ('public','arsip_restore_permintaan','audit_id','uuid','N'),
 ('public','arsip_restore_permintaan','provider_id','uuid','N'),
 ('public','arsip_restore_permintaan','rentang_mulai','date','N'),
 ('public','arsip_restore_permintaan','rentang_selesai','date','N'),
 ('public','arsip_restore_permintaan','nik_hmac_per_versi','jsonb','Y'),
 ('public','arsip_restore_permintaan','nik_tersamar','character(4)','Y'),
 ('public','arsip_restore_permintaan','alasan','character varying(500)','N'),
 ('public','arsip_restore_permintaan','status','text','N'),
 ('public','arsip_restore_permintaan','diminta_at','timestamp with time zone','N'),
 ('public','arsip_restore_permintaan','mulai_at','timestamp with time zone','Y'),
 ('public','arsip_restore_permintaan','siap_at','timestamp with time zone','Y'),
 ('public','arsip_restore_permintaan','kedaluwarsa_at','timestamp with time zone','Y'),
 ('public','arsip_restore_permintaan','jumlah_baris_pulih','bigint','Y'),
 ('public','arsip_restore_permintaan','error_terakhir','character varying(200)','Y'),
 -- D6 arsip_pulih: restore_id on top of the transaction_log columns (checked generically below)
 ('arsip_pulih','transaction_log','restore_id','uuid','N'),
 -- D7 (7.1, 7.2, 7.2a, 7.3)
 ('public','admin_session','mfa_terverifikasi_at','timestamp with time zone','Y'),
 ('public','pengguna_mfa','pengguna_id','uuid','N'),
 ('public','pengguna_mfa','secret_ciphertext','bytea','N'),
 ('public','pengguna_mfa','secret_dek_terbungkus','bytea','N'),
 ('public','pengguna_mfa','secret_nonce','bytea','N'),
 ('public','pengguna_mfa','kek_id','character varying(100)','N'),
 ('public','pengguna_mfa','status','text','N'),
 ('public','pengguna_mfa','langkah_terakhir','bigint','N'),
 ('public','pengguna_mfa','didaftarkan_at','timestamp with time zone','N'),
 ('public','pengguna_mfa','dikonfirmasi_at','timestamp with time zone','Y'),
 ('public','kode_pemulihan_mfa','kode_id','uuid','N'),
 ('public','kode_pemulihan_mfa','pengguna_id','uuid','N'),
 ('public','kode_pemulihan_mfa','kode_hash','character varying(255)','N'),
 ('public','kode_pemulihan_mfa','dibuat_at','timestamp with time zone','N'),
 ('public','kode_pemulihan_mfa','dipakai_at','timestamp with time zone','Y'),
 ('public','kode_daftar_mfa','kode_id','uuid','N'),
 ('public','kode_daftar_mfa','pengguna_id','uuid','N'),
 ('public','kode_daftar_mfa','kode_hash','character(64)','N'),
 ('public','kode_daftar_mfa','diterbitkan_oleh','uuid','N'),
 ('public','kode_daftar_mfa','diterbitkan_at','timestamp with time zone','N'),
 ('public','kode_daftar_mfa','kedaluwarsa_at','timestamp with time zone','N'),
 ('public','kode_daftar_mfa','gagal_count','integer','N'),
 ('public','kode_daftar_mfa','dipakai_at','timestamp with time zone','Y'),
 ('public','kode_daftar_mfa','gugur_at','timestamp with time zone','Y'),
 ('public','kode_daftar_mfa','gugur_alasan','text','Y');

SELECT t.ok(t.hitung('SELECT count(*) FROM f1_kolom') = 88, 'F1 column list is loaded (88 columns)');

SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM f1_kolom k
   WHERE NOT EXISTS (
     SELECT 1 FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = k.skema AND c.relname = k.tabel AND a.attname = k.kolom AND a.attnum > 0 AND NOT a.attisdropped
        AND format_type(a.atttypid, a.atttypmod) = k.tipe AND (NOT a.attnotnull) = (k.nullable = 'Y'))$q$) = 0,
  'every F1 column exists with the contract type and nullability');

-- arsip_pulih.transaction_log = transaction_log (same columns, same types) + restore_id
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM (
    SELECT a.attname, format_type(a.atttypid, a.atttypmod) FROM pg_attribute a WHERE a.attrelid = 'public.transaction_log'::regclass AND a.attnum > 0 AND NOT a.attisdropped
    EXCEPT
    SELECT a.attname, format_type(a.atttypid, a.atttypmod) FROM pg_attribute a WHERE a.attrelid = 'arsip_pulih.transaction_log'::regclass AND a.attnum > 0 AND NOT a.attisdropped) d$q$) = 0,
  'arsip_pulih.transaction_log has every transaction_log column with the same type (AR-06)');

-- F0 objects keep their shape: no column dropped, retyped or re-keyed (compare with the F0 column list)
SELECT t.ok(t.hitung($q$SELECT count(*) FROM t.kolom_contract k WHERE NOT EXISTS (SELECT 1 FROM information_schema.columns c
  WHERE c.table_schema = 'public' AND c.table_name = k.tabel AND c.column_name = k.kolom)$q$) = 0,
  'F0 contract columns all still exist after F1');
SELECT t.ok((SELECT array_agg(a.attname::text ORDER BY a.attname) FROM pg_index i JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY (i.indkey)
              WHERE i.indrelid = 'public.request'::regclass AND i.indisprimary) = ARRAY['request_id'], 'request primary key unchanged');
SELECT t.ok((SELECT array_agg(a.attname::text ORDER BY a.attname) FROM pg_index i JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY (i.indkey)
              WHERE i.indrelid = 'public.transaction_log'::regclass AND i.indisprimary) = ARRAY['kode_transaksi'], 'transaction_log primary key unchanged');

-- Primary / unique keys of the new tables
SELECT t.ok(EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = 'arsip_manifest_satuan_uq' AND indexdef LIKE '%NULLS NOT DISTINCT%'),
  'arsip_manifest unique (tabel_sumber, provider_id, tanggal_wib) treats NULL provider as a value');
SELECT t.ok(EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = 'kode_daftar_mfa_aktif_uq' AND indexdef LIKE '%WHERE%dipakai_at IS NULL%gugur_at IS NULL%'),
  'kode_daftar_mfa: unique partial index, one active code per user (MF-10)');
SELECT t.ok(EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = 'data_balikan_item_aktif_uq' AND indexdef LIKE '%(request_id)%WHERE aktif%'),
  'data_balikan_item: unique partial index, one active item per request (DB-01)');

-- Platform settings seeded with the contract defaults
SELECT t.ok(t.hitung($q$SELECT count(*) FROM konfigurasi_platform WHERE (kunci, nilai) IN (
  ('prioritas_maks', '5'::jsonb), ('webhook_batas_percobaan', '6'::jsonb), ('webhook_backoff_base_detik', '30'::jsonb),
  ('webhook_backoff_cap_detik', '7200'::jsonb), ('webhook_jitter_persen', '20'::jsonb), ('restore_masa_berlaku_hari', '14'::jsonb),
  ('restore_rentang_dengan_nik_maks_bulan', '12'::jsonb), ('restore_rentang_tanpa_nik_maks_hari', '7'::jsonb))$q$) = 8,
  'platform settings carry the contract defaults (F1-2.1, F1-4.2, F1-6.3)');

-- Version-2 provider config documents are accepted next to version 1 (V-05)
BEGIN;
SELECT t.jalan($$INSERT INTO provider_config_versi VALUES ('20000000-0000-7000-8000-000000000001', 2, 2, 'metadata_only',
  '{"data_balikan": {"data_balikan_aktif": false}}', now(), '00000000-0000-7000-8000-000000000001')$$, 'config_schema_version 2 accepted');
ROLLBACK;
