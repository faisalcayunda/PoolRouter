-- AC: every table and column of data contract v0.7.10 section 3 (and 5, 8.2) has a counterpart in the migrations.
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM t.kolom_contract k
   WHERE NOT EXISTS (SELECT 1 FROM information_schema.columns c
                      WHERE c.table_schema = 'public' AND c.table_name = k.tabel AND c.column_name = k.kolom)$q$) = 0,
  'all contract columns exist (missing = 0)');
SELECT t.ok(t.hitung('SELECT count(*) FROM t.kolom_contract') = 135, 'contract column list is loaded (135 unique table.column pairs)');

-- Columns of contract 3.3a / 3.2.6 / 3.7 / 4 that the list above cannot derive from a column table.
WITH exp(tabel, kolom) AS (VALUES
  ('upstream_pool','margin_kuota'),('upstream_pool','batas_percobaan'),('upstream_pool','retry_backoff_base_detik'),
  ('upstream_pool','retry_backoff_cap_detik'),('upstream_pool','retry_rasio'),('upstream_pool','retry_min_per_jam'),
  ('upstream_pool','jendela_kuota'),('upstream_pool','reset_kuota_jam'),('upstream_pool','reset_kuota_zona'),
  ('upstream_pool','kuota_harian'),
  ('upstream_attempt_daily','upstream_pool_id'),('upstream_attempt_daily','periode_mulai'),('upstream_attempt_daily','periode_selesai'),
  ('upstream_attempt_daily','periode_transisi'),('upstream_attempt_daily','tanggal'),('upstream_attempt_daily','jumlah'),
  ('upstream_attempt_daily','snapshot_at'),
  ('client','limiter_penolakan_burst'),('client','limiter_penolakan_isi_ulang_per_detik'),
  ('client_kredensial','status'),('client_kredensial','dicabut_at'),('client_kredensial','dicabut_oleh'),('client_kredensial','kedaluwarsa_at'),
  ('admin_user','bahasa_ui'),('admin_user','pengguna_id'),('admin_session','pengguna_id'),
  ('provider_config_versi','config_schema_version'),('endpoint_config_versi','config_schema_version'),
  ('endpoint_metadata_field','nama_field'),('endpoint_metadata_field','tipe'),('endpoint_metadata_field','wajib'),
  ('endpoint_metadata_field','nilai_diizinkan'),('endpoint_metadata_field','aturan_validasi'),('endpoint_metadata_field','klasifikasi'),
  ('endpoint_metadata_field','kirim_ke_data_balikan'),('endpoint_metadata_field','deskripsi'),('endpoint_metadata_field','versi'),
  ('webhook_domain_allowlist','domain'),('webhook_domain_allowlist','aktif'),
  ('audit_konfigurasi','jumlah_cancelled'),('audit_konfigurasi','jumlah_failed'))
SELECT t.ok(count(*) = 0, 'configuration columns exist') FROM exp e
 WHERE NOT EXISTS (SELECT 1 FROM information_schema.columns c WHERE c.table_schema = 'public' AND c.table_name = e.tabel AND c.column_name = e.kolom);

-- i18n (ADR-0003 Addendum A section 7.2): nullable, no default, checked to id / en
SELECT t.ok((SELECT is_nullable = 'YES' AND column_default IS NULL FROM information_schema.columns
              WHERE table_name = 'admin_user' AND column_name = 'bahasa_ui'), 'admin_user.bahasa_ui is NULL-able without default');
SELECT t.gagal($$UPDATE admin_user SET bahasa_ui = 'fr' WHERE username = 'admin1'$$, '23514', 'bahasa_ui only id/en');
SELECT t.jalan($$UPDATE admin_user SET bahasa_ui = 'en' WHERE username = 'admin1'$$, 'bahasa_ui en accepted');
UPDATE admin_user SET bahasa_ui = NULL WHERE username = 'admin1';

-- DQ-34 (d) index (comment v0.7.8 on this issue)
SELECT t.ok(EXISTS (SELECT 1 FROM pg_indexes WHERE tablename = 'transaction_log'
                      AND indexdef LIKE '%(upstream_pool_id, requested_at)%'), 'index (upstream_pool_id, requested_at) on transaction_log');
