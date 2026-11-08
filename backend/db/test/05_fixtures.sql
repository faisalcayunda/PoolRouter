-- Synthetic data only (no real NIK; 16-digit values with prefix 00 per contract 6.1).
BEGIN;
INSERT INTO admin_user (pengguna_id, username, password_hash) VALUES
  ('00000000-0000-7000-8000-000000000001', 'admin1', '$argon2id$synthetic'),
  ('00000000-0000-7000-8000-000000000002', 'admin2', '$argon2id$synthetic'),
  ('00000000-0000-7000-8000-000000000003', 'auditor1', '$argon2id$synthetic'),
  ('00000000-0000-7000-8000-000000000004', 'operator1', '$argon2id$synthetic');
INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh, disetujui_oleh) VALUES
  ('00000000-0000-7000-8000-000000000001', 'platform_admin', '00000000-0000-7000-8000-000000000001', NULL),
  ('00000000-0000-7000-8000-000000000002', 'platform_admin', '00000000-0000-7000-8000-000000000001', NULL),
  ('00000000-0000-7000-8000-000000000003', 'auditor', '00000000-0000-7000-8000-000000000001', '00000000-0000-7000-8000-000000000002'),
  ('00000000-0000-7000-8000-000000000004', 'operator', '00000000-0000-7000-8000-000000000001', '00000000-0000-7000-8000-000000000002');

INSERT INTO client (client_id, nama) VALUES
  ('10000000-0000-7000-8000-000000000001', 'client-x'), ('10000000-0000-7000-8000-000000000002', 'client-y');
INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, dibuat_oleh) VALUES
  ('11000000-0000-7000-8000-000000000001', '10000000-0000-7000-8000-000000000001', 'api_key', '\x01', '00000000-0000-7000-8000-000000000001'),
  ('11000000-0000-7000-8000-000000000002', '10000000-0000-7000-8000-000000000001', 'oauth2_client_credentials', '\x02', '00000000-0000-7000-8000-000000000001'),
  ('11000000-0000-7000-8000-000000000003', '10000000-0000-7000-8000-000000000002', 'api_key', '\x03', '00000000-0000-7000-8000-000000000001');

INSERT INTO provider (provider_id, nama, data_balikan_aktif) VALUES
  ('20000000-0000-7000-8000-000000000001', 'dukcapil-sintetis', true),
  ('20000000-0000-7000-8000-000000000002', 'provider-lain', false);
INSERT INTO provider_config_versi (provider_id, versi, config_schema_version, log_mode, dokumen, dibuat_oleh) VALUES
  ('20000000-0000-7000-8000-000000000001', 1, 1, 'metadata_only', '{}', '00000000-0000-7000-8000-000000000001');
INSERT INTO provider_secret (nama_secret, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, diubah_oleh) VALUES
  ('PASSWORD', '\xaa', decode(repeat('bb', 60), 'hex'), decode(repeat('cc', 12), 'hex'), 'kek-1', '00000000-0000-7000-8000-000000000001');
INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, kuota_harian) VALUES
  ('30000000-0000-7000-8000-000000000001', '20000000-0000-7000-8000-000000000001', 'up-1', 1000);
INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) VALUES
  ('40000000-0000-7000-8000-000000000001', 'down-1', 1);
INSERT INTO downstream_pool_rute VALUES ('40000000-0000-7000-8000-000000000001', '30000000-0000-7000-8000-000000000001');
INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, downstream_pool_id) VALUES
  ('50000000-0000-7000-8000-000000000001', 'ep-1', '20000000-0000-7000-8000-000000000001', '40000000-0000-7000-8000-000000000001');
INSERT INTO endpoint_config_versi (downstream_endpoint_id, versi, config_schema_version, dokumen, dibuat_oleh) VALUES
  ('50000000-0000-7000-8000-000000000001', 1, 1, '{"parameter_literal": {"TRESHOLD": "80"}}', '00000000-0000-7000-8000-000000000001');
INSERT INTO endpoint_metadata_field (downstream_endpoint_id, nama_field, tipe, wajib, klasifikasi, kirim_ke_data_balikan, deskripsi, versi) VALUES
  ('50000000-0000-7000-8000-000000000001', 'status_penerima_manfaat', 'string', true, 'Rahasia', true, 'x', 1),
  ('50000000-0000-7000-8000-000000000001', 'referensi_client', 'string', false, 'Terbatas', false, 'x', 1);
INSERT INTO client_endpoint VALUES ('10000000-0000-7000-8000-000000000001', '50000000-0000-7000-8000-000000000001');
INSERT INTO client_webhook (client_id, webhook_url, aktif, secret_versi, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, diubah_oleh) VALUES
  ('10000000-0000-7000-8000-000000000001', 'https://hook.example.test/x', true, 1, '\x11', '\x22', '\x33', 'kek-1', '00000000-0000-7000-8000-000000000001');
COMMIT;
