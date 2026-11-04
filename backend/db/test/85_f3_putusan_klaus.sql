-- F3 behaviour test, part 3 (SHAA-354): cases required by the final design ruling (Klaus, items 2, 4, 6, 7) and the terminal
-- transitions DQ-61 (b)-(e) with children in every terminal state. Uses t.f3_siapkan() from 75_f3_constraints.sql.
-- Synthetic data only; every block is rolled back.
\set ON_ERROR_STOP on

-- Moves one child (queued) to a terminal state through the legal path, with its history rows (T-04).
CREATE OR REPLACE FUNCTION t.anak_ke(p_rid uuid, p_ke text) RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_seq integer; v_log uuid;
BEGIN
  SELECT coalesce(max(seq), 0) INTO v_seq FROM request_status_history WHERE request_id = p_rid;
  IF p_ke IN ('done', 'failed') THEN
    UPDATE request SET status = 'dispatched', dispatched_at = now() WHERE request_id = p_rid;
    INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) VALUES (p_rid, v_seq + 1, 'queued', 'dispatched', 'controller');
    UPDATE request SET status = 'in_flight', attempt_count = 1, first_attempt_at = now() WHERE request_id = p_rid;
    INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) VALUES (p_rid, v_seq + 2, 'dispatched', 'in_flight', 'worker');
    IF p_ke = 'done' THEN
      v_log := t.buat_log(p_rid, '{"status_akses":"berhasil"}');
      UPDATE request SET status = 'done', completed_at = now(), output_payload = '{"dukcapil":{"status":"sesuai","skor":1}}', final_kode_transaksi = v_log,
             sumber_hasil = 'provider', webhook_status = 'tidak_dikirim' WHERE request_id = p_rid;
      INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) VALUES (p_rid, v_seq + 3, 'in_flight', 'done', 'worker');
    ELSE
      v_log := t.buat_log(p_rid, '{"status_akses":"tidak_diketahui"}');
      UPDATE request SET status = 'failed', failure_reason = 'retry_habis', completed_at = now(), final_kode_transaksi = v_log,
             webhook_status = 'tidak_dikirim' WHERE request_id = p_rid;
      INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor) VALUES (p_rid, v_seq + 3, 'in_flight', 'failed', 'retry_habis', 'worker');
    END IF;
  ELSIF p_ke = 'expired' THEN
    UPDATE request SET status = 'expired', completed_at = now(), webhook_status = 'tidak_dikirim' WHERE request_id = p_rid;
    INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) VALUES (p_rid, v_seq + 1, 'queued', 'expired', 'controller');
  ELSE
    UPDATE request SET status = 'cancelled', cancel_reason = 'dibatalkan_client', cancelled_by = 'client', completed_at = now(),
           webhook_status = 'tidak_dikirim' WHERE request_id = p_rid;
    INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor) VALUES (p_rid, v_seq + 1, 'queued', 'cancelled', 'dibatalkan_client', 'client');
  END IF;
END $$;

-- Children a1 / a2 end in p_a / p_b, then the blend b3..01 is closed to p_ke with a summary built from the children.
CREATE OR REPLACE FUNCTION t.skenario(p_a text, p_b text, p_ke text) RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_ring jsonb; v_penentu uuid;
BEGIN
  PERFORM t.f3_siapkan();
  PERFORM t.anak_ke('f3000000-0000-7000-8000-0000000000a1', p_a);
  PERFORM t.anak_ke('f3000000-0000-7000-8000-0000000000a2', p_b);
  SELECT jsonb_agg(jsonb_build_object('nama_komponen', k.nama_komponen, 'request_id', r.request_id, 'status_api', r.status) ORDER BY k.prioritas)
    INTO v_ring FROM request r JOIN endpoint_blend_komponen k ON k.endpoint_blend_id = 'e3000000-0000-7000-8000-000000000001' AND k.config_version = 1
                                                              AND k.downstream_endpoint_id = r.downstream_endpoint_id
   WHERE r.blend_id = 'b3000000-0000-7000-8000-000000000001';
  SELECT request_id INTO v_penentu FROM request WHERE blend_id = 'b3000000-0000-7000-8000-000000000001' ORDER BY (status = 'failed') DESC, request_id LIMIT 1;
  UPDATE blend SET status = p_ke, completed_at = now(), ringkasan_komponen = v_ring, webhook_status = 'menunggu',
         output_payload = CASE WHEN p_ke = 'done' THEN '{"dukcapil":{"status":"sesuai"}}'::jsonb END,
         failure_reason = CASE WHEN p_ke = 'failed' THEN 'retry_habis' END, anak_penentu_request_id = CASE WHEN p_ke = 'failed' THEN v_penentu END,
         cancel_reason = CASE WHEN p_ke = 'cancelled' THEN 'dibatalkan_client' END, cancelled_by = CASE WHEN p_ke = 'cancelled' THEN 'client' END
   WHERE blend_id = 'b3000000-0000-7000-8000-000000000001';
  INSERT INTO blend_status_history (blend_id, seq, status_dari, status_ke, alasan, aktor)
    VALUES ('b3000000-0000-7000-8000-000000000001', 2, 'berjalan', p_ke,
            CASE p_ke WHEN 'failed' THEN 'retry_habis' WHEN 'cancelled' THEN 'dibatalkan_client' END, 'controller');
