-- Functions: K-20 mass cancel (U-54), purge P-02/P-03 with H-01, auditor export (E-01..E-06), masked access.
\set ON_ERROR_STOP on

-- ============ U-54: client X has K1 and K2. K1: 10 queued (attempt 0) + 5 waiting retry (attempt 1). K2: 10 queued.
BEGIN;
INSERT INTO client (client_id, nama) VALUES ('10000000-0000-7000-8000-0000000000a1', 'client-u54');
INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, dibuat_oleh) VALUES
  ('11000000-0000-7000-8000-0000000000a1', '10000000-0000-7000-8000-0000000000a1', 'api_key', '\x01', '00000000-0000-7000-8000-000000000001'),
  ('11000000-0000-7000-8000-0000000000a2', '10000000-0000-7000-8000-0000000000a1', 'api_key', '\x02', '00000000-0000-7000-8000-000000000001');
INSERT INTO client_webhook (client_id, webhook_url, aktif, secret_versi, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, diubah_oleh)
  VALUES ('10000000-0000-7000-8000-0000000000a1', 'https://u54.example.test/h', true, 1, '\x11', '\x22', '\x33', 'k', '00000000-0000-7000-8000-000000000001');
SELECT t.buat_request(jsonb_build_object('client_id', '10000000-0000-7000-8000-0000000000a1', 'credential_id', '11000000-0000-7000-8000-0000000000a1',
  'input_payload', '{"NIK":"0000000000000007"}'::jsonb, 'metadata', '{"status_penerima_manfaat":"1","referensi_client":"r"}'::jsonb))
  FROM generate_series(1, 10);
-- 5 waiting for retry: have been dispatched once (log row), back in queued with attempt_count = 1
CREATE TEMP TABLE _w AS SELECT t.buat_request(jsonb_build_object('client_id', '10000000-0000-7000-8000-0000000000a1',
  'credential_id', '11000000-0000-7000-8000-0000000000a1', 'status', 'queued', 'attempt_count', 1,
  'upstream_pool_id', '30000000-0000-7000-8000-000000000001')) AS id FROM generate_series(1, 5);
INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
  provider_config_version, endpoint_config_version, nik, requested_at, status_akses, error_class, log_mode, worker_id)
  SELECT gen_random_uuid(), id, 1, '20000000-0000-7000-8000-000000000001', '30000000-0000-7000-8000-000000000001',
         '10000000-0000-7000-8000-0000000000a1', '50000000-0000-7000-8000-000000000001', 1, 1, '0000000000000008', now(),
         'gagal_provider', 'provider_sementara', 'metadata_only', 'w1' FROM _w;
SELECT t.buat_request(jsonb_build_object('client_id', '10000000-0000-7000-8000-0000000000a1', 'credential_id', '11000000-0000-7000-8000-0000000000a2'))
  FROM generate_series(1, 10);
COMMIT;

SET ROLE pr_admin_api;
SELECT t.ok((SELECT akan_cancelled = 10 AND akan_failed = 5 FROM fn_pratinjau_nonaktif('kredensial', '11000000-0000-7000-8000-0000000000a1')),
  'U-54a: preview is 10 cancelled / 5 failed');
-- not a platform admin -> refused, nothing changes
SELECT t.gagal($$SELECT * FROM fn_nonaktifkan_entitas('kredensial', '11000000-0000-7000-8000-0000000000a1', '00000000-0000-7000-8000-000000000003', 'x')$$,
  '42501', 'auditor cannot deactivate entities');
SELECT t.ok((SELECT jumlah_cancelled = 10 AND jumlah_failed = 5
               FROM fn_nonaktifkan_entitas('kredensial', '11000000-0000-7000-8000-0000000000a1', '00000000-0000-7000-8000-000000000001', 'uji U-54')),
  'U-54a: action reports 10 cancelled / 5 failed');
RESET ROLE;

SELECT t.ok(t.hitung($q$SELECT count(*) FROM request WHERE credential_id = '11000000-0000-7000-8000-0000000000a1' AND status = 'cancelled'
  AND cancel_reason = 'kredensial_nonaktif' AND cancelled_by = '00000000-0000-7000-8000-000000000001' AND attempt_count = 0$q$) = 10, 'K1: 10 cancelled, reason kredensial_nonaktif, cancelled_by = admin id');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM request WHERE credential_id = '11000000-0000-7000-8000-0000000000a1' AND status = 'failed'
  AND failure_reason = 'objek_nonaktif' AND attempt_count = 1 AND final_kode_transaksi IS NOT NULL$q$) = 5, 'K1: 5 waiting-retry requests failed/objek_nonaktif (T-02b)');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM request WHERE credential_id = '11000000-0000-7000-8000-0000000000a2' AND status = 'queued'$q$) = 10, 'K2 requests untouched');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM request WHERE credential_id = '11000000-0000-7000-8000-0000000000a1' AND status = 'cancelled'
  AND input_payload IS NULL AND payload_purged_at IS NOT NULL AND NOT (metadata ? 'status_penerima_manfaat') AND metadata ? 'referensi_client'$q$) = 10,
  'P-02 ran in the same transaction: input and secret-class metadata purged, other metadata kept');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM request WHERE credential_id = '11000000-0000-7000-8000-0000000000a1' AND webhook_status = 'menunggu'$q$) = 15, 'webhook_status = menunggu when the client webhook is active');
