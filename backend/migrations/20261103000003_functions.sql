-- F0 schema, part 3: database functions and masked views (SHAA-193).
-- Purge predicates P-02/P-03 with hold rules H-01/H-02, K-20 activation and mass cancel (EN-03..EN-09, C-01),
-- credential status (EN-03), auditor access with audit_akses in the same transaction (A-01, E-01..E-06).
-- Every function revokes EXECUTE from PUBLIC; part 4 grants it per role.
-- SECURITY DEFINER functions pin search_path (they run as poolrouter_owner).

-- ---------------------------------------------------------------- helpers
CREATE FUNCTION fn_metadata_tanpa_rahasia(p_endpoint uuid, p_metadata jsonb) RETURNS jsonb
LANGUAGE sql STABLE SET search_path = pg_catalog, public AS $$
  SELECT coalesce(p_metadata, '{}'::jsonb) - coalesce(
    (SELECT array_agg(nama_field::text) FROM endpoint_metadata_field
      WHERE downstream_endpoint_id = p_endpoint AND klasifikasi = 'Rahasia'), ARRAY[]::text[]);
$$;

-- C-01: first inactive entity in the fixed order client -> credential -> endpoint -> downstream pool -> upstream pool -> provider.
-- SECURITY INVOKER: the worker and controller only need the status-column grants of contract 8.1b.
CREATE FUNCTION fn_entitas_nonaktif_pertama(p_request_id uuid)
RETURNS TABLE (jenis text, entitas_id uuid, cancel_reason text)
LANGUAGE sql STABLE SET search_path = pg_catalog, public AS $$
  SELECT x.jenis, x.id, x.cancel_reason FROM (
    SELECT 1 AS urut, 'client'::text AS jenis, r.client_id AS id, 'client_nonaktif'::text AS cancel_reason
      FROM request r JOIN client c ON c.client_id = r.client_id WHERE r.request_id = p_request_id AND NOT c.aktif
    UNION ALL
    SELECT 2, 'kredensial', r.credential_id, 'kredensial_nonaktif'
      FROM request r JOIN client_kredensial k ON k.credential_id = r.credential_id WHERE r.request_id = p_request_id AND k.status <> 'aktif'
    UNION ALL
    SELECT 3, 'downstream_endpoint', r.downstream_endpoint_id, 'endpoint_nonaktif'
      FROM request r JOIN downstream_endpoint e ON e.downstream_endpoint_id = r.downstream_endpoint_id WHERE r.request_id = p_request_id AND NOT e.aktif
    UNION ALL
    SELECT 4, 'downstream_pool', r.downstream_pool_id, 'pool_nonaktif'
      FROM request r JOIN downstream_pool d ON d.downstream_pool_id = r.downstream_pool_id WHERE r.request_id = p_request_id AND NOT d.aktif
    UNION ALL
    -- EN-04: only when upstream_pool_id is set
    SELECT 5, 'upstream_pool', r.upstream_pool_id, 'pool_nonaktif'
      FROM request r JOIN upstream_pool u ON u.upstream_pool_id = r.upstream_pool_id WHERE r.request_id = p_request_id AND NOT u.aktif
    UNION ALL
    SELECT 6, 'provider', r.provider_id, 'provider_nonaktif'
      FROM request r JOIN provider p ON p.provider_id = r.provider_id WHERE r.request_id = p_request_id AND NOT p.aktif
  ) x ORDER BY x.urut LIMIT 1;
$$;

