-- F3 behaviour test (SHAA-354): constraints and triggers of the F3 schema per contract rule (BC, BL, GB, BP, DQ-60..65).
-- Synthetic data only. Every block runs in a transaction that is rolled back. t.gagal_commit / t.jalan_commit come from 55_f1_constraints.sql.
\set ON_ERROR_STOP on

-- A complete async blend: second endpoint, a two-component configuration (version 1), the blend (berjalan) with its history row,
-- and two queued children. Ids: blend b3..01, children f3..a1 / f3..a2, endpoint_blend e3..01, endpoint 2 = 50..02.
CREATE OR REPLACE FUNCTION t.f3_siapkan(p_penanda boolean DEFAULT false) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
  INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, downstream_pool_id) VALUES
    ('50000000-0000-7000-8000-000000000002', 'ep-2', '20000000-0000-7000-8000-000000000002', '40000000-0000-7000-8000-000000000001');
  UPDATE provider SET butuh_dasar_hukum_blend = p_penanda WHERE provider_id = '20000000-0000-7000-8000-000000000001';
  INSERT INTO endpoint_blend (endpoint_blend_id, nama) VALUES ('e3000000-0000-7000-8000-000000000001', 'blend_uji');
  INSERT INTO endpoint_blend_versi (endpoint_blend_id, config_version, config_schema_version, mode_diizinkan, batas_waktu_detik, dibuat_oleh)
    VALUES ('e3000000-0000-7000-8000-000000000001', 1, 1, 'keduanya', 3600, '00000000-0000-7000-8000-000000000001');
  INSERT INTO endpoint_blend_komponen VALUES
    ('e3000000-0000-7000-8000-000000000001', 1, 'dukcapil', '50000000-0000-7000-8000-000000000001', 1, '{"nik": "nik"}'),
    ('e3000000-0000-7000-8000-000000000001', 1, 'lain', '50000000-0000-7000-8000-000000000002', 2, '{"nik": "nik"}');
  INSERT INTO blend (blend_id, client_id, credential_id, endpoint_blend_id, endpoint_blend_config_version, mode, status, gerbang_dasar_hukum,
                     ttl_polling_detik, batas_waktu_at, requested_at)
    VALUES ('b3000000-0000-7000-8000-000000000001', '10000000-0000-7000-8000-000000000001', '11000000-0000-7000-8000-000000000001',
            'e3000000-0000-7000-8000-000000000001', 1, 'async', 'berjalan', CASE WHEN p_penanda THEN 'diizinkan' ELSE 'tidak_berlaku' END,
            86400, now() + interval '1 day', now());
  INSERT INTO blend_status_history (blend_id, seq, status_dari, status_ke, aktor) VALUES ('b3000000-0000-7000-8000-000000000001', 1, NULL, 'berjalan', 'webserver');
  PERFORM t.buat_request(jsonb_build_object('request_id', 'f3000000-0000-7000-8000-0000000000a1', 'blend_id', 'b3000000-0000-7000-8000-000000000001',
    'ttl_expires_at', (SELECT batas_waktu_at FROM blend WHERE blend_id = 'b3000000-0000-7000-8000-000000000001')));
  PERFORM t.buat_request(jsonb_build_object('request_id', 'f3000000-0000-7000-8000-0000000000a2', 'blend_id', 'b3000000-0000-7000-8000-000000000001',
    'downstream_endpoint_id', '50000000-0000-7000-8000-000000000002', 'provider_id', '20000000-0000-7000-8000-000000000002',
    'ttl_expires_at', (SELECT batas_waktu_at FROM blend WHERE blend_id = 'b3000000-0000-7000-8000-000000000001')));
END $$;

-- ================================================================ happy path and BL-02 (atomic creation)
BEGIN;
SELECT t.jalan_commit($$SELECT t.f3_siapkan()$$, 'BL-02: blend, two children and the history rows are accepted in one transaction');
ROLLBACK;

-- ================================================================ BL-01 blend_id written once
BEGIN;
SELECT t.f3_siapkan();
SELECT t.gagal($$UPDATE request SET blend_id = NULL WHERE request_id = 'f3000000-0000-7000-8000-0000000000a1'$$, '23514', 'BL-01: blend_id cannot be cleared');
SELECT t.buat_request('{"request_id":"f3000000-0000-7000-8000-0000000000f0"}');
SELECT t.gagal($$UPDATE request SET blend_id = 'b3000000-0000-7000-8000-000000000001' WHERE request_id = 'f3000000-0000-7000-8000-0000000000f0'$$, '23514',
  'BL-01: an F0 request never becomes a child (no NULL -> set)');
ROLLBACK;