END $$;
GRANT EXECUTE ON FUNCTION t.anak_ke(uuid, text), t.skenario(text, text, text) TO PUBLIC;

-- ================================================================ DQ-61 (b)-(e), BL-05: closing states against children in every terminal state
BEGIN; SELECT t.jalan_commit($$SELECT t.skenario('done', 'done', 'done')$$, 'BL-05.1: done with every child done is accepted'); ROLLBACK;
BEGIN; SELECT t.gagal_commit($$SELECT t.skenario('done', 'expired', 'done')$$, '23000', 'BL-05.1: done with an expired child is refused'); ROLLBACK;
BEGIN; SELECT t.gagal_commit($$SELECT t.skenario('done', 'failed', 'done')$$, '23000', 'BL-05.1: done with a failed child is refused'); ROLLBACK;
BEGIN; SELECT t.jalan_commit($$SELECT t.skenario('done', 'failed', 'failed')$$, 'BL-05.2: failed with a failed deciding child is accepted'); ROLLBACK;
BEGIN; SELECT t.jalan_commit($$SELECT t.skenario('failed', 'cancelled', 'failed')$$, 'BL-05.2: failed with a cancelled sibling is accepted'); ROLLBACK;
BEGIN; SELECT t.gagal_commit($$SELECT t.skenario('done', 'done', 'failed')$$, '23000', 'BL-05: failed with every child done is refused'); ROLLBACK;
BEGIN; SELECT t.jalan_commit($$SELECT t.skenario('cancelled', 'cancelled', 'cancelled')$$, 'DQ-61 c: cancelled with all children cancelled and no attempts is accepted'); ROLLBACK;
BEGIN; SELECT t.gagal_commit($$SELECT t.skenario('cancelled', 'done', 'cancelled')$$, '23000', 'DQ-61 c: cancelled while a child had an attempt is refused'); ROLLBACK;
BEGIN; SELECT t.jalan_commit($$SELECT t.skenario('expired', 'expired', 'expired')$$, 'DQ-61 d: expired with all children expired is accepted'); ROLLBACK;
BEGIN; SELECT t.gagal_commit($$SELECT t.skenario('expired', 'done', 'expired')$$, '23000', 'DQ-61 d: expired while a child is done is refused'); ROLLBACK;
BEGIN; SELECT t.gagal_commit($$SELECT t.skenario('expired', 'cancelled', 'expired')$$, '23000', 'DQ-61 d: expired while a child is cancelled is refused'); ROLLBACK;
BEGIN; SELECT t.gagal_commit($$SELECT t.skenario('expired', 'failed', 'expired')$$, '23000', 'DQ-61 d: expired while a child is failed is refused'); ROLLBACK;
BEGIN;
SELECT t.skenario('done', 'done', 'done');
SELECT t.gagal($$UPDATE blend SET status = 'failed' WHERE status = 'done'$$, '23514', 'T-05: a done blend never becomes failed');
ROLLBACK;

-- ================================================================ item 4: DQ-63 (c) one way for HTTP; objek_nonaktif/403 is not a gate rejection
BEGIN;
SELECT t.f3_siapkan();
SELECT t.jalan($$INSERT INTO blend (blend_id, client_id, credential_id, endpoint_blend_id, endpoint_blend_config_version, mode, status, reject_reason, reject_http_status,
  gerbang_dasar_hukum, batas_waktu_at, requested_at, completed_at, webhook_status)
  VALUES (gen_random_uuid(), '10000000-0000-7000-8000-000000000001', '11000000-0000-7000-8000-000000000001', 'e3000000-0000-7000-8000-000000000001', 1, 'async', 'rejected',
          'objek_nonaktif', 403, 'tidak_berlaku', now(), now(), now(), 'tidak_dikirim')$$,
  'DQ-63 c: objek_nonaktif/403 is accepted and carries no gate proof');
SELECT t.gagal($$INSERT INTO blend (blend_id, client_id, credential_id, endpoint_blend_id, endpoint_blend_config_version, mode, status, reject_reason, reject_http_status,
  gerbang_dasar_hukum, batas_waktu_at, requested_at, completed_at, webhook_status)
  VALUES (gen_random_uuid(), '10000000-0000-7000-8000-000000000001', '11000000-0000-7000-8000-000000000001', 'e3000000-0000-7000-8000-000000000001', 1, 'async', 'rejected',
          'objek_nonaktif', 403, 'ditolak', now(), now(), now(), 'tidak_dikirim')$$, '23514',
  'DQ-63 c: objek_nonaktif/403 must not be marked as a gate rejection');