-- ---------------------------------------------------------------- purge (3.4)
-- P-02: expired / cancelled requests lose input_payload and secret-class metadata at the terminal transition.
-- Only fills NULL and payload_purged_at; the predicate does not read payload columns.
CREATE FUNCTION fn_purge_p02(p_request_id uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_n integer;
BEGIN
  UPDATE request r
     SET input_payload = NULL,
         output_payload = NULL,
         metadata = fn_metadata_tanpa_rahasia(r.downstream_endpoint_id, r.metadata),
         payload_purged_at = now()
   WHERE r.request_id = p_request_id AND r.status IN ('expired', 'cancelled') AND r.payload_purged_at IS NULL;
  GET DIAGNOSTICS v_n = ROW_COUNT;
  RETURN v_n = 1;
END $$;

-- H-01/H-02: a done request of a provider with data balikan stays on line until its batch is "diterima".
CREATE FUNCTION fn_ditahan_data_balikan(p_request_id uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
  SELECT EXISTS (
    SELECT 1 FROM request r JOIN provider p ON p.provider_id = r.provider_id
     WHERE r.request_id = p_request_id AND p.data_balikan_aktif AND r.status = 'done' AND r.sumber_hasil = 'provider'
       AND NOT EXISTS (SELECT 1 FROM data_balikan_item i JOIN data_balikan_batch b ON b.batch_id = i.batch_id
                        WHERE i.request_id = r.request_id AND b.status = 'diterima'));
$$;

-- P-03: done / failed requests whose final attempt was metadata_only. Output is kept until completed_at + queue TTL
-- (async) regardless of webhook_status (S-05); sync output goes right after the response. H-01 rows are skipped.
CREATE FUNCTION fn_purge_p03(p_batas integer DEFAULT 1000) RETURNS integer
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_n integer;
BEGIN
  WITH kandidat AS (
    SELECT r.request_id
      FROM request r
      JOIN transaction_log l ON l.kode_transaksi = r.final_kode_transaksi
      LEFT JOIN downstream_pool d ON d.downstream_pool_id = r.downstream_pool_id
      WHERE r.status IN ('done', 'failed') AND r.output_purged_at IS NULL AND l.log_mode = 'metadata_only'
       AND r.completed_at + CASE WHEN r.mode = 'sync' THEN interval '0' ELSE make_interval(secs => coalesce(d.ttl_antrean_detik, 0)) END <= now()
       AND NOT fn_ditahan_data_balikan(r.request_id)
     ORDER BY r.completed_at
     LIMIT p_batas
     FOR UPDATE OF r SKIP LOCKED)
  UPDATE request r
     SET input_payload = CASE WHEN fn_ditahan_data_balikan(r.request_id) THEN r.input_payload ELSE NULL END,
         output_payload = NULL, output_purged_at = now(),
         metadata = fn_metadata_tanpa_rahasia(r.downstream_endpoint_id, r.metadata),
         payload_purged_at = now()
    FROM kandidat k WHERE r.request_id = k.request_id;
  GET DIAGNOSTICS v_n = ROW_COUNT;
  RETURN v_n;
END $$;

-- ---------------------------------------------------------------- authorization helper
CREATE FUNCTION fn_punya_peran(p_pengguna uuid, p_peran text) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
  SELECT EXISTS (
    SELECT 1 FROM admin_user u JOIN admin_user_peran p ON p.pengguna_id = u.pengguna_id
     WHERE u.pengguna_id = p_pengguna AND u.aktif AND p.peran = p_peran AND p.dicabut_at IS NULL
       AND (p.berlaku_sampai IS NULL OR p.berlaku_sampai > now()));
$$;

-- ---------------------------------------------------------------- credential status (EN-03)
CREATE FUNCTION fn_set_status_kredensial(p_credential_id uuid, p_status_baru text, p_pengguna_id uuid, p_alasan text)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_lama client_kredensial%ROWTYPE;
BEGIN
  IF NOT (fn_punya_peran(p_pengguna_id, 'platform_admin') OR fn_punya_peran(p_pengguna_id, 'client_owner')) THEN
    RAISE EXCEPTION 'tidak berwenang' USING ERRCODE = 'insufficient_privilege';
  END IF;
  SELECT * INTO v_lama FROM client_kredensial WHERE credential_id = p_credential_id FOR UPDATE;
  IF NOT FOUND THEN
    RAISE EXCEPTION 'kredensial tidak ada' USING ERRCODE = 'no_data_found';
  END IF;
  IF p_status_baru NOT IN ('aktif', 'dicabut') THEN
    RAISE EXCEPTION 'status tidak sah: %', p_status_baru USING ERRCODE = 'check_violation';
  END IF;
  IF v_lama.status = 'dicabut' THEN
    -- the only legal transition is aktif -> dicabut; the Admin API maps this to 409
    RAISE EXCEPTION 'kredensial_dicabut_terminal' USING ERRCODE = 'check_violation';
  END IF;
  IF p_status_baru = 'aktif' THEN
    RETURN;   -- already aktif, nothing to do
  END IF;
  UPDATE client_kredensial SET status = 'dicabut', dicabut_at = now(), dicabut_oleh = p_pengguna_id WHERE credential_id = p_credential_id;
END $$;

-- ---------------------------------------------------------------- K-20 deactivation with mass cancel (EN-02, EN-08, C-01, C-02, T-02a/b)
-- One transaction: flip the flag, close every waiting request, write history rows (shared terjadi_at = now()),
-- then one audit_konfigurasi row carrying jumlah_cancelled / jumlah_failed. In-flight requests are left alone.
CREATE FUNCTION fn_nonaktifkan_entitas(p_jenis text, p_id uuid, p_pengguna_id uuid, p_alasan text)
RETURNS TABLE (jumlah_cancelled integer, jumlah_failed integer)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_reason text;
  v_kolom text;
  v_lama boolean;
  v_c integer := 0;
  v_f integer := 0;
  r record;
  v_seq integer;
  v_webhook text;
BEGIN
  IF NOT fn_punya_peran(p_pengguna_id, 'platform_admin') THEN
    IF NOT (p_jenis = 'kredensial' AND fn_punya_peran(p_pengguna_id, 'client_owner')
            AND EXISTS (SELECT 1 FROM client_kredensial k JOIN admin_user u ON u.client_id = k.client_id
                        WHERE k.credential_id = p_id AND u.pengguna_id = p_pengguna_id)) THEN
      RAISE EXCEPTION 'hanya platform admin atau pemilik client' USING ERRCODE = 'insufficient_privilege';
    END IF;
  END IF;
  IF p_alasan IS NULL OR length(btrim(p_alasan)) = 0 THEN
    RAISE EXCEPTION 'alasan wajib' USING ERRCODE = 'check_violation';
  END IF;

  CASE p_jenis
    WHEN 'client' THEN
      SELECT aktif INTO v_lama FROM client WHERE client_id = p_id FOR UPDATE;
      UPDATE client SET aktif = false, diubah_at = now() WHERE client_id = p_id;
      v_reason := 'client_nonaktif'; v_kolom := 'client_id';
    WHEN 'kredensial' THEN
      PERFORM fn_set_status_kredensial(p_id, 'dicabut', p_pengguna_id, p_alasan);
      v_lama := true; v_reason := 'kredensial_nonaktif'; v_kolom := 'credential_id';
    WHEN 'downstream_endpoint' THEN
      SELECT aktif INTO v_lama FROM downstream_endpoint WHERE downstream_endpoint_id = p_id FOR UPDATE;
      UPDATE downstream_endpoint SET aktif = false, diubah_at = now() WHERE downstream_endpoint_id = p_id;
      v_reason := 'endpoint_nonaktif'; v_kolom := 'downstream_endpoint_id';
    WHEN 'downstream_pool' THEN
      SELECT aktif INTO v_lama FROM downstream_pool WHERE downstream_pool_id = p_id FOR UPDATE;
      UPDATE downstream_pool SET aktif = false, diubah_at = now() WHERE downstream_pool_id = p_id;
      v_reason := 'pool_nonaktif'; v_kolom := 'downstream_pool_id';
    WHEN 'upstream_pool' THEN
      SELECT aktif INTO v_lama FROM upstream_pool WHERE upstream_pool_id = p_id FOR UPDATE;
      UPDATE upstream_pool SET aktif = false, diubah_at = now() WHERE upstream_pool_id = p_id;
      v_reason := 'pool_nonaktif'; v_kolom := 'upstream_pool_id';   -- EN-04: matches only requests already dispatched once
    WHEN 'provider' THEN
      SELECT aktif INTO v_lama FROM provider WHERE provider_id = p_id FOR UPDATE;
      UPDATE provider SET aktif = false, diubah_at = now() WHERE provider_id = p_id;
      v_reason := 'provider_nonaktif'; v_kolom := 'provider_id';    -- EN-09: pool flags untouched
    ELSE
      RAISE EXCEPTION 'jenis entitas tidak dikenal: %', p_jenis USING ERRCODE = 'check_violation';
  END CASE;
  IF NOT FOUND AND p_jenis <> 'kredensial' THEN
    RAISE EXCEPTION 'entitas tidak ada' USING ERRCODE = 'no_data_found';
  END IF;

  FOR r IN EXECUTE format(
      'SELECT request_id, attempt_count, status, downstream_endpoint_id, client_id FROM request
        WHERE status IN (''queued'', ''dispatched'') AND %I = $1 ORDER BY request_id FOR UPDATE', v_kolom) USING p_id
  LOOP
    SELECT CASE WHEN w.aktif THEN 'menunggu' ELSE 'tidak_dikirim' END INTO v_webhook
      FROM (SELECT coalesce((SELECT aktif FROM client_webhook WHERE client_id = r.client_id), false) AS aktif) w;
    SELECT coalesce(max(seq), 0) + 1 INTO v_seq FROM request_status_history WHERE request_id = r.request_id;
    IF r.attempt_count = 0 THEN
      UPDATE request SET status = 'cancelled', cancel_reason = v_reason, cancelled_by = p_pengguna_id::text,
             completed_at = now(), webhook_status = v_webhook WHERE request_id = r.request_id;
      PERFORM fn_purge_p02(r.request_id);
      INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor, objek_pemicu_id)
        VALUES (r.request_id, v_seq, r.status, 'cancelled', v_reason, p_pengguna_id::text, p_id);
      v_c := v_c + 1;
    ELSE
      UPDATE request SET status = 'failed', failure_reason = 'objek_nonaktif', completed_at = now(),
             final_kode_transaksi = (SELECT kode_transaksi FROM transaction_log WHERE request_id = r.request_id ORDER BY attempt_no DESC LIMIT 1),
             webhook_status = v_webhook WHERE request_id = r.request_id;
      INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor, objek_pemicu_id)
        VALUES (r.request_id, v_seq, r.status, 'failed', 'objek_nonaktif', p_pengguna_id::text, p_id);
      v_f := v_f + 1;
    END IF;
  END LOOP;

  INSERT INTO audit_konfigurasi (audit_konfigurasi_id, pengguna_id, entitas_jenis, entitas_id, aksi, nilai_lama, nilai_baru, alasan, jumlah_cancelled, jumlah_failed)
  VALUES (gen_random_uuid(), p_pengguna_id, p_jenis, p_id, CASE WHEN p_jenis = 'kredensial' THEN 'cabut' ELSE 'nonaktifkan' END,
          jsonb_build_object('aktif', v_lama), jsonb_build_object('aktif', false), p_alasan, v_c, v_f);
  RETURN QUERY SELECT v_c, v_f;
END $$;

-- EN-07: reactivation only affects later requests. Credentials cannot be reactivated (EN-03).
CREATE FUNCTION fn_aktifkan_entitas(p_jenis text, p_id uuid, p_pengguna_id uuid, p_alasan text) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
  IF NOT fn_punya_peran(p_pengguna_id, 'platform_admin') THEN
    RAISE EXCEPTION 'hanya platform admin' USING ERRCODE = 'insufficient_privilege';
  END IF;
  IF p_alasan IS NULL OR length(btrim(p_alasan)) = 0 THEN
    RAISE EXCEPTION 'alasan wajib' USING ERRCODE = 'check_violation';
  END IF;
  CASE p_jenis
    WHEN 'client' THEN UPDATE client SET aktif = true, diubah_at = now() WHERE client_id = p_id;
    WHEN 'downstream_endpoint' THEN UPDATE downstream_endpoint SET aktif = true, diubah_at = now() WHERE downstream_endpoint_id = p_id;
    WHEN 'downstream_pool' THEN UPDATE downstream_pool SET aktif = true, diubah_at = now() WHERE downstream_pool_id = p_id;
    WHEN 'upstream_pool' THEN UPDATE upstream_pool SET aktif = true, diubah_at = now() WHERE upstream_pool_id = p_id;
    WHEN 'provider' THEN UPDATE provider SET aktif = true, diubah_at = now() WHERE provider_id = p_id;
    WHEN 'kredensial' THEN RAISE EXCEPTION 'kredensial_dicabut_terminal' USING ERRCODE = 'check_violation';
    ELSE RAISE EXCEPTION 'jenis entitas tidak dikenal: %', p_jenis USING ERRCODE = 'check_violation';
  END CASE;
  IF NOT FOUND THEN
    RAISE EXCEPTION 'entitas tidak ada' USING ERRCODE = 'no_data_found';
  END IF;
  INSERT INTO audit_konfigurasi (audit_konfigurasi_id, pengguna_id, entitas_jenis, entitas_id, aksi, nilai_lama, nilai_baru, alasan)
  VALUES (gen_random_uuid(), p_pengguna_id, p_jenis, p_id, 'aktifkan', jsonb_build_object('aktif', false), jsonb_build_object('aktif', true), p_alasan);
END $$;

-- EN-08: preview numbers only, nothing stored, no ids.
CREATE FUNCTION fn_pratinjau_nonaktif(p_jenis text, p_id uuid) RETURNS TABLE (akan_cancelled integer, akan_failed integer, dihitung_at timestamptz)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_kolom text;
BEGIN
  v_kolom := CASE p_jenis WHEN 'client' THEN 'client_id' WHEN 'kredensial' THEN 'credential_id'
    WHEN 'downstream_endpoint' THEN 'downstream_endpoint_id' WHEN 'downstream_pool' THEN 'downstream_pool_id'
    WHEN 'upstream_pool' THEN 'upstream_pool_id' WHEN 'provider' THEN 'provider_id' END;
  IF v_kolom IS NULL THEN
    RAISE EXCEPTION 'jenis entitas tidak dikenal: %', p_jenis USING ERRCODE = 'check_violation';
  END IF;
  RETURN QUERY EXECUTE format(
    'SELECT count(*) FILTER (WHERE attempt_count = 0)::int, count(*) FILTER (WHERE attempt_count > 0)::int, now()
       FROM request WHERE status IN (''queued'', ''dispatched'') AND %I = $1', v_kolom) USING p_id;
END $$;

-- ---------------------------------------------------------------- masked views (A-01, A-06, K-29)
-- Owner-run, security_barrier. NIK is only ever the last 4 digits; no payload and no metadata columns.
CREATE VIEW v_transaction_log_tersamar WITH (security_barrier = true) AS
  SELECT l.kode_transaksi, l.request_id, l.attempt_no, l.provider_id, l.upstream_pool_id, l.client_id, l.downstream_endpoint_id,
         l.provider_config_version, l.endpoint_config_version,
         CASE WHEN l.nik IS NULL THEN NULL ELSE repeat('*', 12) || right(l.nik, 4) END AS nik_tersamar,
         l.requested_at, l.responded_at, l.duration_ms, l.status_akses, l.provider_http_status, l.provider_response_code,
         l.error_class, l.log_mode, l.worker_id
    FROM transaction_log l;

CREATE VIEW v_request_tersamar WITH (security_barrier = true) AS
  SELECT r.request_id, r.client_id, r.credential_id, r.downstream_endpoint_id, r.downstream_pool_id, r.provider_id, r.upstream_pool_id,
         r.mode, r.status, r.reject_reason, r.reject_http_status, r.cancel_reason, r.cancelled_by, r.failure_reason,
         r.referensi_client, r.received_at, r.ttl_expires_at, r.dispatched_at, r.first_attempt_at, r.completed_at, r.attempt_count,
         r.final_kode_transaksi, r.sumber_hasil, r.webhook_status, r.webhook_attempt_count, r.webhook_last_at,
         r.webhook_last_http_status, r.webhook_last_error,
         CASE WHEN l.nik IS NULL THEN NULL ELSE repeat('*', 12) || right(l.nik, 4) END AS nik_tersamar
    FROM request r LEFT JOIN transaction_log l ON l.kode_transaksi = r.final_kode_transaksi;

-- Client owner: rows of the caller's own client only, resolved from admin_user (Row filter cannot be a column grant).
CREATE FUNCTION fn_request_milik_client(p_pengguna_id uuid) RETURNS SETOF v_request_tersamar
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
  SELECT v.* FROM v_request_tersamar v
   WHERE fn_punya_peran(p_pengguna_id, 'client_owner')
     AND v.client_id = (SELECT client_id FROM admin_user WHERE pengguna_id = p_pengguna_id);
$$;

-- ---------------------------------------------------------------- auditor access (A-01, A-04, E-01..E-06)
CREATE FUNCTION fn_catat_ditolak(p_pengguna uuid, p_peran text, p_filter jsonb, p_ip text, p_sesi text) RETURNS void
LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
  INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, filter_kueri, jumlah_record, hasil, sumber_ip, sesi_id)
  VALUES (gen_random_uuid(), p_pengguna, p_peran, 'akses_ditolak', 'tidak_ada', p_filter, 0, 'ditolak', p_ip, p_sesi);
$$;

-- Full NIK + payload of one attempt, auditor only, reason required. Not an auditor -> akses_ditolak row, no data.
CREATE FUNCTION fn_auditor_lihat_transaksi(p_pengguna_id uuid, p_alasan text, p_kode_transaksi uuid, p_ip text, p_sesi text)
RETURNS SETOF transaction_log
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_row transaction_log%ROWTYPE;
BEGIN
  IF NOT fn_punya_peran(p_pengguna_id, 'auditor') THEN
    PERFORM fn_catat_ditolak(p_pengguna_id, 'tidak_berwenang', jsonb_build_object('kode_transaksi', p_kode_transaksi), p_ip, p_sesi);
    RETURN;
  END IF;
  IF p_alasan IS NULL OR length(btrim(p_alasan)) = 0 THEN
    RAISE EXCEPTION 'alasan wajib' USING ERRCODE = 'check_violation';
  END IF;
  SELECT * INTO v_row FROM transaction_log WHERE kode_transaksi = p_kode_transaksi;
  INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, filter_kueri, target_request_id, target_kode_transaksi,
                           nik_tersamar, jumlah_record, alasan, hasil, sumber_ip, sesi_id)
  VALUES (gen_random_uuid(), p_pengguna_id, 'auditor', 'lihat_payload', 'utuh', jsonb_build_object('kode_transaksi', p_kode_transaksi),
          v_row.request_id, p_kode_transaksi, CASE WHEN v_row.nik IS NULL THEN NULL ELSE repeat('*', 12) || right(v_row.nik, 4) END,
          CASE WHEN FOUND THEN 1 ELSE 0 END, p_alasan, 'diizinkan', p_ip, p_sesi);
  IF FOUND THEN RETURN NEXT v_row; END IF;