-- EN-08 / DQ-37 (e): history rows of the action share terjadi_at and match the audit counts
SELECT t.ok(t.hitung($q$SELECT count(DISTINCT terjadi_at) FROM request_status_history WHERE objek_pemicu_id = '11000000-0000-7000-8000-0000000000a1'$q$) = 1,
  'EN-08: one terjadi_at for the whole mass cancel');
SELECT t.ok(t.hitung($q$
  SELECT count(*) FROM audit_konfigurasi a
   WHERE a.entitas_jenis = 'kredensial' AND a.entitas_id = '11000000-0000-7000-8000-0000000000a1' AND a.aksi = 'cabut'
     AND a.jumlah_cancelled = (SELECT count(*) FROM request_status_history h WHERE h.objek_pemicu_id = a.entitas_id AND h.terjadi_at = a.terjadi_at AND h.status_ke = 'cancelled' AND h.aktor = a.pengguna_id::text)
     AND a.jumlah_failed = (SELECT count(*) FROM request_status_history h WHERE h.objek_pemicu_id = a.entitas_id AND h.terjadi_at = a.terjadi_at AND h.status_ke = 'failed' AND h.aktor = a.pengguna_id::text)$q$) = 1,
  'DQ-37 (e): audit counts reconcile with history rows (difference 0)');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM request_status_history WHERE objek_pemicu_id = '11000000-0000-7000-8000-0000000000a1' AND alasan NOT IN ('kredensial_nonaktif', 'objek_nonaktif')$q$) = 0,
  'DQ-37 (c): reason matches the entity kind');
-- U-54c: revoked is terminal for every path
SELECT t.gagal($$UPDATE client_kredensial SET status = 'aktif' WHERE credential_id = '11000000-0000-7000-8000-0000000000a1'$$, '23514', 'U-54c: owner-level UPDATE dicabut -> aktif blocked by trigger');
-- U-54c: there is no direct revocation path. Only fn_nonaktifkan_entitas may revoke (EN-03); a direct call would
-- skip client ownership, audit_konfigurasi and the mass cancel.
SET ROLE pr_admin_api;
SELECT t.gagal($$SELECT fn_set_status_kredensial('11000000-0000-7000-8000-0000000000a2', 'dicabut', '00000000-0000-7000-8000-000000000001', 'x')$$, '42501',
  'U-54c: pr_admin_api cannot call fn_set_status_kredensial directly (42501)');
SELECT t.gagal($$SELECT fn_set_status_kredensial('11000000-0000-7000-8000-0000000000a1', 'aktif', '00000000-0000-7000-8000-000000000001', 'x')$$, '42501',
  'U-54c: dicabut -> aktif has no path at all');
SELECT t.ok((SELECT status = 'aktif' AND dicabut_at IS NULL FROM client_kredensial WHERE credential_id = '11000000-0000-7000-8000-0000000000a2'),
  'U-54c: the refused direct call changed nothing');
SELECT t.gagal($$SELECT fn_aktifkan_entitas('kredensial', '11000000-0000-7000-8000-0000000000a1', '00000000-0000-7000-8000-000000000001', 'x')$$, '23514',
  'U-54b: credentials cannot be reactivated');
-- U-54f: a replacement credential can be created (one slot is free)
SELECT t.jalan($$INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, dibuat_oleh)
  VALUES ('11000000-0000-7000-8000-0000000000a3', '10000000-0000-7000-8000-0000000000a1', 'api_key', '\x03', '00000000-0000-7000-8000-000000000001')$$,
  'U-54f: new credential K3 accepted after revocation');
RESET ROLE;
SELECT t.ok(t.hitung($q$SELECT count(*) FROM client_kredensial WHERE (status = 'aktif' AND dicabut_at IS NOT NULL) OR (status = 'dicabut' AND (dicabut_at IS NULL OR dicabut_oleh IS NULL))$q$) = 0,
  'DQ-37 (f) = 0 after the credential tests');