ROLLBACK;

-- ================================================================ item 6: ttl_polling_detik, three sides
BEGIN;
SELECT t.f3_siapkan();
-- (1) async, not rejected: filled (the happy path of t.f3_siapkan carries 86400) and required
SELECT t.ok((SELECT ttl_polling_detik FROM blend WHERE blend_id = 'b3000000-0000-7000-8000-000000000001') = 86400, 'item 6: an accepted async blend carries ttl_polling_detik');
SELECT t.gagal($$INSERT INTO blend SELECT (jsonb_populate_record(NULL::blend, to_jsonb(u) || '{"blend_id":"b3000000-0000-7000-8000-0000000000e1","ttl_polling_detik":null}'::jsonb)).*
  FROM blend u WHERE blend_id = 'b3000000-0000-7000-8000-000000000001'$$, '23514', 'item 6: an async blend that is not rejected cannot have a NULL ttl_polling_detik');
-- (2) async rejected: NULL is the rule (the GB-05 case in 75_f3_constraints.sql inserts it without a value); (3) filled is refused
SELECT t.gagal($$INSERT INTO blend (blend_id, client_id, credential_id, endpoint_blend_id, endpoint_blend_config_version, mode, status, reject_reason, reject_http_status,
  gerbang_dasar_hukum, batas_waktu_at, requested_at, completed_at, webhook_status, ttl_polling_detik)
  VALUES (gen_random_uuid(), '10000000-0000-7000-8000-000000000001', '11000000-0000-7000-8000-000000000001', 'e3000000-0000-7000-8000-000000000001', 1, 'async', 'rejected',
          'kuota_habis', 429, 'tidak_berlaku', now(), now(), now(), 'tidak_dikirim', 100)$$, '23514', 'item 6: an async rejected blend with ttl_polling_detik is refused');
SELECT t.jalan($$INSERT INTO blend (blend_id, client_id, credential_id, endpoint_blend_id, endpoint_blend_config_version, mode, status, reject_reason, reject_http_status,
  gerbang_dasar_hukum, batas_waktu_at, requested_at, completed_at, webhook_status)
  VALUES (gen_random_uuid(), '10000000-0000-7000-8000-000000000001', '11000000-0000-7000-8000-000000000001', 'e3000000-0000-7000-8000-000000000001', 1, 'async', 'rejected',
          'kuota_habis', 429, 'tidak_berlaku', now(), now(), now(), 'tidak_dikirim')$$, 'item 6: an async rejected blend with NULL ttl_polling_detik is accepted');
ROLLBACK;

-- ================================================================ item 7: fungsi_gagal is in blend.failure_reason, not in the F0 request enum
BEGIN;
SELECT t.f3_siapkan();
SELECT t.gagal($$UPDATE request SET status = 'failed', failure_reason = 'fungsi_gagal' WHERE request_id = 'f3000000-0000-7000-8000-0000000000a1'$$, '23514',
  'item 7: request.failure_reason refuses fungsi_gagal (F0 enum untouched)');
SELECT t.ok(NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conrelid = 'request'::regclass AND contype = 'c' AND pg_get_constraintdef(oid) LIKE '%fungsi_gagal%'),
  'item 7: no request constraint mentions fungsi_gagal');
SELECT t.ok(EXISTS (SELECT 1 FROM pg_constraint WHERE conrelid = 'blend'::regclass AND contype = 'c' AND pg_get_constraintdef(oid) LIKE '%failure_reason%fungsi_gagal%'),
  'item 7: blend.failure_reason accepts fungsi_gagal');
ROLLBACK;

-- ================================================================ item 2: gate trail ordering on equal timestamps
BEGIN;
SELECT t.ok(fn_catat_gerbang_blend('true', 'DOC-U-1', 'i1') = 'gerbang_blend_dibuka', 'GB-09: open');
SELECT t.ok(fn_catat_gerbang_blend('false', NULL, 'i1') = 'gerbang_blend_ditutup', 'GB-09: closed');
SELECT t.ok(fn_catat_gerbang_blend('true', 'DOC-U-1', 'i1') = 'gerbang_blend_dibuka',
  'GB-09: re-opening with the same reference after a close writes again (order follows peristiwa_id, not the identical timestamp)');
SELECT t.ok((SELECT count(DISTINCT terjadi_at) FROM audit_gerbang_blend) = 1 AND (SELECT count(*) FROM audit_gerbang_blend) = 3,
  'GB-09: the three events share one timestamp and are still all kept');
SELECT t.ok((SELECT jenis FROM audit_gerbang_blend ORDER BY peristiwa_id DESC LIMIT 1) = 'gerbang_blend_dibuka', 'GB-09: the last event by peristiwa_id is the final state');
ROLLBACK;
