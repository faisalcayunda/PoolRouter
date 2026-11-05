-- Rollback of 20261105000004_t4_webhook.sql (ARC-50 T4).
-- Restores the F0 NOT NULL secret columns and the original grants. Refuses while a webhook config row
-- has no complete secret, because the F0 shape cannot represent that state.

DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM client_webhook WHERE secret_versi IS NULL OR secret_ciphertext IS NULL)
     AND coalesce(current_setting('poolrouter.izinkan_hapus_data_t4', true), '') <> 'ya' THEN
    RAISE EXCEPTION 'rollback ditolak: ada baris webhook dengan secret belum lengkap (set poolrouter.izinkan_hapus_data_t4 = ya setelah persetujuan Klaus dan owner)';
  END IF;
END $$;

DELETE FROM client_webhook WHERE secret_versi IS NULL OR secret_ciphertext IS NULL;

ALTER TABLE client_webhook DROP CONSTRAINT IF EXISTS ck_webhook_secret_lengkap;

ALTER TABLE client_webhook ALTER COLUMN secret_versi SET NOT NULL;
ALTER TABLE client_webhook ALTER COLUMN secret_ciphertext SET NOT NULL;
ALTER TABLE client_webhook ALTER COLUMN secret_dek_terbungkus SET NOT NULL;
ALTER TABLE client_webhook ALTER COLUMN secret_nonce SET NOT NULL;
ALTER TABLE client_webhook ALTER COLUMN kek_id SET NOT NULL;

REVOKE DELETE ON webhook_domain_allowlist FROM pr_admin_api;
REVOKE SELECT ON webhook_log FROM pr_admin_api;
