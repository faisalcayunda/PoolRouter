-- Verification for k20.sql (SHAA-255). Raises an exception on the first failed check.
CREATE FUNCTION pg_temp.cek(kondisi boolean, nama text) RETURNS void LANGUAGE plpgsql AS $p$
BEGIN
  IF kondisi IS NOT TRUE THEN RAISE EXCEPTION 'FAIL: %', nama; END IF;
  RAISE NOTICE 'ok - %', nama;
END $p$;

DO $$
DECLARE
  k1 uuid := '11000000-0000-7000-8000-0000000000b1';
  k2 uuid := '11000000-0000-7000-8000-0000000000b2';
  cl uuid := '10000000-0000-7000-8000-0000000000b1';
  n bigint;
  p record;
BEGIN
  SELECT count(*) INTO n FROM request WHERE client_id = cl;
  PERFORM pg_temp.cek(n = 25, format('K-20 has 25 requests (got %s)', n));

  SELECT count(*) INTO n FROM request WHERE credential_id = k1;
  PERFORM pg_temp.cek(n = 15, format('K1 has 15 requests (got %s)', n));
  SELECT count(*) INTO n FROM request WHERE credential_id = k1 AND status = 'queued' AND attempt_count = 0;
  PERFORM pg_temp.cek(n = 10, format('K1: 10 queued with attempt 0 (got %s)', n));
  SELECT count(*) INTO n FROM request WHERE credential_id = k1 AND status = 'queued' AND attempt_count = 1;
  PERFORM pg_temp.cek(n = 5, format('K1: 5 queued with attempt 1 (got %s)', n));

  SELECT count(*) INTO n FROM request WHERE credential_id = k2 AND status = 'queued' AND attempt_count = 0;
  PERFORM pg_temp.cek(n = 10, format('K2: 10 queued with attempt 0 (got %s)', n));

  SELECT count(*) INTO n FROM transaction_log tl JOIN request r USING (request_id) WHERE r.credential_id = k1;
  PERFORM pg_temp.cek(n = 5, format('K1: 5 transaction_log rows for retries (got %s)', n));

  -- T-04 on the commit path: every request has history, and the last history row matches the stored status
  SELECT count(*) INTO n FROM request r
   WHERE r.client_id = cl
     AND NOT EXISTS (SELECT 1 FROM request_status_history h
                      WHERE h.request_id = r.request_id AND h.seq = 1 AND h.status_dari IS NULL);
  PERFORM pg_temp.cek(n = 0, 'every K-20 request has its first history row (seq 1)');
  SELECT count(*) INTO n FROM request r
   WHERE r.client_id = cl
     AND (SELECT h.status_ke FROM request_status_history h WHERE h.request_id = r.request_id
           ORDER BY h.seq DESC LIMIT 1) IS DISTINCT FROM r.status;
  PERFORM pg_temp.cek(n = 0, 'last history row matches stored status for every K-20 request');
  SELECT count(*) INTO n FROM request_status_history h JOIN request r USING (request_id)
   WHERE r.client_id = cl AND h.seq = 2;
  PERFORM pg_temp.cek(n = 5, 'five queued->dispatched transitions committed with history');

  -- U-54 preview numbers
  SELECT * INTO p FROM fn_pratinjau_nonaktif('kredensial', k1);
  PERFORM pg_temp.cek(p.akan_cancelled = 10 AND p.akan_failed = 5,
      format('preview K1 is 10 cancelled / 5 failed (got %s / %s)', p.akan_cancelled, p.akan_failed));
  SELECT * INTO p FROM fn_pratinjau_nonaktif('client', cl);
  PERFORM pg_temp.cek(p.akan_cancelled = 20 AND p.akan_failed = 5,
      format('preview client K-20 is 20 cancelled / 5 failed (got %s / %s)', p.akan_cancelled, p.akan_failed));
END $$;