-- ================================================================ BL-03 structure of the children (DQ-60)
BEGIN;
SELECT t.f3_siapkan();
SELECT t.gagal($$SELECT t.buat_request('{"blend_id":"b3000000-0000-7000-8000-000000000001","client_id":"10000000-0000-7000-8000-000000000002","credential_id":"11000000-0000-7000-8000-000000000003",
  "ttl_expires_at":"2099-01-01T00:00:00Z"}')$$, '23514', 'BL-03 a / DQ-60 c: a child of another client is refused');
SELECT t.gagal($$SELECT t.buat_request(jsonb_build_object('blend_id','b3000000-0000-7000-8000-000000000001','idempotency_key','k1','idempotency_fingerprint',repeat('a',64),
  'ttl_expires_at',(SELECT batas_waktu_at FROM blend)))$$, '23514', 'BL-03 c / DQ-60 d: a child carries no idempotency key');
SELECT t.gagal($$SELECT t.buat_request(jsonb_build_object('blend_id','b3000000-0000-7000-8000-000000000001','ttl_expires_at',now() + interval '2 days'))$$, '23514',
  'BL-03 d / DQ-60 e: child ttl_expires_at equals batas_waktu_at');
SELECT t.gagal($$SELECT t.buat_request(jsonb_build_object('blend_id','b3000000-0000-7000-8000-000000000001','received_at',now() - interval '10 seconds',
  'ttl_expires_at',(SELECT batas_waktu_at FROM blend)))$$, '23514', 'BL-03 e: received_at within 5 s of the parent');
SELECT t.gagal($$SELECT t.buat_request(jsonb_build_object('blend_id','b3000000-0000-7000-8000-000000000001','request_id','f3000000-0000-7000-8000-0000000000a9',
  'ttl_expires_at',(SELECT batas_waktu_at FROM blend)))$$, '23505', 'DQ-60 g: two children of one blend on the same endpoint');
SELECT t.gagal($$SELECT t.buat_request(jsonb_build_object('blend_id','b3000000-0000-7000-8000-000000000099','ttl_expires_at',now() + interval '1 day'))$$, '23000',
  'BL-03 a / DQ-60 a: a child without a parent');
INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, downstream_pool_id) VALUES
  ('50000000-0000-7000-8000-000000000003', 'ep-3', '20000000-0000-7000-8000-000000000002', '40000000-0000-7000-8000-000000000001');
SELECT t.gagal($$SELECT t.buat_request(jsonb_build_object('blend_id','b3000000-0000-7000-8000-000000000001','downstream_endpoint_id','50000000-0000-7000-8000-000000000003',
  'ttl_expires_at',(SELECT batas_waktu_at FROM blend)))$$, '23514', 'BL-03 b: the child endpoint must be a component of the parent version');
SELECT t.gagal($$UPDATE request SET webhook_status = 'menunggu' WHERE request_id = 'f3000000-0000-7000-8000-0000000000a1'$$, '23514',
  'BL-03 f: a child never enters the webhook outbox');
SELECT t.gagal($$INSERT INTO webhook_log VALUES (gen_random_uuid(), 'f3000000-0000-7000-8000-0000000000a1', '10000000-0000-7000-8000-000000000001', 1, 'done', now(), NULL, 200, 'terkirim', NULL, 10, 1)$$,
  '23514', 'DQ-65 c: no webhook_log row for a child');
ROLLBACK;

-- DQ-60 (b): the number of children equals the number of components (checked at commit); a rejected blend has no children (g)
BEGIN;
INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, downstream_pool_id) VALUES
  ('50000000-0000-7000-8000-000000000002', 'ep-2', '20000000-0000-7000-8000-000000000002', '40000000-0000-7000-8000-000000000001');
INSERT INTO endpoint_blend (endpoint_blend_id, nama) VALUES ('e3000000-0000-7000-8000-000000000001', 'blend_uji');
INSERT INTO endpoint_blend_versi (endpoint_blend_id, config_version, config_schema_version, mode_diizinkan, dibuat_oleh)
  VALUES ('e3000000-0000-7000-8000-000000000001', 1, 1, 'sync', '00000000-0000-7000-8000-000000000001');
INSERT INTO endpoint_blend_komponen VALUES
  ('e3000000-0000-7000-8000-000000000001', 1, 'a', '50000000-0000-7000-8000-000000000001', 1, '{}'),
  ('e3000000-0000-7000-8000-000000000001', 1, 'b', '50000000-0000-7000-8000-000000000002', 2, '{}');
INSERT INTO blend (blend_id, client_id, credential_id, endpoint_blend_id, endpoint_blend_config_version, mode, status, gerbang_dasar_hukum, batas_waktu_at, requested_at)
  VALUES ('b3000000-0000-7000-8000-000000000002', '10000000-0000-7000-8000-000000000001', '11000000-0000-7000-8000-000000000001',
          'e3000000-0000-7000-8000-000000000001', 1, 'sync', 'berjalan', 'tidak_berlaku', now() + interval '25 seconds', now());
INSERT INTO blend_status_history (blend_id, seq, status_dari, status_ke, aktor) VALUES ('b3000000-0000-7000-8000-000000000002', 1, NULL, 'berjalan', 'webserver');
SELECT t.buat_request('{"blend_id":"b3000000-0000-7000-8000-000000000002","mode":"sync","status":"in_flight","attempt_count":1,"ttl_expires_at":null,"dispatch_key":null}');
SELECT t.gagal_commit($$SELECT 1$$, '23000', 'DQ-60 b: one child for two components is refused at commit');
ROLLBACK;