-- ---- pool deactivation: EN-04 (upstream pool matches only requests dispatched at least once), C-01 order, reactivation
BEGIN;
CREATE TEMP TABLE _d AS SELECT t.buat_request('{}') AS id;
CREATE TEMP TABLE _e AS SELECT t.buat_request('{"status":"queued","attempt_count":1,"upstream_pool_id":"30000000-0000-7000-8000-000000000001"}') AS id;
INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
  provider_config_version, endpoint_config_version, nik, requested_at, status_akses, error_class, log_mode, worker_id)
  SELECT gen_random_uuid(), id, 1, '20000000-0000-7000-8000-000000000001', '30000000-0000-7000-8000-000000000001',
         '10000000-0000-7000-8000-000000000001', '50000000-0000-7000-8000-000000000001', 1, 1, '0000000000000009', now(), 'gagal_provider', 'provider_sementara', 'metadata_only', 'w1' FROM _e;
SELECT t.ok((SELECT akan_cancelled = 0 FROM fn_pratinjau_nonaktif('upstream_pool', '30000000-0000-7000-8000-000000000001')),
  'EN-04: upstream pool deactivation does not hit a never-dispatched request');
SELECT t.ok((SELECT akan_failed >= 1 FROM fn_pratinjau_nonaktif('upstream_pool', '30000000-0000-7000-8000-000000000001')),
  'EN-04: ... but does hit one waiting for retry');
COMMIT;

BEGIN;
UPDATE downstream_endpoint SET aktif = false WHERE downstream_endpoint_id = '50000000-0000-7000-8000-000000000001';
UPDATE provider SET aktif = false WHERE provider_id = '20000000-0000-7000-8000-000000000001';
SELECT t.ok((SELECT jenis = 'downstream_endpoint' AND cancel_reason = 'endpoint_nonaktif' FROM fn_entitas_nonaktif_pertama((SELECT id FROM _d))),
  'C-01: first inactive entity in the fixed order (endpoint before provider)');
ROLLBACK;

-- ============ purge P-02 / P-03 / H-01
BEGIN;
CREATE TEMP TABLE _p AS SELECT
  t.buat_request('{"status":"queued","input_payload":{"NIK":"0000000000000011"},"metadata":{"status_penerima_manfaat":"1"}}') AS cancelled,
  t.buat_request('{"status":"queued"}') AS queued;
UPDATE request SET status = 'cancelled', cancel_reason = 'client_nonaktif', cancelled_by = 'sistem', completed_at = now(), webhook_status = 'tidak_dikirim'
  WHERE request_id = (SELECT cancelled FROM _p);
INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor, objek_pemicu_id)
  SELECT cancelled, 2, 'queued', 'cancelled', 'client_nonaktif', 'worker', '10000000-0000-7000-8000-000000000001' FROM _p;
SELECT t.ok((SELECT fn_purge_p02(cancelled) FROM _p), 'P-02: cancelled request purged');
SELECT t.ok(NOT (SELECT fn_purge_p02(queued) FROM _p), 'P-02: a queued request is never purged');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM request r JOIN _p ON r.request_id = _p.cancelled WHERE r.input_payload IS NULL AND r.metadata = '{}'::jsonb AND r.payload_purged_at IS NOT NULL$q$) = 1,
  'P-02: payload and secret-class metadata gone');
ROLLBACK;

BEGIN;
-- done + metadata_only, old: one for a provider with data balikan (H-01 hold), one for a provider without
INSERT INTO endpoint_config_versi VALUES ('50000000-0000-7000-8000-000000000001', 2, 1, NULL, '{}', now(), '00000000-0000-7000-8000-000000000001') ON CONFLICT DO NOTHING;
CREATE TEMP TABLE _x AS SELECT gen_random_uuid() AS rid, gen_random_uuid() AS kode, 1 AS n UNION ALL SELECT gen_random_uuid(), gen_random_uuid(), 2;
-- request 1: provider with data balikan
SELECT t.buat_request(jsonb_build_object('request_id', rid, 'status', 'in_flight', 'attempt_count', 1, 'upstream_pool_id', '30000000-0000-7000-8000-000000000001',
  'input_payload', '{"NIK":"0000000000000021"}'::jsonb, 'received_at', '2026-10-01T00:00:00Z')) FROM _x WHERE n = 1;
INSERT INTO provider (provider_id, nama, data_balikan_aktif) VALUES ('20000000-0000-7000-8000-0000000000b2', 'prov-b', false);
INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, kuota_harian) VALUES ('30000000-0000-7000-8000-0000000000b2', '20000000-0000-7000-8000-0000000000b2', 'up-b', 100);
INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, downstream_pool_id) VALUES
  ('50000000-0000-7000-8000-0000000000b2', 'ep-b', '20000000-0000-7000-8000-0000000000b2', '40000000-0000-7000-8000-000000000001');
