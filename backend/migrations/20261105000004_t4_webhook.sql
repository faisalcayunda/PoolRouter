-- F4 admin API: webhook config nullable secrets, allowlist delete, log select for PA (ARC-50 T4).
-- Source: products/poolrouter/specs/f0-admin-api/spec.md (AC-21–24) and DESIGN.md §3.2.
--
-- Why the changes: AC-22 requires the first rotateWebhookSecret to return versi 1, but a row can be
-- created by putWebhookConfig without yet having a secret. The simplest model is to make the four
-- secret columns nullable and enforce an all-or-none rule with a CHECK constraint; the response schema
-- allows secret_versi nullable exactly for this state. AC-24 requires DELETE on webhook_domain_allowlist,
-- and AC-23 lists PA as allowed for listWebhookLog: pr_admin_api therefore needs SELECT on webhook_log.
-- All changes are reversible, so the rollback refuses if there are rows with empty secrets that would
-- violate the restored NOT NULL constraints.
--
-- Runs as poolrouter_owner, after the F3 F4 rolls. Rollback: backend/db/rollback/20261105000004_t4_webhook.down.sql.

-- Allow PUT/Webhook to create a config row before any secret is generated.
ALTER TABLE client_webhook ALTER COLUMN secret_versi DROP NOT NULL;
ALTER TABLE client_webhook ALTER COLUMN secret_ciphertext DROP NOT NULL;
ALTER TABLE client_webhook ALTER COLUMN secret_dek_terbungkus DROP NOT NULL;
ALTER TABLE client_webhook ALTER COLUMN secret_nonce DROP NOT NULL;
ALTER TABLE client_webhook ALTER COLUMN kek_id DROP NOT NULL;

-- Keep it consistent: either fully configured or not at all.
ALTER TABLE client_webhook DROP CONSTRAINT IF EXISTS ck_webhook_secret_lengkap;
ALTER TABLE client_webhook ADD CONSTRAINT ck_webhook_secret_lengkap CHECK (
  (secret_versi IS NULL AND secret_ciphertext IS NULL AND secret_dek_terbungkus IS NULL AND secret_nonce IS NULL AND kek_id IS NULL)
  OR
  (secret_versi IS NOT NULL AND secret_ciphertext IS NOT NULL AND secret_dek_terbungkus IS NOT NULL AND secret_nonce IS NOT NULL AND kek_id IS NOT NULL)
);

-- DELETE grant: AC-24 says deleteWebhookAllowlistEntry removes a row idempotently.
GRANT DELETE ON webhook_domain_allowlist TO pr_admin_api;

-- SELECT grant: AC-23 allows PA to read their own client's webhook delivery log.
GRANT SELECT ON webhook_log TO pr_admin_api;
