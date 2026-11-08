-- Rollback of 20261105000003_f4_secret_global.sql (ARC-48, AD1). Run as poolrouter_owner, before the F3
-- rollback (reverse order): the constraint restored here still carries 'endpoint_blend'.
-- Refuses while the secret store holds rows, because the F0 shape cannot express a global name.

DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM provider_secret)
     AND coalesce(current_setting('poolrouter.izinkan_hapus_data_f4', true), '') <> 'ya' THEN
    RAISE EXCEPTION 'rollback ditolak: provider_secret berisi data (set poolrouter.izinkan_hapus_data_f4 = ya setelah persetujuan Klaus dan owner)';
  END IF;
END $$;

DROP TABLE provider_secret;

-- Back to the F0 shape: one row per provider and name (migrations/20261103000001_config.sql).
DROP TABLE IF EXISTS provider_secret CASCADE;
CREATE TABLE provider_secret (
  provider_id uuid NOT NULL REFERENCES provider (provider_id),
  nama_secret text NOT NULL CHECK (nama_secret ~ '^[A-Za-z][A-Za-z0-9_]{0,63}$'),
  secret_versi integer NOT NULL DEFAULT 1 CHECK (secret_versi >= 1),
  secret_ciphertext bytea NOT NULL,
  secret_dek_terbungkus bytea NOT NULL,
  secret_nonce bytea NOT NULL,
  kek_id varchar(100) NOT NULL,
  diubah_at timestamptz NOT NULL DEFAULT now(),
  diubah_oleh uuid NOT NULL REFERENCES admin_user (pengguna_id),
  PRIMARY KEY (provider_id, nama_secret)
);

-- F0 grants (migrations/20261103000004_grants.sql).
GRANT SELECT ON provider_secret TO pr_api, pr_worker;
GRANT SELECT (provider_id, nama_secret, secret_versi, diubah_at, diubah_oleh) ON provider_secret TO pr_admin_api;
GRANT INSERT ON provider_secret TO pr_admin_api;
GRANT UPDATE (secret_versi, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, diubah_at, diubah_oleh)
  ON provider_secret TO pr_admin_api;

ALTER TABLE audit_konfigurasi DROP CONSTRAINT audit_konfigurasi_entitas_jenis_check;
ALTER TABLE audit_konfigurasi ADD CONSTRAINT audit_konfigurasi_entitas_jenis_check CHECK (entitas_jenis IN (
  'client', 'kredensial', 'downstream_endpoint', 'downstream_pool', 'upstream_pool', 'provider',
  'client_webhook', 'webhook_domain_allowlist', 'provider_config', 'endpoint_config',
  'endpoint_metadata_field', 'konfigurasi_platform', 'admin_user', 'endpoint_blend'));