SELECT t.buat_request(jsonb_build_object('request_id', rid, 'status', 'in_flight', 'attempt_count', 1, 'upstream_pool_id', '30000000-0000-7000-8000-0000000000b2',
  'provider_id', '20000000-0000-7000-8000-0000000000b2', 'downstream_endpoint_id', '50000000-0000-7000-8000-0000000000b2',
  'input_payload', '{"NIK":"0000000000000022"}'::jsonb, 'received_at', '2026-10-01T00:00:00Z')) FROM _x WHERE n = 2;
INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
  provider_config_version, endpoint_config_version, nik, requested_at, status_akses, log_mode, worker_id)
  SELECT kode, rid, 1, CASE n WHEN 1 THEN '20000000-0000-7000-8000-000000000001'::uuid ELSE '20000000-0000-7000-8000-0000000000b2'::uuid END,
         CASE n WHEN 1 THEN '30000000-0000-7000-8000-000000000001'::uuid ELSE '30000000-0000-7000-8000-0000000000b2'::uuid END,
         '10000000-0000-7000-8000-000000000001',
         CASE n WHEN 1 THEN '50000000-0000-7000-8000-000000000001'::uuid ELSE '50000000-0000-7000-8000-0000000000b2'::uuid END,
         1, 1, '00000000000000' || (20 + n), '2026-10-01 00:00+00', 'berhasil', 'metadata_only', 'w1' FROM _x;
UPDATE request r SET status = 'done', completed_at = '2026-10-01 00:01+00', output_payload = '{"a":{"status":"sesuai","skor":1}}',
       final_kode_transaksi = _x.kode, sumber_hasil = 'provider', webhook_status = 'terkirim' FROM _x WHERE r.request_id = _x.rid;
INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) SELECT rid, 2, 'in_flight', 'done', 'worker' FROM _x;
SELECT t.ok((SELECT fn_ditahan_data_balikan(rid) FROM _x WHERE n = 1), 'H-01: done request of a data-balikan provider is held');
SELECT t.ok(NOT (SELECT fn_ditahan_data_balikan(rid) FROM _x WHERE n = 2), 'H-01: no hold for a provider without data balikan');
SET ROLE pr_controller;
SELECT t.ok(fn_purge_p03(100) = 1, 'P-03: purged exactly the unheld done request past completed_at + TTL');
RESET ROLE;
SELECT t.ok(t.hitung($q$SELECT count(*) FROM request r JOIN _x ON r.request_id = _x.rid WHERE _x.n = 1 AND r.input_payload IS NOT NULL AND r.output_payload IS NOT NULL$q$) = 1, 'H-01: held request keeps input and output');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM request r JOIN _x ON r.request_id = _x.rid WHERE _x.n = 2 AND r.input_payload IS NULL AND r.output_payload IS NULL AND r.payload_purged_at IS NOT NULL$q$) = 1, 'P-03: purged request lost input and output');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM request r WHERE status = 'done' AND completed_at > now() - interval '1 hour' AND payload_purged_at IS NOT NULL$q$) = 0, 'P-03: nothing inside the polling window is purged (S-05)');
-- H-02: reported (diterima) releases the hold
INSERT INTO data_balikan_batch (batch_id, provider_id, pemicu, periode_mulai_at, periode_selesai_at, jumlah_item, status, jadwal_kirim_at, payload_hash)
  VALUES ('c0000000-0000-7000-8000-000000000001', '20000000-0000-7000-8000-000000000001', 'periode', now(), now(), 1, 'diterima', now(), repeat('b', 64));
INSERT INTO data_balikan_item SELECT 'c0000000-0000-7000-8000-000000000001', rid, kode, '0000000000000021', '{}', '{}', '2026-10-01 00:00+00' FROM _x WHERE n = 1;
SELECT t.ok(NOT (SELECT fn_ditahan_data_balikan(rid) FROM _x WHERE n = 1), 'H-02: hold released once the batch is diterima');
ROLLBACK;

-- ============ reconciler function (R-02)
BEGIN;
CREATE TEMP TABLE _r AS SELECT t.buat_request('{"status":"in_flight","attempt_count":1,"upstream_pool_id":"30000000-0000-7000-8000-000000000001"}') AS id, gen_random_uuid() AS kode;
INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
  provider_config_version, endpoint_config_version, nik, requested_at, log_mode, worker_id)
  SELECT kode, id, 1, '20000000-0000-7000-8000-000000000001', '30000000-0000-7000-8000-000000000001', '10000000-0000-7000-8000-000000000001',
         '50000000-0000-7000-8000-000000000001', 1, 1, '0000000000000031', now() - interval '10 minutes', 'metadata_only', 'w1' FROM _r;
