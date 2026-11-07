-- Rollback of 20261107000001_f1_webhook_retry_klaim.sql (ARC-120).
-- Restores F0's 4-param fn_klaim_webhook/fn_tutup_klaim_macet (0->1 claim only) and revokes the
-- konfigurasi_platform / fn_konfigurasi_int grants added for the retry job. Refuses while any row has
-- webhook_attempt_count > 1 or a pending webhook_not_before, because the F0 shape cannot represent a
-- row mid-retry.

DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM request WHERE webhook_attempt_count > 1 OR webhook_not_before IS NOT NULL)
     AND coalesce(current_setting('poolrouter.izinkan_hapus_data_f1_webhook', true), '') <> 'ya' THEN
    RAISE EXCEPTION 'rollback ditolak: ada baris webhook sedang retry (set poolrouter.izinkan_hapus_data_f1_webhook = ya setelah persetujuan Klaus dan owner)';
  END IF;
END $$;

DROP FUNCTION IF EXISTS fn_klaim_webhook(uuid, timestamptz, integer, text, integer);
DROP FUNCTION IF EXISTS fn_tutup_klaim_macet(timestamptz, integer);

CREATE FUNCTION fn_klaim_webhook(
  p_request_id uuid,
  p_sekarang timestamptz,
  p_http_status integer,
  p_galat text)
RETURNS integer
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
  n integer;
BEGIN
  UPDATE public.request
     SET webhook_attempt_count = 1,
         webhook_last_at = p_sekarang,
         webhook_status = CASE WHEN p_galat IS NULL THEN webhook_status ELSE 'gagal' END,
         webhook_last_http_status = p_http_status,
         webhook_last_error = p_galat
   WHERE request_id = p_request_id
     AND webhook_status = 'menunggu'
     AND webhook_attempt_count = 0
     AND (p_galat IS NULL OR webhook_last_at < p_sekarang - interval '5 minutes');
  GET DIAGNOSTICS n = ROW_COUNT;
  RETURN n;
END $$;
REVOKE ALL ON FUNCTION fn_klaim_webhook(uuid, timestamptz, integer, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION fn_klaim_webhook(uuid, timestamptz, integer, text) TO pr_webhook;

CREATE FUNCTION fn_tutup_klaim_macet(p_sekarang timestamptz, p_detik integer)
RETURNS bigint
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
  n bigint;
BEGIN
  UPDATE public.request
     SET webhook_status = 'gagal',
         webhook_last_error = 'klaim_kedaluwarsa'
   WHERE webhook_status = 'menunggu'
     AND webhook_attempt_count = 1
     AND webhook_last_at < p_sekarang - make_interval(secs => p_detik);
  GET DIAGNOSTICS n = ROW_COUNT;
  RETURN n;
END $$;
REVOKE ALL ON FUNCTION fn_tutup_klaim_macet(timestamptz, integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION fn_tutup_klaim_macet(timestamptz, integer) TO pr_webhook;

CREATE OR REPLACE VIEW v_request_webhook WITH (security_barrier = true) AS
  SELECT r.request_id, r.client_id, r.status, r.attempt_count, r.cancel_reason, r.failure_reason, r.completed_at,
         r.webhook_status, r.webhook_attempt_count, r.output_payload, r.payload_purged_at
    FROM request r
   WHERE r.mode = 'async' AND r.status IN ('done', 'failed', 'expired', 'cancelled');

REVOKE EXECUTE ON FUNCTION fn_konfigurasi_int(text, integer) FROM pr_webhook;
REVOKE SELECT (kunci, nilai) ON konfigurasi_platform FROM pr_webhook;