BEGIN;
SELECT t.f3_siapkan();
INSERT INTO blend (blend_id, client_id, credential_id, endpoint_blend_id, endpoint_blend_config_version, mode, status, reject_reason, reject_http_status, gerbang_dasar_hukum, batas_waktu_at, requested_at)
  VALUES ('b3000000-0000-7000-8000-000000000003', '10000000-0000-7000-8000-000000000001', '11000000-0000-7000-8000-000000000001',
          'e3000000-0000-7000-8000-000000000001', 1, 'sync', 'rejected', 'kuota_habis', 429, 'tidak_berlaku', now(), now());
INSERT INTO blend_status_history (blend_id, seq, status_dari, status_ke, alasan, aktor) VALUES ('b3000000-0000-7000-8000-000000000003', 1, NULL, 'rejected', 'kuota_habis', 'webserver');
SELECT t.gagal($$SELECT t.buat_request(jsonb_build_object('blend_id','b3000000-0000-7000-8000-000000000003','ttl_expires_at',now() + interval '1 day'))$$, '23514',
  'BL-03 g / DQ-60 f: a rejected blend has no children');
ROLLBACK;

-- ================================================================ section 4.3 enums, DQ-62 domain
BEGIN;
SELECT t.f3_siapkan();
SELECT t.gagal($$UPDATE blend SET status = 'dispatched'$$, '23514', 'blend.status takes only the six values');
SELECT t.gagal($$UPDATE blend SET failure_reason = 'komponen_gagal'$$, '23514', 'failure_reason: no komponen_gagal (ADR rev 1 value is gone)');
SELECT t.gagal($$UPDATE blend SET failure_reason = 'gerbang_blend_tertutup'$$, '23514', 'gerbang_blend_tertutup is not a failure_reason');
SELECT t.gagal($$SELECT t.buat_request('{"status":"rejected","reject_reason":"gerbang_blend_tertutup","reject_http_status":403,"attempt_count":0,"completed_at":"2026-11-04T00:00:00Z","ttl_expires_at":"2026-11-05T00:00:00Z"}')$$,
  '23514', 'F0: request.reject_reason does not accept gerbang_blend_tertutup');
-- gate proof (GB-05, DQ-63 c)
SELECT t.jalan($$INSERT INTO blend (blend_id, client_id, credential_id, endpoint_blend_id, endpoint_blend_config_version, mode, status, reject_reason, reject_http_status, gerbang_dasar_hukum, batas_waktu_at, requested_at, completed_at, webhook_status)
  VALUES ('b3000000-0000-7000-8000-000000000010', '10000000-0000-7000-8000-000000000001', '11000000-0000-7000-8000-000000000001', 'e3000000-0000-7000-8000-000000000001', 1, 'async', 'rejected',
          'gerbang_blend_tertutup', 403, 'ditolak', now(), now(), now(), 'tidak_dikirim')$$, 'GB-05: rejected / gerbang_blend_tertutup / 403 / ditolak is accepted');
SELECT t.gagal($$INSERT INTO blend (blend_id, client_id, credential_id, endpoint_blend_id, endpoint_blend_config_version, mode, status, reject_reason, reject_http_status, gerbang_dasar_hukum, batas_waktu_at, requested_at, completed_at, webhook_status)
  VALUES (gen_random_uuid(), '10000000-0000-7000-8000-000000000001', '11000000-0000-7000-8000-000000000001', 'e3000000-0000-7000-8000-000000000001', 1, 'async', 'rejected',
          'gerbang_blend_tertutup', 403, 'diizinkan', now(), now(), now(), 'tidak_dikirim')$$, '23514', 'DQ-63 c: gerbang_blend_tertutup requires proof ditolak');
SELECT t.gagal($$INSERT INTO blend (blend_id, client_id, credential_id, endpoint_blend_id, endpoint_blend_config_version, mode, status, reject_reason, reject_http_status, gerbang_dasar_hukum, batas_waktu_at, requested_at, completed_at, webhook_status)
  VALUES (gen_random_uuid(), '10000000-0000-7000-8000-000000000001', '11000000-0000-7000-8000-000000000001', 'e3000000-0000-7000-8000-000000000001', 1, 'async', 'rejected',
          'kuota_habis', 429, 'ditolak', now(), now(), now(), 'tidak_dikirim')$$, '23514', 'DQ-63 c: proof ditolak only with gerbang_blend_tertutup');
SELECT t.gagal($$INSERT INTO blend (blend_id, client_id, credential_id, endpoint_blend_id, endpoint_blend_config_version, mode, status, reject_reason, reject_http_status, gerbang_dasar_hukum, batas_waktu_at, requested_at, completed_at, webhook_status)
  VALUES (gen_random_uuid(), '10000000-0000-7000-8000-000000000001', '11000000-0000-7000-8000-000000000001', 'e3000000-0000-7000-8000-000000000001', 1, 'async', 'rejected',
          'gerbang_blend_tertutup', 503, 'ditolak', now(), now(), now(), 'tidak_dikirim')$$, '23514', 'DQ-62: gerbang_blend_tertutup is 403 only');
