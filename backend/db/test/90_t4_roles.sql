-- F4 admin API, T4 (ARC-50): webhook + allowlist privileges and the nullable-secret shape.
-- Pinned outside 30_roles.sql so the F0 file stays frozen; that file's broad
-- "no DELETE outside the allowed configuration tables" assertion lists
-- webhook_domain_allowlist for the reason given here (AC-24).
\set ON_ERROR_STOP 1

-- ============ AC-24: deleteWebhookAllowlistEntry is a real DELETE, PA only
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.role_table_grants
   WHERE table_name = 'webhook_domain_allowlist' AND grantee = 'pr_admin_api'
     AND privilege_type = 'DELETE'$q$) = 1, 'AC-24: pr_admin_api may DELETE webhook_domain_allowlist');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.role_table_grants
   WHERE table_name = 'webhook_domain_allowlist' AND privilege_type = 'DELETE'
     AND grantee NOT IN ('pr_admin_api', 'poolrouter_owner')$q$) = 0, 'AC-24: no other application role may DELETE the allowlist');

-- ============ AC-23: PA reads the delivery log; no other application role does
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.role_table_grants
   WHERE table_name = 'webhook_log' AND grantee = 'pr_admin_api' AND privilege_type = 'SELECT'$q$) = 1, 'AC-23: pr_admin_api may SELECT webhook_log');

BEGIN;
SET LOCAL ROLE pr_admin_api;
SELECT t.jalan('SELECT domain, aktif FROM webhook_domain_allowlist', 'AC-24: the admin API lists the allowlist');
SELECT t.jalan($$UPDATE webhook_domain_allowlist SET aktif = false WHERE false$$, 'AC-24: the admin API flips aktif');
SELECT t.jalan('SELECT webhook_url, aktif, secret_versi FROM client_webhook', 'AC-21: the admin API reads the config row');
SELECT t.gagal('SELECT secret_ciphertext FROM client_webhook', '42501', 'AC-22: pr_admin_api never reads the webhook secret envelope');
SELECT t.gagal('SELECT * FROM client_webhook', '42501', 'AC-22: no whole-row read of the config row');
ROLLBACK;

-- ============ AC-22: putWebhookConfig may create the row before any rotation
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM information_schema.columns
   WHERE table_name = 'client_webhook' AND column_name IN ('secret_versi', 'secret_ciphertext', 'secret_dek_terbungkus', 'secret_nonce', 'kek_id')
     AND is_nullable = 'YES'$q$) = 5, 'AC-22: the five secret columns are nullable so a config row can exist without a secret');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM pg_constraint
   WHERE conrelid = 'client_webhook'::regclass AND conname = 'ck_webhook_secret_lengkap' AND contype = 'c'$q$) = 1, 'AC-22: ck_webhook_secret_lengkap keeps the secret all-or-none');

BEGIN;
SET LOCAL ROLE pr_admin_api;
SELECT t.gagal($$
  INSERT INTO client_webhook (client_id, webhook_url, aktif, secret_versi, diubah_oleh)
  SELECT c.client_id, 'https://mpm.example.go.id/hook', true, 1, a.pengguna_id
    FROM client c, admin_user a LIMIT 1$$, '23514', 'AC-22: a partial secret (versi without envelope) is rejected');
ROLLBACK;