GRANT SELECT ON _r TO pr_controller;
SET ROLE pr_controller;
SELECT t.gagal($$SELECT fn_tutup_log_tidak_diketahui((SELECT kode FROM _r), 0)$$, '23514', 'R-02: zero threshold rejected');
SELECT t.ok(NOT (SELECT fn_tutup_log_tidak_diketahui(kode, 3600) FROM _r), 'R-02: not closed before the threshold');
SELECT t.ok((SELECT fn_tutup_log_tidak_diketahui(kode, 60) FROM _r), 'R-02: closed as tidak_diketahui past the threshold');
SELECT t.ok(NOT (SELECT fn_tutup_log_tidak_diketahui(kode, 60) FROM _r), 'R-02: a completed row is not closed again');
RESET ROLE;
ROLLBACK;

-- ============ reconciler threshold comes from konfigurasi_platform, not from the caller (R-02, E-03)
-- A caller may only raise the threshold. pr_controller calling (kode, 1) must not close an attempt younger than
-- rekonsiliasi_ambang_detik (60 s by default), otherwise it can close an attempt that is still running.
BEGIN;
CREATE TEMP TABLE _r2 AS SELECT t.buat_request('{"status":"in_flight","attempt_count":1,"upstream_pool_id":"30000000-0000-7000-8000-000000000001"}') AS id, gen_random_uuid() AS kode;
INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
  provider_config_version, endpoint_config_version, nik, requested_at, log_mode, worker_id)
  SELECT kode, id, 1, '20000000-0000-7000-8000-000000000001', '30000000-0000-7000-8000-000000000001', '10000000-0000-7000-8000-000000000001',
         '50000000-0000-7000-8000-000000000001', 1, 1, '0000000000000032', now() - interval '5 seconds', 'metadata_only', 'w1' FROM _r2;
GRANT SELECT ON _r2 TO pr_controller;
SELECT t.ok((SELECT (nilai #>> '{}')::integer = 60 FROM konfigurasi_platform WHERE kunci = 'rekonsiliasi_ambang_detik'),
  'R-02: rekonsiliasi_ambang_detik is 60 s in the seed');
SET ROLE pr_controller;
SELECT t.ok(NOT (SELECT fn_tutup_log_tidak_diketahui(kode, 1) FROM _r2),
  'R-02: (kode, 1) on a 5 s old attempt does not close it: the configuration threshold wins');
RESET ROLE;
SELECT t.ok((SELECT status_akses IS NULL FROM transaction_log l JOIN _r2 ON l.kode_transaksi = _r2.kode),
  'R-02: the refused call left status_akses NULL');
UPDATE konfigurasi_platform SET nilai = '120'::jsonb WHERE kunci = 'rekonsiliasi_ambang_detik';
SET ROLE pr_controller;
SELECT t.ok(NOT (SELECT fn_tutup_log_tidak_diketahui(kode, 60) FROM _r2),
  'R-02: after raising the configuration to 120 s a caller value of 60 no longer closes it');
-- a caller cannot close it via the threshold while requested_at is in the future (the test trades the age for the
-- check above), but the guard itself is not "caller always wins": with the configuration at 120 s, a caller value
-- of 60 stays refused, and a caller value above it opens the window only for rows old enough.
SELECT t.ok(NOT (SELECT fn_tutup_log_tidak_diketahui(kode, 120) FROM _r2),
  'R-02: a 5 s old attempt stays open even when the caller passes the configuration value itself');
RESET ROLE;
ROLLBACK;
-- an operator asking for the auditor functions is refused and the refusal is logged
SELECT count(*) AS n_ditolak_sebelum FROM audit_akses WHERE aksi = 'akses_ditolak' \gset
SET ROLE pr_admin_api;
SELECT t.ok((SELECT count(*) FROM fn_auditor_lihat_transaksi('00000000-0000-7000-8000-000000000004', 'coba', 'b0000000-0000-7000-8000-000000000001', '10.0.0.1', 'sesi-op')) = 0,
  'operator gets no full NIK from the auditor function');
SELECT t.ok((SELECT count(*) FROM fn_auditor_cari_nik('00000000-0000-7000-8000-000000000004', '0000000000001234', repeat('a', 64), '2026-11-01Z', '2026-11-05Z', 'coba', '10.0.0.1', 'sesi-op')) = 0,
  'operator cannot search by NIK');
SELECT t.ok((SELECT berkas IS NULL AND hasil = 'ditolak' FROM fn_ekspor_log_auditor('00000000-0000-7000-8000-000000000004', gen_random_uuid(), 'coba', NULL, NULL, '2026-11-01Z', '2026-11-05Z', NULL, '10.0.0.1', 'sesi-op')),
  'operator cannot export (A-04)');
RESET ROLE;
SELECT t.ok(t.hitung($q$SELECT count(*) FROM audit_akses WHERE aksi = 'akses_ditolak' AND hasil = 'ditolak'$q$) - :n_ditolak_sebelum = 3, 'each refusal wrote one akses_ditolak row');

SET ROLE pr_admin_api;
SELECT t.ok((SELECT nik = '0000000000001234' FROM fn_auditor_lihat_transaksi('00000000-0000-7000-8000-000000000003', 'investigasi U-1', 'b0000000-0000-7000-8000-000000000001', '10.0.0.2', 'sesi-au')),
  'auditor reads the full NIK with a reason');
SELECT t.gagal($$SELECT * FROM fn_auditor_lihat_transaksi('00000000-0000-7000-8000-000000000003', '', 'b0000000-0000-7000-8000-000000000001', '10.0.0.2', 'sesi-au')$$, '23514', 'reason is mandatory');
SELECT t.ok((SELECT count(*) FROM fn_auditor_cari_nik('00000000-0000-7000-8000-000000000003', '0000000000001234', repeat('a', 64), '2026-11-01Z', '2026-11-05Z', 'US-14', '10.0.0.2', 'sesi-au')) = 1,
  'US-14: auditor finds the attempt by NIK and time range');
RESET ROLE;
SELECT t.ok(t.hitung($q$SELECT count(*) FROM audit_akses WHERE aksi = 'lihat_payload' AND tampilan_nik = 'utuh' AND alasan = 'investigasi U-1' AND target_kode_transaksi = 'b0000000-0000-7000-8000-000000000001'$q$) = 1,
  'full view wrote exactly one audit row in the same transaction');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM audit_akses WHERE aksi = 'cari_nik' AND nik_hmac = repeat('a', 64) AND filter_kueri::text !~ '[0-9]{16}'$q$) = 1,
  'NIK search audit row holds only the HMAC and masked NIK');