SELECT t.gagal($$UPDATE blend SET gerbang_dasar_hukum = 'mungkin'$$, '23514', 'DQ-62: gerbang_dasar_hukum domain');
SELECT t.gagal($$INSERT INTO blend (blend_id, client_id, credential_id, endpoint_blend_id, endpoint_blend_config_version, mode, status, reject_reason, reject_http_status, gerbang_dasar_hukum, batas_waktu_at, requested_at, completed_at, webhook_status, ttl_polling_detik)
  VALUES (gen_random_uuid(), '10000000-0000-7000-8000-000000000001', '11000000-0000-7000-8000-000000000001', 'e3000000-0000-7000-8000-000000000001', 1, 'async', 'rejected',
          'kuota_habis', 429, 'tidak_berlaku', now(), now(), now(), 'tidak_dikirim', 100)$$, '23514', 'a rejected blend keeps no polling window');
ROLLBACK;

-- ================================================================ DQ-62 (f) summary shape, BP-04 payload, webhook, idempotency, T-05
BEGIN;
SELECT t.f3_siapkan();
CREATE TEMP TABLE b_uji AS SELECT * FROM blend WHERE blend_id = 'b3000000-0000-7000-8000-000000000001';
SELECT t.jalan($$INSERT INTO blend SELECT (b).* FROM (SELECT jsonb_populate_record(NULL::blend, to_jsonb(u) || jsonb_build_object('blend_id', gen_random_uuid(), 'status', 'failed',
  'failure_reason', 'retry_habis', 'anak_penentu_request_id', 'f3000000-0000-7000-8000-0000000000a1', 'completed_at', now(), 'webhook_status', 'menunggu',
  'ringkasan_komponen', '[{"nama_komponen":"dukcapil","request_id":"f3000000-0000-7000-8000-0000000000a1","status_api":"failed","failure_reason":"retry_habis"}]'::jsonb)) AS b FROM b_uji u) x$$,
  'DQ-62 f: a summary with the four allowed keys is accepted');
SELECT t.gagal($$INSERT INTO blend SELECT (b).* FROM (SELECT jsonb_populate_record(NULL::blend, to_jsonb(u) || jsonb_build_object('blend_id', gen_random_uuid(), 'status', 'failed',
  'failure_reason', 'retry_habis', 'anak_penentu_request_id', 'f3000000-0000-7000-8000-0000000000a1', 'completed_at', now(), 'webhook_status', 'menunggu',
  'ringkasan_komponen', '[{"nama_komponen":"dukcapil","nik":"0000000000000007"}]'::jsonb)) AS b FROM b_uji u) x$$, '23514', 'DQ-62 f: a summary key other than the four is refused');
SELECT t.gagal($$INSERT INTO blend SELECT (b).* FROM (SELECT jsonb_populate_record(NULL::blend, to_jsonb(u) || jsonb_build_object('blend_id', gen_random_uuid(), 'status', 'failed',
  'failure_reason', 'retry_habis', 'anak_penentu_request_id', 'f3000000-0000-7000-8000-0000000000a1', 'completed_at', now(), 'webhook_status', 'menunggu',
  'ringkasan_komponen', '[{"nama_komponen":"0000000000000007"}]'::jsonb)) AS b FROM b_uji u) x$$, '23514', 'DQ-62 f: a 16-digit value in the summary is refused');
SELECT t.gagal($$INSERT INTO blend SELECT (b).* FROM (SELECT jsonb_populate_record(NULL::blend, to_jsonb(u) || jsonb_build_object('blend_id', gen_random_uuid(), 'status', 'failed',
  'failure_reason', 'retry_habis', 'anak_penentu_request_id', 'f3000000-0000-7000-8000-0000000000a1', 'completed_at', now(), 'webhook_status', 'menunggu',
  'ringkasan_komponen', NULL)) AS b FROM b_uji u) x$$, '23514', 'a terminal blend other than rejected has a summary');
-- failed needs the deciding child (BL-05); output only for done (BP-04); webhook shape
SELECT t.gagal($$UPDATE blend SET status = 'failed', failure_reason = 'retry_habis', completed_at = now(), webhook_status = 'menunggu', ringkasan_komponen = '[]' $$, '23514',
  'BL-05: failed without anak_penentu_request_id is refused');
