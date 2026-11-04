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