-- export: bytes, checksum and the audit row from one function
SET ROLE pr_admin_api;
INSERT INTO t.hasil_ekspor SELECT 'a', * FROM fn_ekspor_log_auditor('00000000-0000-7000-8000-000000000003', 'd0000000-0000-7000-8000-000000000001', 'audit rutin',
  repeat('c', 64), '************1234', '2026-11-01Z', '2026-11-05Z', NULL, '10.0.0.2', 'sesi-au');
SELECT t.ok((SELECT hasil = 'diizinkan' AND jumlah_record >= 1 AND ekspor_checksum = encode(sha256(berkas), 'hex') FROM t.hasil_ekspor WHERE nama = 'a'), 'E-02: checksum = sha256 of the delivered bytes');
SELECT t.ok((SELECT convert_from(berkas, 'UTF8') LIKE E'nik,requested_at,status_akses,kode_transaksi,queue_id,client_id\n0000000000001234,2026-11-03T08:00:00.000000+07:00,berhasil,%' FROM t.hasil_ekspor WHERE nama = 'a'),
  'E-04: header, text NIK, +07:00 timestamps');
SELECT t.ok((SELECT convert_from(berkas, 'UTF8') !~ E'\r' AND right(convert_from(berkas, 'UTF8'), 1) = E'\n' FROM t.hasil_ekspor WHERE nama = 'a'), 'E-04: LF line endings, trailing newline');
RESET ROLE;
SELECT t.ok(t.hitung($q$SELECT count(*) FROM audit_akses a JOIN t.hasil_ekspor e ON e.nama = 'a' AND e.audit_id = a.audit_id WHERE a.aksi = 'ekspor' AND a.hasil = 'diizinkan'
  AND a.ekspor_checksum = e.ekspor_checksum AND a.jumlah_record = e.jumlah_record AND a.alasan = 'audit rutin'$q$) = 1, 'E-01/E-05: one audit row carrying the checksum');
-- E-03: over the row limit -> no file, ditolak, checksum NULL, jumlah_record = matching rows
UPDATE konfigurasi_platform SET nilai = '1'::jsonb WHERE kunci = 'ekspor_batas_baris';
SET ROLE pr_admin_api;
INSERT INTO t.hasil_ekspor SELECT 'b', * FROM fn_ekspor_log_auditor('00000000-0000-7000-8000-000000000003', 'd0000000-0000-7000-8000-000000000002', 'audit besar',
  repeat('c', 64), '************1234', '2026-10-01Z', '2026-12-05Z', NULL, '10.0.0.2', 'sesi-au');