SELECT t.gagal($$UPDATE blend SET output_payload = '{"a":1}'::jsonb$$, '23514', 'BP-04: a blend that is not done keeps no output_payload');
SELECT t.gagal($$UPDATE blend SET webhook_status = 'menunggu'$$, '23514', 'a running blend has no webhook_status yet');
SELECT t.gagal($$UPDATE blend SET idempotency_key = 'k', idempotency_fingerprint = repeat('b', 64)$$, '23514', 'identity columns (idempotency) never change (trigger)');
SELECT t.gagal($$UPDATE blend SET client_id = '10000000-0000-7000-8000-000000000002'$$, '23514', 'blend identity columns are immutable');
SELECT t.gagal($$UPDATE blend SET status = 'rejected', reject_reason = 'kuota_habis', reject_http_status = 429, completed_at = now(), webhook_status = 'tidak_dikirim'$$, '23514',
  'T-05: berjalan -> rejected is not a transition');
ROLLBACK;

-- T-04 history and T-05 terminal immutability, BL-05 consistency at the close
BEGIN;
SELECT t.f3_siapkan();
UPDATE request SET status = 'expired', completed_at = now() WHERE blend_id = 'b3000000-0000-7000-8000-000000000001';
INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor)
  SELECT request_id, 2, 'queued', 'expired', NULL, 'controller' FROM request WHERE blend_id = 'b3000000-0000-7000-8000-000000000001';
SELECT t.gagal_commit($$UPDATE blend SET status = 'expired', completed_at = now(), ringkasan_komponen = '[]', webhook_status = 'menunggu'$$, '23000',
  'T-04: a blend status without its history row is refused at commit');
ROLLBACK;

BEGIN;
SELECT t.f3_siapkan();
UPDATE request SET status = 'expired', completed_at = now() WHERE blend_id = 'b3000000-0000-7000-8000-000000000001';
INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor)
  SELECT request_id, 2, 'queued', 'expired', NULL, 'controller' FROM request WHERE blend_id = 'b3000000-0000-7000-8000-000000000001';
SELECT t.jalan_commit($$UPDATE blend SET status = 'expired', completed_at = now(), ringkasan_komponen = '[]', webhook_status = 'menunggu' WHERE blend_id = 'b3000000-0000-7000-8000-000000000001';
  INSERT INTO blend_status_history (blend_id, seq, status_dari, status_ke, aktor) VALUES ('b3000000-0000-7000-8000-000000000001', 2, 'berjalan', 'expired', 'controller')$$,
  'BL-05.4: all children expired without attempts -> blend expired (with its history row)');
SELECT t.gagal($$UPDATE blend SET failure_reason = 'retry_habis'$$, '23514', 'T-05: a terminal blend keeps its outcome');
SELECT t.jalan($$UPDATE blend SET webhook_status = 'terkirim', webhook_attempt_count = 1 WHERE blend_id = 'b3000000-0000-7000-8000-000000000001'$$, 'webhook bookkeeping may still change on a terminal blend');
SELECT t.gagal($$UPDATE blend_status_history SET alasan = 'x'$$, '23000', 'blend_status_history is append-only');
SELECT t.gagal($$DELETE FROM blend_status_history$$, '23000', 'blend_status_history cannot be deleted');
ROLLBACK;

-- DQ-61 (c)(d): cancelled needs zero attempts; expired needs no attempts and no failed / cancelled / done child; done needs all done
BEGIN;
SELECT t.f3_siapkan();
UPDATE request SET status = 'in_flight', attempt_count = 1, dispatched_at = now(), first_attempt_at = now() WHERE request_id = 'f3000000-0000-7000-8000-0000000000a1';
UPDATE request SET status = 'cancelled', cancel_reason = 'dibatalkan_client', cancelled_by = 'client', completed_at = now() WHERE request_id = 'f3000000-0000-7000-8000-0000000000a2';
SELECT t.gagal_commit($$UPDATE blend SET status = 'cancelled', cancel_reason = 'dibatalkan_client', cancelled_by = 'client', completed_at = now(), ringkasan_komponen = '[]', webhook_status = 'menunggu'$$,
  '23000', 'DQ-61 c / BL-06: cancelled blend with a child that already has an attempt is refused');
ROLLBACK;
BEGIN;
SELECT t.f3_siapkan();
SELECT t.gagal_commit($$UPDATE blend SET status = 'done', completed_at = now(), output_payload = '{"x":1}', ringkasan_komponen = '[]', webhook_status = 'menunggu'$$,
  '23000', 'BL-05.1: done blend with children that are not done is refused');
ROLLBACK;
BEGIN;
SELECT t.f3_siapkan();
UPDATE request SET status = 'in_flight', attempt_count = 1, dispatched_at = now(), first_attempt_at = now() WHERE request_id = 'f3000000-0000-7000-8000-0000000000a1';
SELECT t.gagal_commit($$UPDATE blend SET status = 'expired', completed_at = now(), ringkasan_komponen = '[]', webhook_status = 'menunggu'$$,
  '23000', 'DQ-61 d: expired blend with attempts is refused');
ROLLBACK;
BEGIN;
SELECT t.f3_siapkan();
SELECT t.gagal_commit($$UPDATE blend SET status = 'failed', failure_reason = 'retry_habis', anak_penentu_request_id = 'f3000000-0000-7000-8000-0000000000f0', completed_at = now(),
  ringkasan_komponen = '[]', webhook_status = 'menunggu'$$, '23503', 'BL-05: the deciding child must exist (FK)');
