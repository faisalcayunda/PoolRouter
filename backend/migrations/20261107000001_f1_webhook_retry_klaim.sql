-- F1 webhook retry: klaim n -> n+1, grant konfigurasi_platform, klaim macet generik (ARC-120 AC-5..AC-7).
-- Source: products/poolrouter/specs/f1-webhook-retry-data-balikan/spec.md (T2).
--
-- Why a forward migration instead of editing F0's fn_klaim_webhook/fn_tutup_klaim_macet in place: a
-- database that already applied the F0 migration keeps the 4-param signature with its own GRANT; an
-- in-place edit of the migration file is never re-run by the migrator, so production stays on the old
-- signature while the repo file silently drifts. This migration drops the F0 signature and creates the
-- 5-param n->n+1 version instead, so `main` and any already-migrated environment converge.
--
-- Runs as poolrouter_owner, after F4/T4. Rollback: backend/db/rollback/20261107000001_f1_webhook_retry_klaim.down.sql.

-- WB2a: pr_webhook reads retry policy from konfigurasi_platform.
GRANT SELECT (kunci, nilai) ON konfigurasi_platform TO pr_webhook;
-- webhook.rs reads fn_konfigurasi_int (base/cap/batas_percobaan/jitter) through the pr_webhook pool.
GRANT EXECUTE ON FUNCTION fn_konfigurasi_int(text, integer) TO pr_webhook;

-- v_request_webhook needs webhook_not_before so the outbox scan can find due retries.
CREATE OR REPLACE VIEW v_request_webhook WITH (security_barrier = true) AS
  SELECT r.request_id, r.client_id, r.status, r.attempt_count, r.cancel_reason, r.failure_reason, r.completed_at,
         r.webhook_status, r.webhook_attempt_count, r.webhook_not_before, r.output_payload, r.payload_purged_at
    FROM request r
   WHERE r.mode = 'async' AND r.status IN ('done', 'failed', 'expired', 'cancelled');

-- Drop the F0 4-param claim function; replaced below by the n -> n+1 version.
DROP FUNCTION IF EXISTS fn_klaim_webhook(uuid, timestamptz, integer, text);

CREATE FUNCTION fn_klaim_webhook(
  p_request_id uuid,
  p_sekarang timestamptz,
  p_http_status integer,
  p_galat text,
  p_diharapkan integer DEFAULT 0)
RETURNS integer
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
  n integer;
BEGIN
  UPDATE public.request
     SET webhook_attempt_count = p_diharapkan + 1,
         webhook_last_at = p_sekarang,
         webhook_not_before = NULL,
         webhook_status = CASE WHEN p_galat IS NULL THEN webhook_status ELSE 'gagal' END,
         webhook_last_http_status = p_http_status,
         webhook_last_error = p_galat
   WHERE request_id = p_request_id
     AND webhook_status = 'menunggu'
     AND webhook_attempt_count = p_diharapkan
     AND (webhook_not_before IS NULL OR webhook_not_before <= p_sekarang)
     AND (p_galat IS NULL OR webhook_last_at < p_sekarang - interval '5 minutes');
  GET DIAGNOSTICS n = ROW_COUNT;
  RETURN n;
END $$;
REVOKE ALL ON FUNCTION fn_klaim_webhook(uuid, timestamptz, integer, text, integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION fn_klaim_webhook(uuid, timestamptz, integer, text, integer) TO pr_webhook;

-- fn_tutup_klaim_macet: generalized to any attempt_count >= 1 (not only the F0 first-attempt case),
-- and only while there is no pending webhook_not_before (a scheduled retry is not a stuck claim).
CREATE OR REPLACE FUNCTION fn_tutup_klaim_macet(p_sekarang timestamptz, p_detik integer)
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
     AND webhook_attempt_count >= 1
     AND webhook_not_before IS NULL
     AND webhook_last_at < p_sekarang - make_interval(secs => p_detik);
  GET DIAGNOSTICS n = ROW_COUNT;
  RETURN n;
END $$;
