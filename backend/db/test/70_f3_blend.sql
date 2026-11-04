-- F3 behaviour test (SHAA-354): delta data contract v0.3.2 BC, BL, GB, BP, BA, DQ-60..66 as enforced by the database.
-- Synthetic data only. Setup is committed; every test block runs in a transaction that is rolled back.
\set ON_ERROR_STOP on

CREATE OR REPLACE FUNCTION t.gagal_commit(p_sql text, p_sqlstate text, p_nama text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
  BEGIN EXECUTE p_sql; EXECUTE 'SET CONSTRAINTS ALL IMMEDIATE';
  EXCEPTION WHEN OTHERS THEN
    IF SQLSTATE = p_sqlstate THEN RAISE NOTICE 'ok - % (%)', p_nama, SQLSTATE; RETURN; END IF;
    RAISE EXCEPTION 'FAIL: % -> expected %, got % (%)', p_nama, p_sqlstate, SQLSTATE, SQLERRM;
  END;
  RAISE EXCEPTION 'FAIL: % -> statement succeeded, expected %', p_nama, p_sqlstate;
END $$;
CREATE OR REPLACE FUNCTION t.jalan_commit(p_sql text, p_nama text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN EXECUTE p_sql; EXECUTE 'SET CONSTRAINTS ALL IMMEDIATE'; RAISE NOTICE 'ok - %', p_nama;
EXCEPTION WHEN OTHERS THEN RAISE EXCEPTION 'FAIL: % -> % (%)', p_nama, SQLERRM, SQLSTATE; END $$;

-- ---- setup: a second endpoint and one blend configuration with two components
BEGIN;
INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, downstream_pool_id) VALUES
  ('50000000-0000-7000-8000-000000000002', 'ep-2', '20000000-0000-7000-8000-000000000002', '40000000-0000-7000-8000-000000000001');
INSERT INTO endpoint_blend (endpoint_blend_id, nama) VALUES ('b1000000-0000-7000-8000-000000000001', 'blend_uji');
INSERT INTO endpoint_blend_versi (endpoint_blend_id, config_version, config_schema_version, mode_diizinkan, batas_waktu_detik, dibuat_oleh)
  VALUES ('b1000000-0000-7000-8000-000000000001', 1, 1, 'keduanya', 600, '00000000-0000-7000-8000-000000000001');
INSERT INTO endpoint_blend_komponen VALUES
  ('b1000000-0000-7000-8000-000000000001', 1, 'identitas', '50000000-0000-7000-8000-000000000001', 1, '{"nik":"nik"}'),
  ('b1000000-0000-7000-8000-000000000001', 1, 'pembanding', '50000000-0000-7000-8000-000000000002', 2, '{"nik":"nik"}');
COMMIT;

-- Helper: an async blend (running) with a history row, optionally with both children. p overrides blend columns.
CREATE OR REPLACE FUNCTION t.buat_blend(p jsonb DEFAULT '{}', p_anak boolean DEFAULT true) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
  base jsonb := jsonb_build_object('blend_id', gen_random_uuid(), 'client_id', '10000000-0000-7000-8000-000000000001',
    'credential_id', '11000000-0000-7000-8000-000000000001', 'endpoint_blend_id', 'b1000000-0000-7000-8000-000000000001',
    'endpoint_blend_config_version', 1, 'mode', 'async', 'status', 'berjalan', 'gerbang_dasar_hukum', 'tidak_berlaku',
    'ttl_polling_detik', 86400, 'batas_waktu_at', now() + interval '10 minutes', 'requested_at', now(), 'webhook_attempt_count', 0);
  rec blend;
BEGIN
  rec := jsonb_populate_record(NULL::blend, base || p);
  INSERT INTO blend SELECT rec.*;
  INSERT INTO blend_status_history (blend_id, seq, status_dari, status_ke, aktor, alasan)
    VALUES (rec.blend_id, 1, NULL, rec.status, 'webserver', rec.reject_reason);
  IF p_anak THEN
    PERFORM t.buat_request(jsonb_build_object('blend_id', rec.blend_id, 'mode', rec.mode, 'ttl_expires_at', rec.batas_waktu_at,
      'downstream_endpoint_id', '50000000-0000-7000-8000-000000000001', 'request_id', gen_random_uuid()));
    PERFORM t.buat_request(jsonb_build_object('blend_id', rec.blend_id, 'mode', rec.mode, 'ttl_expires_at', rec.batas_waktu_at,
      'downstream_endpoint_id', '50000000-0000-7000-8000-000000000002', 'provider_id', '20000000-0000-7000-8000-000000000002',
      'request_id', gen_random_uuid()));
  END IF;
  RETURN rec.blend_id;
END $$;
GRANT EXECUTE ON FUNCTION t.buat_blend(jsonb, boolean) TO PUBLIC;

-- ================================================================ configuration (BC-01, BC-02, GB-02 point 1 base)
BEGIN;
SELECT t.gagal($$INSERT INTO endpoint_blend (endpoint_blend_id, nama, status) VALUES (gen_random_uuid(), 'blend_aktif_langsung', 'aktif')$$, '23514',
  'GB-02: a new endpoint_blend cannot be created aktif');
SELECT t.gagal($$INSERT INTO endpoint_blend (endpoint_blend_id, nama) VALUES (gen_random_uuid(), 'Blend-Besar')$$, '23514', 'nama is snake_case ASCII');
SELECT t.gagal($$UPDATE endpoint_blend SET status = 'aktif' WHERE nama = 'blend_uji'$$, '23514', 'aktif needs an active config version');
SELECT t.gagal($$UPDATE endpoint_blend_versi SET batas_waktu_detik = 5$$, '23000', 'BC-01: a snapshot never changes');
SELECT t.gagal($$DELETE FROM endpoint_blend_komponen$$, '23000', 'BC-01: components are never deleted');
SELECT t.gagal($$INSERT INTO endpoint_blend_versi (endpoint_blend_id, config_version, config_schema_version, mode_diizinkan, batas_waktu_detik, dibuat_oleh)
  VALUES ('b1000000-0000-7000-8000-000000000001', 2, 1, 'sync', 600, '00000000-0000-7000-8000-000000000001')$$, '23514',
  'batas_waktu_detik must be NULL for sync only');
SELECT t.gagal($$INSERT INTO endpoint_blend_komponen VALUES ('b1000000-0000-7000-8000-000000000001', 1, 'dobel', '50000000-0000-7000-8000-000000000001', 9, '{}')$$,
  '23505', 'BC-02: the same endpoint twice is refused');
SELECT t.gagal($$INSERT INTO endpoint_blend_komponen VALUES ('b1000000-0000-7000-8000-000000000001', 1, 'seri', gen_random_uuid(), 1, '{}')$$,
  '23503', 'a component must be an existing downstream endpoint');
ROLLBACK;

BEGIN;
INSERT INTO endpoint_blend (endpoint_blend_id, nama) VALUES ('b1000000-0000-7000-8000-000000000009', 'blend_satu_komponen');
INSERT INTO endpoint_blend_versi (endpoint_blend_id, config_version, config_schema_version, mode_diizinkan, dibuat_oleh)
  VALUES ('b1000000-0000-7000-8000-000000000009', 1, 1, 'sync', '00000000-0000-7000-8000-000000000001');
INSERT INTO endpoint_blend_komponen VALUES ('b1000000-0000-7000-8000-000000000009', 1, 'satu', '50000000-0000-7000-8000-000000000001', 1, '{}');
SELECT t.gagal($$SET CONSTRAINTS ALL IMMEDIATE$$, '23000', 'ADR-0007 D1: a snapshot with fewer than 2 components is refused at commit');
ROLLBACK;

-- ================================================================ blend rows: enums, pairing, gate evidence (DQ-62, DQ-63 c)
BEGIN;
SELECT t.gagal($$SELECT t.buat_blend('{"status":"selesai"}', false)$$, '23514', 'DQ-62: blend status outside the six values');
SELECT t.gagal($$SELECT t.buat_blend('{"status":"rejected","reject_reason":"komponen_gagal","reject_http_status":422,"completed_at":"2026-11-05T00:00:00Z","gerbang_dasar_hukum":"tidak_berlaku"}', false)$$,
  '23514', 'DQ-62: reject_reason outside section 4.3');
SELECT t.jalan($$SELECT t.buat_blend('{"status":"rejected","reject_reason":"gerbang_blend_tertutup","reject_http_status":403,"completed_at":"2026-11-05T00:00:00Z","gerbang_dasar_hukum":"ditolak","webhook_status":"tidak_dikirim"}', false)$$,
  'gerbang_blend_tertutup is valid in blend.reject_reason with 403 and evidence ditolak');
ROLLBACK;
BEGIN;
SELECT t.gagal($$SELECT t.buat_blend('{"status":"rejected","reject_reason":"gerbang_blend_tertutup","reject_http_status":503,"completed_at":"2026-11-05T00:00:00Z","gerbang_dasar_hukum":"ditolak","webhook_status":"tidak_dikirim"}', false)$$,
  '23514', 'DQ-63 c: gerbang_blend_tertutup only with 403');
ROLLBACK;
BEGIN;
SELECT t.gagal($$SELECT t.buat_blend('{"status":"rejected","reject_reason":"kuota_habis","reject_http_status":429,"completed_at":"2026-11-05T00:00:00Z","gerbang_dasar_hukum":"ditolak","webhook_status":"tidak_dikirim"}', false)$$,
  '23514', 'DQ-63 c: gerbang_dasar_hukum ditolak only with gerbang_blend_tertutup');
ROLLBACK;
BEGIN;
SELECT t.gagal($$SELECT t.buat_blend('{"status":"rejected","reject_reason":"gerbang_blend_tertutup","reject_http_status":403,"completed_at":"2026-11-05T00:00:00Z","gerbang_dasar_hukum":"diizinkan","webhook_status":"tidak_dikirim"}', false)$$,
  '23514', 'DQ-63 c: gerbang_blend_tertutup needs evidence ditolak');
ROLLBACK;
BEGIN;
SELECT t.gagal($$SELECT t.buat_blend('{"idempotency_key":"k1"}', false)$$, '23514', 'chk_blend_idem: key and fingerprint go together');
SELECT t.gagal($$SELECT t.buat_blend('{"mode":"sync","ttl_polling_detik":86400}', false)$$, '23514', 'sync blend has no polling window');
SELECT t.gagal($$SELECT t.buat_blend('{"batas_waktu_at":"2000-01-01T00:00:00Z"}', false)$$, '23514', 'batas_waktu_at after requested_at');
ROLLBACK;

-- ================================================================ parent-child (BL-01..BL-03, DQ-60)
BEGIN;
SELECT t.jalan_commit($$SELECT t.buat_blend()$$, 'BL-02: a blend with one child per component is accepted');
ROLLBACK;
BEGIN;
SELECT t.gagal_commit($$SELECT t.buat_blend('{}', false)$$, '23000', 'DQ-60 b: a running blend with no children is refused at commit');
ROLLBACK;
BEGIN;
SELECT t.buat_blend('{"blend_id":"b2000000-0000-7000-8000-000000000001"}', false);
SELECT t.buat_request('{"blend_id":"b2000000-0000-7000-8000-000000000001","request_id":"b3000000-0000-7000-8000-000000000001","ttl_expires_at":null}');
SELECT t.gagal_commit($$SELECT 1$$, '23000', 'DQ-60 b: one child of two components is refused at commit');
ROLLBACK;
BEGIN;
SELECT t.buat_blend('{"blend_id":"b2000000-0000-7000-8000-000000000002"}', false);
SELECT t.gagal($$SELECT t.buat_request('{"blend_id":"b2000000-0000-7000-8000-000000000002","ttl_expires_at":"2026-11-05T00:00:00Z"}')$$, '23514',
  'BL-03 d: child ttl_expires_at must equal blend.batas_waktu_at');
SELECT t.gagal($$SELECT t.buat_request(jsonb_build_object('blend_id','b2000000-0000-7000-8000-000000000002','client_id','10000000-0000-7000-8000-000000000002',
  'credential_id','11000000-0000-7000-8000-000000000003','ttl_expires_at',(SELECT batas_waktu_at FROM blend WHERE blend_id='b2000000-0000-7000-8000-000000000002')))$$,
  '23514', 'BL-03 a: client of the child must equal the parent');
SELECT t.gagal($$SELECT t.buat_request(jsonb_build_object('blend_id','b2000000-0000-7000-8000-000000000002','received_at',now() + interval '10 seconds',
  'ttl_expires_at',(SELECT batas_waktu_at FROM blend WHERE blend_id='b2000000-0000-7000-8000-000000000002')))$$, '23514', 'BL-03 e: received_at within 5 s');
SELECT t.gagal($$SELECT t.buat_request(jsonb_build_object('blend_id','b2000000-0000-7000-8000-000000000002','idempotency_key','k','idempotency_fingerprint',repeat('a',64),
  'ttl_expires_at',(SELECT batas_waktu_at FROM blend WHERE blend_id='b2000000-0000-7000-8000-000000000002')))$$, '23514', 'BL-03 c: child idempotency is NULL');
SELECT t.buat_request(jsonb_build_object('blend_id','b2000000-0000-7000-8000-000000000002','downstream_endpoint_id','50000000-0000-7000-8000-000000000001',
  'ttl_expires_at',(SELECT batas_waktu_at FROM blend WHERE blend_id='b2000000-0000-7000-8000-000000000002')));
SELECT t.gagal($$SELECT t.buat_request(jsonb_build_object('blend_id','b2000000-0000-7000-8000-000000000002','downstream_endpoint_id','50000000-0000-7000-8000-000000000001',
  'ttl_expires_at',(SELECT batas_waktu_at FROM blend WHERE blend_id='b2000000-0000-7000-8000-000000000002')))$$, '23505', 'DQ-60 g: two children for one component');
ROLLBACK;

-- ================================================================ terminal states are frozen (T-05) and transitions (DQ-62)
BEGIN;
SELECT t.buat_blend('{"blend_id":"b4000000-0000-7000-8000-000000000001","status":"rejected","reject_reason":"kuota_habis","reject_http_status":429,
  "completed_at":"2026-11-05T00:00:00Z","webhook_status":"tidak_dikirim"}', false);
SELECT t.gagal($$UPDATE blend SET status = 'berjalan', reject_reason = NULL, reject_http_status = NULL, completed_at = NULL, webhook_status = NULL
  WHERE blend_id = 'b4000000-0000-7000-8000-000000000001'$$, '23514', 'T-05: rejected never returns to berjalan');
SELECT t.gagal($$UPDATE blend SET completed_at = now() WHERE blend_id = 'b4000000-0000-7000-8000-000000000001'$$, '23514', 'T-05: completed_at of a terminal blend is frozen');
SELECT t.gagal($$UPDATE blend SET client_id = '10000000-0000-7000-8000-000000000002' WHERE blend_id = 'b4000000-0000-7000-8000-000000000001'$$, '23514',
  'identity columns of a blend never change');
SELECT t.gagal($$INSERT INTO blend_status_history (blend_id, seq, status_dari, status_ke, aktor) VALUES ('b4000000-0000-7000-8000-000000000001', 2, 'rejected', 'done', 'controller')$$,
  '23514', 'history: no transition out of rejected');
SELECT t.gagal($$UPDATE blend_status_history SET aktor = 'client'$$, '23000', 'history is append-only (UPDATE)');
SELECT t.gagal($$DELETE FROM blend_status_history$$, '23000', 'history is append-only (DELETE outside the purge)');
ROLLBACK;
BEGIN;
SELECT t.gagal($$SELECT t.buat_blend('{"status":"done","completed_at":"2026-11-05T00:00:00Z","webhook_status":"menunggu"}', false)$$, '23514',
  'a blend is created berjalan or rejected only / done needs summary');
ROLLBACK;

-- ================================================================ gate trail (GB-09, DQ-63 h, i)
BEGIN;
DO $$
DECLARE r text;
BEGIN
  r := fn_gerbang_blend_catat(true, 'true', 'DOC-UJI-1', 'inst-1');
  IF r IS DISTINCT FROM 'gerbang_blend_dibuka' THEN RAISE EXCEPTION 'FAIL: first open expected dibuka, got %', r; END IF;
  r := fn_gerbang_blend_catat(true, 'true', 'DOC-UJI-1', 'inst-2');
  IF r IS NOT NULL THEN RAISE EXCEPTION 'FAIL: same state from a second instance must write nothing, got %', r; END IF;
  r := fn_gerbang_blend_catat(true, 'true', 'DOC-UJI-2', 'inst-1');
  IF r IS DISTINCT FROM 'gerbang_blend_dibuka' THEN RAISE EXCEPTION 'FAIL: new reference is a change, got %', r; END IF;
  r := fn_gerbang_blend_catat(false, 'false', NULL, 'inst-1');
  IF r IS DISTINCT FROM 'gerbang_blend_ditutup' THEN RAISE EXCEPTION 'FAIL: close expected ditutup, got %', r; END IF;
  r := fn_gerbang_blend_catat(false, 'TRUE', NULL, 'inst-1');
  IF r IS NOT NULL THEN RAISE EXCEPTION 'FAIL: still closed must write nothing, got %', r; END IF;
  IF (SELECT count(*) FROM audit_gerbang_blend) <> 3 THEN RAISE EXCEPTION 'FAIL: expected 3 gate events'; END IF;
  IF (SELECT sebab_tertutup FROM audit_gerbang_blend WHERE peristiwa = 'gerbang_blend_ditutup') <> 'flag_bukan_true' THEN RAISE EXCEPTION 'FAIL: sebab_tertutup'; END IF;
  RAISE NOTICE 'ok - GB-09 compare-and-set writes one event per change';
END $$;
SELECT t.gagal($$SELECT fn_gerbang_blend_catat(true, 'false', 'DOC', 'i')$$, '23514', 'GB-01: open needs the flag to be exactly true');
SELECT t.gagal($$SELECT fn_gerbang_blend_catat(true, 'true', '   ', 'i')$$, '23514', 'GB-01: open needs a non-empty reference');
SELECT t.gagal($$UPDATE audit_gerbang_blend SET instance = 'x'$$, '23000', 'DQ-63 i: gate trail is append-only (UPDATE)');
SELECT t.gagal($$DELETE FROM audit_gerbang_blend$$, '23000', 'DQ-63 i: gate trail is append-only (DELETE)');
ROLLBACK;

-- ================================================================ retention (BP-07, DQ-64)
BEGIN;
SELECT t.buat_blend('{"blend_id":"b5000000-0000-7000-8000-000000000001","status":"rejected","reject_reason":"kuota_habis","reject_http_status":429,
  "completed_at":"2026-01-01T00:00:00Z","webhook_status":"tidak_dikirim"}', false);
SELECT t.buat_blend('{"blend_id":"b5000000-0000-7000-8000-000000000002","status":"rejected","reject_reason":"kuota_habis","reject_http_status":429,
  "completed_at":"2026-11-04T00:00:00Z","webhook_status":"tidak_dikirim"}', false);
DO $$
DECLARE n bigint;
BEGIN
  n := fn_blend_hapus_online('2026-06-01T00:00:00Z');
  IF n <> 1 THEN RAISE EXCEPTION 'FAIL: expected 1 blend deleted, got %', n; END IF;
  IF EXISTS (SELECT 1 FROM blend WHERE blend_id = 'b5000000-0000-7000-8000-000000000001') THEN RAISE EXCEPTION 'FAIL: old blend still there'; END IF;
  IF NOT EXISTS (SELECT 1 FROM blend WHERE blend_id = 'b5000000-0000-7000-8000-000000000002') THEN RAISE EXCEPTION 'FAIL: recent blend deleted'; END IF;
  IF EXISTS (SELECT 1 FROM blend_status_history WHERE blend_id = 'b5000000-0000-7000-8000-000000000001') THEN RAISE EXCEPTION 'FAIL: history did not follow its blend'; END IF;
  RAISE NOTICE 'ok - BP-07 retention deletes old blends with history, keeps recent ones';
END $$;
ROLLBACK;
BEGIN;
-- a running blend and a blend whose children are still online are never deleted
SELECT t.buat_blend('{"blend_id":"b5000000-0000-7000-8000-000000000003","requested_at":"2026-01-01T00:00:00Z","batas_waktu_at":"2026-01-02T00:00:00Z"}');
DO $$ BEGIN
  IF fn_blend_hapus_online(now() + interval '1 year') <> 0 THEN RAISE EXCEPTION 'FAIL: a running blend with children was deleted'; END IF;
  RAISE NOTICE 'ok - running blend with online children is kept';
END $$;
ROLLBACK;

-- ================================================================ roles, least privilege (BA-01..BA-04, DQ-63 j)
BEGIN; SET LOCAL ROLE pr_blend;
SELECT t.jalan('SELECT blend_id, status FROM blend', 'BA-01: pr_blend reads blend');
SELECT t.jalan('SELECT request_id, blend_id, status, output_payload FROM request', 'BA-01: pr_blend reads the child columns it needs');
SELECT t.gagal('SELECT input_payload FROM request', '42501', 'BA-01: pr_blend has no input_payload');
SELECT t.gagal('SELECT nik FROM request', '42501', 'BA-01: pr_blend has no NIK');
SELECT t.gagal('SELECT nik FROM transaction_log', '42501', 'BA-01: pr_blend has no transaction_log');
SELECT t.gagal($$UPDATE request SET status = 'done'$$, '42501', 'BA-01: pr_blend cannot update request');
SELECT t.gagal($$UPDATE blend SET reject_reason = 'kuota_habis'$$, '42501', 'BA-01: pr_blend cannot write reject columns');
SELECT t.gagal($$UPDATE blend SET webhook_attempt_count = 1$$, '42501', 'BA-01: pr_blend cannot book webhook attempts');
SELECT t.gagal('DELETE FROM blend', '42501', 'BA-01: pr_blend cannot delete blend');
SELECT t.gagal('SELECT 1 FROM audit_gerbang_blend', '42501', 'pr_blend: no gate trail');
SELECT t.gagal('SELECT fn_blend_hapus_online(now())', '42501', 'pr_blend: no retention function');
ROLLBACK;
BEGIN; SET LOCAL ROLE pr_api;
SELECT t.jalan('SELECT blend_id FROM request', 'BA-02: pr_api reads request.blend_id');
SELECT t.gagal('SELECT 1 FROM blend_webhook_log', '42501', 'pr_api: no blend_webhook_log');
SELECT t.gagal($$UPDATE blend SET reject_reason = 'kuota_habis'$$, '42501', 'pr_api cannot rewrite reject_reason');
SELECT t.gagal($$UPDATE blend SET gerbang_dasar_hukum = 'diizinkan'$$, '42501', 'pr_api cannot rewrite the gate evidence');
SELECT t.gagal('DELETE FROM blend', '42501', 'pr_api cannot delete blend');
SELECT t.gagal($$INSERT INTO endpoint_blend (endpoint_blend_id, nama) VALUES (gen_random_uuid(), 'x_api')$$, '42501', 'pr_api cannot write blend configuration');
SELECT t.gagal('SELECT fn_blend_hapus_online(now())', '42501', 'pr_api: no retention function');
ROLLBACK;
BEGIN; SET LOCAL ROLE pr_controller;
SELECT t.jalan('SELECT blend_id, status, payload_purged_at FROM blend', 'BA-02: pr_controller reads blend without the payload');
SELECT t.gagal('SELECT output_payload FROM blend', '42501', 'BA-02: pr_controller never reads output_payload');
SELECT t.gagal($$UPDATE blend SET status = 'done'$$, '42501', 'pr_controller cannot change blend status');
SELECT t.jalan($$UPDATE blend SET output_payload = NULL, payload_purged_at = now() WHERE false$$, 'BP-02: pr_controller purges output_payload');
SELECT t.jalan('SELECT fn_blend_hapus_online(''2000-01-01'')', 'pr_controller runs the retention function');
SELECT t.gagal('DELETE FROM blend', '42501', 'pr_controller has no direct DELETE on blend');
ROLLBACK;
BEGIN; SET LOCAL ROLE pr_webhook;
SELECT t.jalan('SELECT blend_id, status FROM blend', 'BA-02: pr_webhook reads blend');
SELECT t.jalan($$UPDATE blend SET webhook_attempt_count = webhook_attempt_count WHERE false$$, 'BA-02: pr_webhook books attempts');
SELECT t.gagal($$UPDATE blend SET status = 'done'$$, '42501', 'pr_webhook cannot change blend status');
SELECT t.gagal('SELECT 1 FROM audit_gerbang_blend', '42501', 'pr_webhook: no gate trail');
ROLLBACK;
BEGIN; SET LOCAL ROLE pr_admin_api;
SELECT t.jalan('SELECT 1 FROM endpoint_blend', 'BA-02: pr_admin_api reads configuration');
SELECT t.gagal('SELECT 1 FROM blend', '42501', 'BA-02: pr_admin_api sees no blend rows');
SELECT t.gagal('DELETE FROM endpoint_blend', '42501', 'pr_admin_api cannot delete configuration');
SELECT t.gagal($$UPDATE endpoint_blend_versi SET batas_waktu_detik = 1$$, '42501', 'BC-01: pr_admin_api cannot edit a snapshot');
SELECT t.jalan($$UPDATE provider SET butuh_dasar_hukum_blend = butuh_dasar_hukum_blend WHERE false$$, 'pr_admin_api maintains the provider marker');
ROLLBACK;
BEGIN; SET LOCAL ROLE pr_worker;
SELECT t.gagal('SELECT 1 FROM blend', '42501', 'pr_worker: no access to blend');
SELECT t.gagal('SELECT 1 FROM endpoint_blend', '42501', 'pr_worker: no access to blend configuration');
ROLLBACK;