ROLLBACK;

-- ================================================================ BC-01 / BC-02 configuration
BEGIN;
SELECT t.f3_siapkan();
SELECT t.gagal($$UPDATE endpoint_blend_versi SET tata_letak = 'gabung_field'$$, '23000', 'BC-01: a configuration version never changes');
SELECT t.gagal($$DELETE FROM endpoint_blend_komponen$$, '23000', 'BC-01: components of a version are never deleted');
SELECT t.gagal($$INSERT INTO endpoint_blend_komponen VALUES ('e3000000-0000-7000-8000-000000000001', 1, 'tiga', '50000000-0000-7000-8000-000000000001', 3, '{}')$$, '23000',
  'BC-01: no component is added to a version a blend refers to');
SELECT t.gagal($$INSERT INTO endpoint_blend (endpoint_blend_id, nama, status) VALUES (gen_random_uuid(), 'baru_aktif', 'aktif')$$, '23514', 'BC-02: a new blend is always nonaktif');
SELECT t.gagal($$INSERT INTO endpoint_blend (endpoint_blend_id, nama) VALUES (gen_random_uuid(), 'Nama Salah')$$, '23514', 'nama is snake_case ASCII');
SELECT t.gagal($$INSERT INTO endpoint_blend_versi (endpoint_blend_id, config_version, config_schema_version, mode_diizinkan, batas_waktu_detik, dibuat_oleh)
  VALUES ('e3000000-0000-7000-8000-000000000001', 2, 1, 'sync', 60, '00000000-0000-7000-8000-000000000001')$$, '23514', 'batas_waktu_detik is refused on a sync-only blend (422 in the API)');
SELECT t.gagal($$INSERT INTO endpoint_blend_versi (endpoint_blend_id, config_version, config_schema_version, mode_diizinkan, dibuat_oleh)
  VALUES ('e3000000-0000-7000-8000-000000000001', 2, 1, 'async', '00000000-0000-7000-8000-000000000001')$$, '23514', 'batas_waktu_detik is required when async is allowed');
ROLLBACK;
BEGIN;
INSERT INTO endpoint_blend (endpoint_blend_id, nama) VALUES ('e3000000-0000-7000-8000-000000000002', 'satu_komponen');
INSERT INTO endpoint_blend_versi (endpoint_blend_id, config_version, config_schema_version, mode_diizinkan, dibuat_oleh)
  VALUES ('e3000000-0000-7000-8000-000000000002', 1, 1, 'sync', '00000000-0000-7000-8000-000000000001');
INSERT INTO endpoint_blend_komponen VALUES ('e3000000-0000-7000-8000-000000000002', 1, 'a', '50000000-0000-7000-8000-000000000001', 1, '{}');
SELECT t.gagal_commit($$SELECT 1$$, '23514', 'ADR-0007 D1 butir 1: a version with fewer than two components is refused at commit');
ROLLBACK;
BEGIN;
SELECT t.f3_siapkan();
SELECT t.gagal($$INSERT INTO endpoint_blend_komponen VALUES ('e3000000-0000-7000-8000-000000000001', 2, 'x', '50000000-0000-7000-8000-000000000001', 1, '{}')$$, '23503', 'a component needs its version');
ROLLBACK;

-- ================================================================ gate F3-4: G-4 (marker change) and G-1 (activation), GB-07
BEGIN;
SELECT t.f3_siapkan(false);
UPDATE endpoint_blend SET status = 'aktif' WHERE endpoint_blend_id = 'e3000000-0000-7000-8000-000000000001';
SELECT t.gagal($$UPDATE provider SET butuh_dasar_hukum_blend = true WHERE provider_id = '20000000-0000-7000-8000-000000000001'$$, '23514',
  'G-4 / GB-02 point 2: the marker cannot go false -> true while an active blend uses the provider');
SELECT t.jalan($$UPDATE provider SET butuh_dasar_hukum_blend = true WHERE provider_id = '20000000-0000-7000-8000-000000000002'$$,
  'G-4 does not touch a provider that no active blend uses');
UPDATE endpoint_blend SET status = 'nonaktif' WHERE endpoint_blend_id = 'e3000000-0000-7000-8000-000000000001';
SELECT t.jalan($$UPDATE provider SET butuh_dasar_hukum_blend = true WHERE provider_id = '20000000-0000-7000-8000-000000000001'$$, 'G-4: after deactivation the marker may be set');
ROLLBACK;

BEGIN;
SELECT t.f3_siapkan(true);
SELECT t.gagal($$UPDATE endpoint_blend SET status = 'aktif' WHERE endpoint_blend_id = 'e3000000-0000-7000-8000-000000000001'$$, '23514',
  'G-1 / GB-01: activation with a marked provider is refused while the gate is not stated open (fail-closed)');