END $$;

-- US-14: search by full NIK plus a time range (auditor only). The caller supplies HMAC-SHA256(NIK) for the audit row.
CREATE FUNCTION fn_auditor_cari_nik(p_pengguna_id uuid, p_nik text, p_nik_hmac text, p_dari timestamptz, p_sampai timestamptz,
                                    p_alasan text, p_ip text, p_sesi text)
RETURNS TABLE (kode_transaksi uuid, request_id uuid, client_id uuid, requested_at timestamptz, status_akses text)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_n integer;
BEGIN
  IF NOT fn_punya_peran(p_pengguna_id, 'auditor') THEN
    PERFORM fn_catat_ditolak(p_pengguna_id, 'tidak_berwenang', jsonb_build_object('rentang', jsonb_build_array(p_dari, p_sampai)), p_ip, p_sesi);
    RETURN;
  END IF;
  IF p_alasan IS NULL OR length(btrim(p_alasan)) = 0 THEN
    RAISE EXCEPTION 'alasan wajib' USING ERRCODE = 'check_violation';
  END IF;
  SELECT count(*) INTO v_n FROM transaction_log l
   WHERE l.nik = p_nik AND l.requested_at >= p_dari AND l.requested_at < p_sampai;
  INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, filter_kueri, nik_hmac, nik_tersamar, jumlah_record, alasan, hasil, sumber_ip, sesi_id)
  VALUES (gen_random_uuid(), p_pengguna_id, 'auditor', 'cari_nik', 'utuh',
          jsonb_build_object('nik_hmac', p_nik_hmac, 'nik_tersamar', repeat('*', 12) || right(p_nik, 4), 'dari', p_dari, 'sampai', p_sampai),
          p_nik_hmac, repeat('*', 12) || right(p_nik, 4), v_n, p_alasan, 'diizinkan', p_ip, p_sesi);
  RETURN QUERY SELECT l.kode_transaksi, l.request_id, l.client_id, l.requested_at, l.status_akses FROM transaction_log l
   WHERE l.nik = p_nik AND l.requested_at >= p_dari AND l.requested_at < p_sampai ORDER BY l.requested_at, l.kode_transaksi;
