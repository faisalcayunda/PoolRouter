-- F4 secrets, part 1: the provider secret namespace becomes global per name (ARC-48, AD1).
-- Source: products/poolrouter/specs/f0-admin-api/spec.md (AC-8..AC-11) and
-- products/poolrouter/grilling/2026-11-05-f0-admin-api.md AD1.
--
-- Why the key changes: `createProvider` must reject a `{{secret.X}}` that does not exist, but a provider's
-- id only exists after it is created; `listSecrets.dipakai_oleh_provider_count` and the 409 from
-- `deleteSecret` both assume one secret is used by several providers. With `(provider_id, nama_secret)`
-- neither statement can be written. The table therefore gets one row per name, and the provider side
-- references it from `provider_config_versi.dokumen` (`{{nama}}` placeholders), which is already where
-- the template lives.
--
-- What does not change: the envelope columns (ciphertext, wrapped DEK, nonce, kek_id) stay exactly as F0
-- defined them, `pr_admin_api` stays write-only, and `pr_api`/`pr_worker` still read the ciphertext to
-- decrypt in process. The envelope is sealed by `core::secret::Brankas` (AES-256-GCM, one DEK per value,
-- name as associated data), so the length checks added here state the format that module already writes.
--
-- Runs as poolrouter_owner, after 20261105*; grants for the recreated table are in this file because
-- DROP TABLE takes the F0 grants with it. Rollback: backend/db/rollback/20261105000003_f4_secret_global.down.sql.

-- Refuse to throw away secret values: the recreated table cannot keep rows that have no global name yet.
-- An operator who has accepted the loss (no production value exists in F0; the Dukcapil values are entered
-- through putSecret during the PoC) sets poolrouter.izinkan_migrasi_secret_global = 'ya' for this run.
DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM provider_secret)
     AND coalesce(current_setting('poolrouter.izinkan_migrasi_secret_global', true), '') <> 'ya' THEN
    RAISE EXCEPTION 'migrasi ditolak: provider_secret berisi data; set poolrouter.izinkan_migrasi_secret_global = ya setelah persetujuan Klaus dan owner'
      USING ERRCODE = 'insufficient_privilege';
  END IF;
END $$;

DROP TABLE provider_secret;

CREATE TABLE provider_secret (
  nama_secret text PRIMARY KEY CHECK (nama_secret ~ '^[A-Za-z][A-Za-z0-9_]{0,63}$'),   -- global, no provider_id
  secret_versi integer NOT NULL DEFAULT 1 CHECK (secret_versi >= 1),
  secret_ciphertext bytea NOT NULL CHECK (octet_length(secret_ciphertext) > 0),
  secret_dek_terbungkus bytea NOT NULL CHECK (octet_length(secret_dek_terbungkus) = 60),   -- 12 nonce + 32 DEK + 16 tag
  secret_nonce bytea NOT NULL CHECK (octet_length(secret_nonce) = 12),
  kek_id varchar(100) NOT NULL CHECK (length(btrim(kek_id)) > 0),
  diubah_at timestamptz NOT NULL DEFAULT now(),
  diubah_oleh uuid NOT NULL REFERENCES admin_user (pengguna_id)
);

-- pr_api and pr_worker decrypt in process, so they read the envelope; they never write it.
GRANT SELECT ON provider_secret TO pr_api, pr_worker;
-- pr_admin_api: write-only envelope. It may read the name/version/stamps for listSecrets (AC-9), and it
-- needs DELETE because deleteSecret is a real delete, not a status change (AC-10).
GRANT SELECT (nama_secret, secret_versi, diubah_at, diubah_oleh) ON provider_secret TO pr_admin_api;
GRANT INSERT, DELETE ON provider_secret TO pr_admin_api;
GRANT UPDATE (secret_versi, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, diubah_at, diubah_oleh)
  ON provider_secret TO pr_admin_api;

-- audit_konfigurasi: every write to the secret store is audited under its own entity kind (AC-5).
ALTER TABLE audit_konfigurasi DROP CONSTRAINT audit_konfigurasi_entitas_jenis_check;
ALTER TABLE audit_konfigurasi ADD CONSTRAINT audit_konfigurasi_entitas_jenis_check CHECK (entitas_jenis IN (
  'client', 'kredensial', 'downstream_endpoint', 'downstream_pool', 'upstream_pool', 'provider',
  'client_webhook', 'webhook_domain_allowlist', 'provider_config', 'endpoint_config',
  'endpoint_metadata_field', 'konfigurasi_platform', 'admin_user', 'endpoint_blend', 'provider_secret'));
