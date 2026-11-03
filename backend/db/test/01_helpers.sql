-- Test helpers. Statements run as the CURRENT role, so SET ROLE decides which privileges are exercised.
CREATE SCHEMA IF NOT EXISTS t;
GRANT USAGE ON SCHEMA t TO PUBLIC;

CREATE OR REPLACE FUNCTION t.ok(p_cond boolean, p_nama text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
  IF p_cond IS NOT TRUE THEN RAISE EXCEPTION 'FAIL: %', p_nama; END IF;
  RAISE NOTICE 'ok - %', p_nama;
END $$;

-- The statement must fail with the given SQLSTATE.
CREATE OR REPLACE FUNCTION t.gagal(p_sql text, p_sqlstate text, p_nama text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
  BEGIN
    EXECUTE p_sql;
  EXCEPTION WHEN OTHERS THEN
    IF SQLSTATE = p_sqlstate THEN RAISE NOTICE 'ok - % (%)', p_nama, SQLSTATE; RETURN; END IF;
    RAISE EXCEPTION 'FAIL: % -> expected %, got % (%)', p_nama, p_sqlstate, SQLSTATE, SQLERRM;
  END;
  RAISE EXCEPTION 'FAIL: % -> statement succeeded, expected %', p_nama, p_sqlstate;
END $$;

-- The statement must succeed.
CREATE OR REPLACE FUNCTION t.jalan(p_sql text, p_nama text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
  EXECUTE p_sql;
  RAISE NOTICE 'ok - %', p_nama;
EXCEPTION WHEN OTHERS THEN
  RAISE EXCEPTION 'FAIL: % -> % (%)', p_nama, SQLERRM, SQLSTATE;
END $$;

CREATE OR REPLACE FUNCTION t.hitung(p_sql text) RETURNS bigint LANGUAGE plpgsql AS $$
DECLARE n bigint;
BEGIN EXECUTE p_sql INTO n; RETURN n; END $$;

CREATE TABLE IF NOT EXISTS t.kolom_contract (tabel text, kolom text);

-- Create a request plus its first history row in the current transaction. p overrides the base record.
CREATE OR REPLACE FUNCTION t.buat_request(p jsonb DEFAULT '{}') RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
  base jsonb := jsonb_build_object(
    'request_id', gen_random_uuid(), 'client_id', '10000000-0000-7000-8000-000000000001',
    'credential_id', '11000000-0000-7000-8000-000000000001', 'downstream_endpoint_id', '50000000-0000-7000-8000-000000000001',
    'downstream_pool_id', '40000000-0000-7000-8000-000000000001', 'provider_id', '20000000-0000-7000-8000-000000000001',
    'provider_config_version', 1, 'endpoint_config_version', 1, 'mode', 'async', 'prioritas', 1, 'prioritas_efektif', 1,
    'status', 'queued', 'metadata', '{}'::jsonb, 'received_at', now(), 'ttl_expires_at', now() + interval '1 day',
    'attempt_count', 0, 'webhook_attempt_count', 0, 'dispatch_key', gen_random_uuid());
  rec request;
BEGIN
  rec := jsonb_populate_record(NULL::request, base || p);
  INSERT INTO request SELECT rec.*;
  INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor, alasan)
    VALUES (rec.request_id, 1, NULL, rec.status, 'webserver', rec.reject_reason);
  RETURN rec.request_id;
END $$;
GRANT EXECUTE ON FUNCTION t.buat_request(jsonb) TO PUBLIC;

CREATE TABLE IF NOT EXISTS t.hasil_ekspor (nama text, audit_id uuid, berkas bytea, ekspor_checksum text, jumlah_record integer, hasil text);
GRANT ALL ON t.hasil_ekspor TO PUBLIC;