END $$;

-- E-01..E-05: file bytes, checksum, and the audit row come from one function in one transaction.
-- Over the row limit (E-03): no file, one audit row with hasil = ditolak, ekspor_checksum NULL, jumlah_record = matching rows.
-- Not an auditor: akses_ditolak row (8.1), no file.
CREATE FUNCTION fn_ekspor_log_auditor(p_pengguna_id uuid, p_audit_id uuid, p_alasan text, p_nik_hmac text, p_nik_tersamar text,
                                      p_dari timestamptz, p_sampai timestamptz, p_client_id uuid, p_ip text, p_sesi text,
                                      p_nik text DEFAULT NULL)
RETURNS TABLE (audit_id uuid, berkas bytea, ekspor_checksum text, jumlah_record integer, hasil text)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_batas integer;
  v_n integer;
  v_csv text;
  v_bytes bytea;
  v_sum text;
  v_filter jsonb;
BEGIN
  v_filter := jsonb_strip_nulls(jsonb_build_object('nik_hmac', p_nik_hmac, 'nik_tersamar', p_nik_tersamar,
                                                    'dari', p_dari, 'sampai', p_sampai, 'client_id', p_client_id));
  IF NOT fn_punya_peran(p_pengguna_id, 'auditor') THEN
    PERFORM fn_catat_ditolak(p_pengguna_id, 'tidak_berwenang', v_filter, p_ip, p_sesi);
    RETURN QUERY SELECT p_audit_id, NULL::bytea, NULL::text, 0, 'ditolak'::text;
    RETURN;
  END IF;
  IF p_alasan IS NULL OR length(btrim(p_alasan)) = 0 THEN
    RAISE EXCEPTION 'alasan wajib' USING ERRCODE = 'check_violation';
  END IF;
  SELECT (nilai #>> '{}')::integer INTO v_batas FROM konfigurasi_platform WHERE kunci = 'ekspor_batas_baris';
  SELECT count(*) INTO v_n FROM transaction_log l
   WHERE l.requested_at >= p_dari AND l.requested_at < p_sampai
     AND (p_client_id IS NULL OR l.client_id = p_client_id) AND (p_nik IS NULL OR l.nik = p_nik) AND l.nik IS NOT NULL;
  IF v_n > v_batas THEN
    INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, filter_kueri, nik_hmac, nik_tersamar, jumlah_record, alasan, hasil, sumber_ip, sesi_id)
    VALUES (p_audit_id, p_pengguna_id, 'auditor', 'ekspor', 'utuh', v_filter, p_nik_hmac, p_nik_tersamar, v_n, p_alasan, 'ditolak', p_ip, p_sesi);
    RETURN QUERY SELECT p_audit_id, NULL::bytea, NULL::text, v_n, 'ditolak'::text;
    RETURN;
  END IF;
  -- E-04: fixed header, nik as text, +07:00 offsets, ordered by requested_at then kode_transaksi, spreadsheet-injection prefix (E-02).
  SELECT 'nik,requested_at,status_akses,kode_transaksi,queue_id,client_id' || E'\n' || coalesce(string_agg(
           format('%s,%s,%s,%s,%s,%s',
             CASE WHEN l.nik ~ '^[=+\-@\t\r]' THEN '''' || l.nik ELSE l.nik END,
             to_char(l.requested_at AT TIME ZONE 'Asia/Jakarta', 'YYYY-MM-DD"T"HH24:MI:SS.US') || '+07:00',
             coalesce(l.status_akses, ''), l.kode_transaksi, l.request_id, l.client_id),
           E'\n' ORDER BY l.requested_at, l.kode_transaksi), '') || CASE WHEN v_n > 0 THEN E'\n' ELSE '' END
    INTO v_csv
    FROM transaction_log l
   WHERE l.requested_at >= p_dari AND l.requested_at < p_sampai
     AND (p_client_id IS NULL OR l.client_id = p_client_id) AND (p_nik IS NULL OR l.nik = p_nik) AND l.nik IS NOT NULL;
  v_bytes := convert_to(v_csv, 'UTF8');
  v_sum := encode(sha256(v_bytes), 'hex');
  INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, filter_kueri, nik_hmac, nik_tersamar, jumlah_record,
                           ekspor_checksum, alasan, hasil, sumber_ip, sesi_id)
   VALUES (p_audit_id, p_pengguna_id, 'auditor', 'ekspor', 'utuh', v_filter, p_nik_hmac, p_nik_tersamar, v_n,
          v_sum, p_alasan, 'diizinkan', p_ip, p_sesi);
  RETURN QUERY SELECT p_audit_id, v_bytes, v_sum, v_n, 'diizinkan'::text;
END $$;

-- E-06: reading audit_akses is itself audited; the response has no nik_hmac.
CREATE FUNCTION fn_lihat_audit(p_pengguna_id uuid, p_dari timestamptz, p_sampai timestamptz, p_batas integer, p_ip text, p_sesi text)
RETURNS TABLE (audit_id uuid, pengguna_id uuid, peran text, aksi text, tampilan_nik text, filter_kueri jsonb, nik_tersamar varchar,
               jumlah_record integer, alasan varchar, hasil text, sumber_ip varchar, terjadi_at timestamptz)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_n integer;
BEGIN
  IF NOT fn_punya_peran(p_pengguna_id, 'auditor') THEN
    PERFORM fn_catat_ditolak(p_pengguna_id, 'tidak_berwenang', jsonb_build_object('dari', p_dari, 'sampai', p_sampai), p_ip, p_sesi);
    RETURN;
  END IF;
  IF p_batas IS NULL OR p_batas < 1 THEN RAISE EXCEPTION 'batas wajib positif' USING ERRCODE = 'check_violation'; END IF;
  SELECT count(*) INTO v_n FROM (SELECT 1 FROM audit_akses a WHERE a.terjadi_at >= p_dari AND a.terjadi_at < p_sampai AND a.terjadi_at < now() LIMIT p_batas) q;
  INSERT INTO audit_akses (audit_id, pengguna_id, peran, aksi, tampilan_nik, filter_kueri, jumlah_record, hasil, sumber_ip, sesi_id)
  VALUES (gen_random_uuid(), p_pengguna_id, 'auditor', 'lihat_audit', 'tidak_ada',
          jsonb_build_object('dari', p_dari, 'sampai', p_sampai, 'batas', p_batas), v_n, 'diizinkan', p_ip, p_sesi);
  RETURN QUERY SELECT a.audit_id, a.pengguna_id, a.peran, a.aksi, a.tampilan_nik, a.filter_kueri - 'nik_hmac', a.nik_tersamar, a.jumlah_record, a.alasan,
                      a.hasil, a.sumber_ip, a.terjadi_at
    FROM audit_akses a WHERE a.terjadi_at >= p_dari AND a.terjadi_at < p_sampai AND a.terjadi_at < now()
    ORDER BY a.terjadi_at, a.audit_id LIMIT p_batas;
END $$;

-- R-02: the reconciler closes a pending attempt as tidak_diketahui, only while it is still NULL and old enough.
CREATE FUNCTION fn_tutup_log_tidak_diketahui(p_kode_transaksi uuid, p_ambang_detik integer) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
  v_n integer;
BEGIN
  IF p_ambang_detik < 1 THEN
    RAISE EXCEPTION 'ambang detik harus positif' USING ERRCODE = 'check_violation';
  END IF;
  UPDATE transaction_log SET status_akses = 'tidak_diketahui'
   WHERE kode_transaksi = p_kode_transaksi AND status_akses IS NULL
     AND requested_at <= now() - make_interval(secs => p_ambang_detik);
  GET DIAGNOSTICS v_n = ROW_COUNT;
  RETURN v_n = 1;
END $$;

-- Webhook delivery log of the caller's own client (client owner, contract 8.1).
CREATE FUNCTION fn_webhook_log_milik_client(p_pengguna_id uuid) RETURNS SETOF webhook_log
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
  SELECT w.* FROM webhook_log w
   WHERE fn_punya_peran(p_pengguna_id, 'client_owner')
     AND w.client_id = (SELECT client_id FROM admin_user WHERE pengguna_id = p_pengguna_id);
$$;

-- pr_webhook reads terminal async requests through this view only (column grants cannot restrict rows).
CREATE VIEW v_request_webhook WITH (security_barrier = true) AS
  SELECT r.request_id, r.client_id, r.status, r.attempt_count, r.cancel_reason, r.failure_reason, r.completed_at,
         r.webhook_status, r.webhook_attempt_count, r.output_payload, r.payload_purged_at
    FROM request r
   WHERE r.mode = 'async' AND r.status IN ('done', 'failed', 'expired', 'cancelled');

-- ---------------------------------------------------------------- lock down function execution
DO $$
DECLARE
  f record;
BEGIN
  FOR f IN SELECT p.oid::regprocedure AS sig FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
            WHERE n.nspname = 'public' AND p.prokind = 'f'
              AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.objid = p.oid AND d.deptype = 'e')
  LOOP
    EXECUTE format('REVOKE ALL ON FUNCTION %s FROM PUBLIC', f.sig);
  END LOOP;
END $$;