SELECT set_config('poolrouter.gerbang_blend_terbuka', 'ya', true);
SELECT t.gagal($$UPDATE endpoint_blend SET status = 'aktif' WHERE endpoint_blend_id = 'e3000000-0000-7000-8000-000000000001'$$, '23514',
  'GB-01: any value other than the string true is closed');
SELECT set_config('poolrouter.gerbang_blend_terbuka', 'true', true);
SELECT t.jalan($$UPDATE endpoint_blend SET status = 'aktif' WHERE endpoint_blend_id = 'e3000000-0000-7000-8000-000000000001'$$, 'G-1: activation passes when the gate is open');
ROLLBACK;
BEGIN;
SELECT t.f3_siapkan(false);
SELECT t.jalan($$UPDATE endpoint_blend SET status = 'aktif' WHERE endpoint_blend_id = 'e3000000-0000-7000-8000-000000000001'$$, 'G-1: a blend without marked provider activates without the gate');
ROLLBACK;

-- GB-03 audit of the marker (DQ-63 e, f)
BEGIN;
SELECT t.ok(t.hitung($q$SELECT count(*) FROM provider_penanda_audit WHERE aktor = 'migrasi' AND nilai_lama IS NULL$q$) = t.hitung('SELECT count(*) FROM provider WHERE false') + t.hitung($q$SELECT count(*) FROM provider_penanda_audit WHERE aktor = 'migrasi'$q$),
  'GB-03: migration entries carry no previous value');
SELECT t.gagal($$INSERT INTO provider_penanda_audit (audit_penanda_id, provider_id, aktor, pengguna_id, nilai_lama, nilai_baru, alasan)
  VALUES (gen_random_uuid(), '20000000-0000-7000-8000-000000000001', 'admin', '00000000-0000-7000-8000-000000000001', true, false, 'x')$$, '23514',
  'DQ-63 f: true -> false without the approval reference is refused');
SELECT t.jalan($$INSERT INTO provider_penanda_audit (audit_penanda_id, provider_id, aktor, pengguna_id, nilai_lama, nilai_baru, alasan, referensi_persetujuan_nomor, referensi_persetujuan_tanggal)
  VALUES (gen_random_uuid(), '20000000-0000-7000-8000-000000000001', 'admin', '00000000-0000-7000-8000-000000000001', true, false, 'x', 'B-001/UJI', '2026-11-04')$$,
  'GB-03: true -> false with number and date is accepted');
SELECT t.gagal($$INSERT INTO provider_penanda_audit (audit_penanda_id, provider_id, aktor, pengguna_id, nilai_lama, nilai_baru, alasan, blend_terdampak)
  VALUES (gen_random_uuid(), '20000000-0000-7000-8000-000000000001', 'admin', '00000000-0000-7000-8000-000000000001', false, true, 'x', '[{"nama":"b"}]')$$, '23514',
  'DQ-63 e: a successful false -> true with affected active blends is refused');
SELECT t.jalan($$INSERT INTO provider_penanda_audit (audit_penanda_id, provider_id, aktor, pengguna_id, nilai_lama, nilai_baru, alasan, hasil, blend_terdampak)
  VALUES (gen_random_uuid(), '20000000-0000-7000-8000-000000000001', 'admin', '00000000-0000-7000-8000-000000000001', false, true, 'x', 'ditolak_gerbang', '[{"nama":"b"}]')$$,
  'GB-02 point 2: a refused change is recorded as ditolak_gerbang with the affected list');
SELECT t.gagal($$UPDATE provider_penanda_audit SET alasan = 'y'$$, '23000', 'the marker audit is append-only');
ROLLBACK;

-- ================================================================ GB-09 gate trail
BEGIN;
SELECT t.ok(fn_catat_gerbang_blend('true', '', 'i1') IS NULL, 'GB-09: no events yet counts as (closed, empty); closed again writes nothing');
SELECT t.ok(fn_catat_gerbang_blend('true', 'DOC-UJI-1', 'i1') = 'gerbang_blend_dibuka', 'GB-09: closed -> open writes one gerbang_blend_dibuka');
SELECT t.ok(fn_catat_gerbang_blend('true', 'DOC-UJI-1', 'i2') IS NULL, 'GB-09: a second instance with the same pair writes nothing (once per change, not per instance)');
SELECT t.ok(fn_catat_gerbang_blend('true', 'DOC-UJI-2', 'i1') = 'gerbang_blend_dibuka', 'GB-09: still open with a new dasar_hukum_ref writes a new dibuka');
SELECT t.ok(fn_catat_gerbang_blend('false', NULL, 'i1') = 'gerbang_blend_ditutup', 'GB-09: open -> closed writes one ditutup');
SELECT t.ok(fn_catat_gerbang_blend(NULL, 'DOC-UJI-2', 'i1') IS NULL, 'GB-09: staying closed writes nothing, whatever the raw flag');
SELECT t.ok(fn_catat_gerbang_blend('true', '  ', 'i1') IS NULL, 'GB-09: flag true without a reference stays closed and writes nothing');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM audit_gerbang_blend$q$) = 3, 'GB-09: three events for the whole sequence');
SELECT t.ok((SELECT sebab_tertutup FROM audit_gerbang_blend WHERE jenis = 'gerbang_blend_ditutup') = 'flag_bukan_true', 'GB-09: ditutup records sebab_tertutup');
SELECT t.gagal($$INSERT INTO audit_gerbang_blend (jenis, nilai_flag, sebab_tertutup, instance) VALUES ('gerbang_blend_ditutup', 'false', 'flag_bukan_true', 'i9')$$, '23505',
  'DQ-63 i: two ditutup in a row are refused for any writer');