RESET ROLE;
UPDATE konfigurasi_platform SET nilai = '10000'::jsonb WHERE kunci = 'ekspor_batas_baris';
SELECT t.ok((SELECT berkas IS NULL AND ekspor_checksum IS NULL AND hasil = 'ditolak' AND jumlah_record > 1 FROM t.hasil_ekspor WHERE nama = 'b'), 'E-03: over the limit returns no file');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM audit_akses WHERE audit_id = 'd0000000-0000-7000-8000-000000000002' AND hasil = 'ditolak' AND ekspor_checksum IS NULL AND aksi = 'ekspor'$q$) = 1,
  'E-03: one ditolak audit row without checksum');
-- DQ-35 on the test data
SELECT t.ok(t.hitung($q$SELECT count(*) FROM audit_akses WHERE aksi = 'ekspor' AND hasil = 'diizinkan'
  AND (ekspor_checksum !~ '^[0-9a-f]{64}$' OR alasan IS NULL OR nik_hmac IS NULL OR peran <> 'auditor')$q$) = 0, 'DQ-35 (a)(c) = 0');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM audit_akses WHERE aksi = 'ekspor' AND hasil = 'ditolak' AND ekspor_checksum IS NOT NULL$q$) = 0, 'DQ-35 (b) = 0');
-- E-06: reading the audit is audited, no nik_hmac in the response
SET ROLE pr_admin_api;
SELECT t.ok((SELECT count(*) FROM fn_lihat_audit('00000000-0000-7000-8000-000000000003', '2026-01-01Z', '2027-01-01Z', 1000, '10.0.0.2', 'sesi-au')) >= 5, 'fn_lihat_audit returns rows');
SELECT t.gagal($$SELECT nik_hmac FROM fn_lihat_audit('00000000-0000-7000-8000-000000000003', '2026-01-01Z', '2027-01-01Z', 10, '10.0.0.2', 'sesi-au')$$, '42703', 'E-06: response has no nik_hmac');
RESET ROLE;
SELECT t.ok(t.hitung($q$SELECT count(*) FROM audit_akses WHERE aksi = 'lihat_audit'$q$) = 1, 'E-06: each read of audit_akses wrote a lihat_audit row');

-- client owner and webhook-log row filters
INSERT INTO admin_user (pengguna_id, username, password_hash, client_id) VALUES ('00000000-0000-7000-8000-000000000005', 'owner-x', '$argon2id$synthetic', '10000000-0000-7000-8000-000000000001');
INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh) VALUES ('00000000-0000-7000-8000-000000000005', 'client_owner', '00000000-0000-7000-8000-000000000001');
SET ROLE pr_admin_api;
SELECT t.ok((SELECT count(*) > 0 AND bool_and(client_id = '10000000-0000-7000-8000-000000000001') FROM fn_request_milik_client('00000000-0000-7000-8000-000000000005')),
  'client owner sees only requests of their own client');
SELECT t.ok((SELECT count(*) = 0 FROM fn_request_milik_client('00000000-0000-7000-8000-000000000004')), 'an operator is not a client owner: no rows');

-- U-54c (cross client, EN-03): owner A's credential belongs to client-u54; owner of client-x must not revoke it,
-- even through the sanctioned function. Client ownership is checked inside fn_nonaktifkan_entitas only.
INSERT INTO admin_user (pengguna_id, username, password_hash, client_id) VALUES
  ('00000000-0000-7000-8000-000000000006', 'owner-u54', '$argon2id$synthetic', '10000000-0000-7000-8000-0000000000a1');
INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh) VALUES
  ('00000000-0000-7000-8000-000000000006', 'client_owner', '00000000-0000-7000-8000-000000000001');
SET ROLE pr_admin_api;
SELECT t.gagal($$SELECT * FROM fn_nonaktifkan_entitas('kredensial', '11000000-0000-7000-8000-0000000000a2', '00000000-0000-7000-8000-000000000005', 'x')$$,
  '42501', 'U-54c: client owner A cannot revoke a credential of client B through fn_nonaktifkan_entitas (42501)');
RESET ROLE;
SELECT t.ok((SELECT status = 'aktif' FROM client_kredensial WHERE credential_id = '11000000-0000-7000-8000-0000000000a2'),
  'U-54c: the refused cross-client call left the credential aktif');
-- the owner of the credential's own client may revoke it, and that path writes audit and cancels waiting requests
SET ROLE pr_admin_api;
SELECT t.ok((SELECT jumlah_cancelled = 10 AND jumlah_failed = 0
               FROM fn_nonaktifkan_entitas('kredensial', '11000000-0000-7000-8000-0000000000a2', '00000000-0000-7000-8000-000000000006', 'uji U-54c')),
  'U-54c: the owning client owner revokes through fn_nonaktifkan_entitas');