SELECT t.gagal($$INSERT INTO audit_gerbang_blend (jenis, nilai_flag, dasar_hukum_ref, instance) VALUES ('gerbang_blend_dibuka', 'true', NULL, 'i9')$$, '23514',
  'DQ-63 i: dibuka without dasar_hukum_ref is refused');
SELECT t.gagal($$INSERT INTO audit_gerbang_blend (jenis, nilai_flag, dasar_hukum_ref, instance) VALUES ('gerbang_blend_dibuka', 'false', 'DOC-X', 'i9')$$, '23514',
  'DQ-63 i: dibuka needs nilai_flag = true');
SELECT t.gagal($$INSERT INTO audit_gerbang_blend (jenis, nilai_flag, instance) VALUES ('gerbang_blend_ditutup', 'false', 'i9')$$, '23514', 'DQ-63 i: ditutup needs sebab_tertutup');
SELECT t.gagal($$INSERT INTO audit_gerbang_blend (jenis, nilai_flag, sebab_tertutup, instance) VALUES ('gerbang_blend_ditutup', 'true', 'flag_bukan_true', 'i9')$$, '23514',
  'sebab_tertutup matches the raw flag (true without reference = rujukan_kosong)');
SELECT t.gagal($$DELETE FROM audit_gerbang_blend$$, '23000', 'the gate trail is append-only');
SELECT t.gagal($$TRUNCATE audit_gerbang_blend$$, '23000', 'the gate trail cannot be truncated');
ROLLBACK;

-- ================================================================ BP-01 / BP-02 / BP-05 payload retention
BEGIN;
SELECT t.f3_siapkan();
CREATE TEMP TABLE b_uji AS SELECT * FROM blend WHERE blend_id = 'b3000000-0000-7000-8000-000000000001';
-- an async done blend completed 2 days ago (polling window 1 day) and a sync one completed just now with a payload
INSERT INTO blend SELECT (b).* FROM (SELECT jsonb_populate_record(NULL::blend, to_jsonb(u) || jsonb_build_object('blend_id', 'b3000000-0000-7000-8000-0000000000d1', 'status', 'done',
  'requested_at', now() - interval '3 days', 'batas_waktu_at', now() - interval '2 days', 'completed_at', now() - interval '2 days', 'webhook_status', 'terkirim',
  'output_payload', '{"dukcapil":{"x":1}}'::jsonb, 'ringkasan_komponen', '[]'::jsonb)) AS b FROM b_uji u) x;
INSERT INTO blend SELECT (b).* FROM (SELECT jsonb_populate_record(NULL::blend, to_jsonb(u) || jsonb_build_object('blend_id', 'b3000000-0000-7000-8000-0000000000d2', 'status', 'done',
  'requested_at', now() - interval '1 hour', 'batas_waktu_at', now() + interval '1 hour', 'completed_at', now() - interval '1 hour', 'webhook_status', 'terkirim',
  'output_payload', '{"dukcapil":{"x":1}}'::jsonb, 'ringkasan_komponen', '[]'::jsonb)) AS b FROM b_uji u) x;
INSERT INTO blend SELECT (b).* FROM (SELECT jsonb_populate_record(NULL::blend, to_jsonb(u) || jsonb_build_object('blend_id', 'b3000000-0000-7000-8000-0000000000d3', 'status', 'done', 'mode', 'sync',
  'ttl_polling_detik', NULL, 'requested_at', now() - interval '1 minute', 'batas_waktu_at', now() + interval '1 hour', 'completed_at', now() - interval '1 minute',
  'webhook_status', 'tidak_dikirim', 'output_payload', '{"a":1}'::jsonb, 'ringkasan_komponen', '[]'::jsonb)) AS b FROM b_uji u) x;
SELECT t.ok(fn_purge_blend_payload() = 2, 'BP-01 / BP-02: the async blend past its window and the sync blend are purged, the async one inside its window is kept');
SELECT t.ok((SELECT output_payload IS NULL AND payload_purged_at IS NOT NULL FROM blend WHERE blend_id = 'b3000000-0000-7000-8000-0000000000d1'), 'BP-01: payload NULL and payload_purged_at set');
SELECT t.ok((SELECT output_payload IS NOT NULL FROM blend WHERE blend_id = 'b3000000-0000-7000-8000-0000000000d2'), 'BP-01: payload kept until completed_at + ttl_polling_detik (S-05)');
ROLLBACK;