RESET ROLE;
SELECT t.ok((SELECT status = 'dicabut' AND dicabut_oleh = '00000000-0000-7000-8000-000000000006' AND dicabut_at IS NOT NULL
               FROM client_kredensial WHERE credential_id = '11000000-0000-7000-8000-0000000000a2'),
  'U-54c: revoking client owner recorded as dicabut_oleh');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM request WHERE credential_id = '11000000-0000-7000-8000-0000000000a2'$q$) = 10
        AND t.hitung($q$SELECT count(*) FROM request WHERE credential_id = '11000000-0000-7000-8000-0000000000a2' AND status = 'cancelled'
                       AND cancel_reason = 'kredensial_nonaktif' AND cancelled_by = '00000000-0000-7000-8000-000000000006'$q$) = 10,
  'U-54c: 10 waiting requests of K2 cancelled with the revoking owner as cancelled_by');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM audit_konfigurasi WHERE entitas_jenis = 'kredensial'
                       AND entitas_id = '11000000-0000-7000-8000-0000000000a2' AND aksi = 'cabut'
                       AND pengguna_id = '00000000-0000-7000-8000-000000000006'$q$) = 1,
  'U-54c: the sanctioned revocation wrote exactly one audit_konfigurasi row (EN-03, DQ-37 e)');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM pg_proc p WHERE p.proname = 'fn_set_status_kredensial'
                       AND has_function_privilege('pr_admin_api', p.oid, 'EXECUTE')$q$) = 0,
  'U-54c: pr_admin_api has no EXECUTE on fn_set_status_kredensial in the catalog');
RESET ROLE;

-- append-only now that rows exist (DQ-21)
SELECT t.gagal($$UPDATE audit_akses SET peran = 'x'$$, '23000', 'audit_akses is append-only (update)');
SELECT t.gagal($$DELETE FROM audit_akses$$, '23000', 'audit_akses is append-only (delete)');
SELECT t.gagal($$TRUNCATE audit_akses$$, '23000', 'audit_akses is append-only (truncate)');
SELECT t.gagal($$UPDATE audit_konfigurasi SET alasan = 'x'$$, '23000', 'audit_konfigurasi is append-only (update)');
SELECT t.gagal($$DELETE FROM request_status_history$$, '23000', 'request_status_history is append-only');

-- every application function refuses PUBLIC
SELECT t.ok(t.hitung($q$SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
  WHERE n.nspname = 'public' AND p.prokind = 'f' AND has_function_privilege('public', p.oid, 'EXECUTE')
    AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.objid = p.oid AND d.deptype = 'e')$q$) = 0, 'no function is executable by PUBLIC');
SELECT t.ok(t.hitung($q$SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
  WHERE n.nspname = 'public' AND p.prosecdef AND (p.proconfig IS NULL OR NOT EXISTS (SELECT 1 FROM unnest(p.proconfig) c WHERE c LIKE 'search_path=%'))$q$) = 0,
  'every SECURITY DEFINER function pins search_path');

-- ============ uuidv7() (R-02 rows written by functions; contract 3 ID UUIDv7)
SELECT t.ok((SELECT get_byte(uuid_send(uuidv7()), 6) >> 4 = 7), 'uuidv7: version nibble is 7');
SELECT t.ok((SELECT get_byte(uuid_send(uuidv7()), 8) >> 6 = 2), 'uuidv7: variant bits are 10 (RFC 9562)');
-- bit(60) then / 4096 reads the 48-bit prefix without sign extension
SELECT t.ok((SELECT abs(extract(epoch FROM now() - to_timestamp(
                      (('x' || substr(replace(uuidv7()::text, '-', ''), 1, 12))::bit(60)::bigint / 4096) / 1000.0))) < 5),
  'uuidv7: the 48-bit prefix is the current unix_ts_ms');
SELECT t.ok((SELECT count(DISTINCT uuidv7()) = 1000 FROM generate_series(1, 1000)), 'uuidv7: 1000 calls, 1000 distinct values');

-- every id a function writes is a v7 uuid: audit_akses.audit_id (all audit functions) and
-- audit_konfigurasi.audit_konfigurasi_id (fn_nonaktifkan_entitas / fn_aktifkan_entitas).
SELECT t.ok((SELECT count(*) > 0 AND bool_and((get_byte(uuid_send(audit_id), 6) >> 4 = 7)::int = 1) FROM audit_akses),
  'audit_akses.audit_id written by functions is UUIDv7');
SELECT t.ok((SELECT count(*) > 0 AND bool_and((get_byte(uuid_send(audit_konfigurasi_id), 6) >> 4 = 7)::int = 1) FROM audit_konfigurasi),
  'audit_konfigurasi.audit_konfigurasi_id written by functions is UUIDv7');
SELECT t.ok((SELECT count(*) > 0 AND bool_and((get_byte(uuid_send(audit_id), 8) >> 6 = 2)::int = 1) FROM audit_akses),
  'audit_akses.audit_id has the RFC 9562 variant bits');
