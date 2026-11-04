-- Uji negatif F1 (SHAA-335). Setiap blok harus ditolak; penolakan yang tidak terjadi menggagalkan run.
\set ON_ERROR_STOP on
DO $neg$ DECLARE v_ditolak boolean := false; BEGIN
  BEGIN
    INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id,
            downstream_endpoint_id, provider_config_version, endpoint_config_version, nik, requested_at, responded_at,
            duration_ms, status_akses, provider_http_status, log_mode, worker_id, response_hash)
            SELECT gen_random_uuid(), request_id, 2, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
            1, 1, '123456789012345', requested_at, responded_at, 100, status_akses, 200, log_mode, worker_id, response_hash
            FROM transaction_log ORDER BY kode_transaksi LIMIT 1;
  EXCEPTION WHEN others THEN v_ditolak := true; RAISE NOTICE 'ditolak (%): %', 'NEG-01', SQLERRM;
  END;
  IF NOT v_ditolak THEN RAISE EXCEPTION 'NEG-01 NIK 15 digit ditolak (CHECK nik ~ ^[0-9]{16}$) tidak ditolak'; END IF;
END $neg$;
DO $neg$ DECLARE v_ditolak boolean := false; BEGIN
  BEGIN
    INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id,
            downstream_endpoint_id, provider_config_version, endpoint_config_version, nik, requested_at, responded_at,
            duration_ms, status_akses, provider_http_status, log_mode, worker_id, response_hash)
            SELECT gen_random_uuid(), request_id, 3, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
            1, 1, NULL, requested_at, responded_at, 100, status_akses, 200, log_mode, worker_id, response_hash
            FROM transaction_log ORDER BY kode_transaksi LIMIT 1;
  EXCEPTION WHEN others THEN v_ditolak := true; RAISE NOTICE 'ditolak (%): %', 'NEG-02', SQLERRM;
  END;
  IF NOT v_ditolak THEN RAISE EXCEPTION 'NEG-02 nik NULL ditolak untuk provider wajib_log_nik (C-01) tidak ditolak'; END IF;
END $neg$;
DO $neg$ DECLARE v_ditolak boolean := false; BEGIN
  BEGIN
    INSERT INTO data_balikan_item (batch_id, request_id, kode_transaksi, nik, field_input, field_metadata, requested_at)
            SELECT batch_id, request_id, kode_transaksi, '12345678901234X', field_input, field_metadata, requested_at
            FROM data_balikan_item LIMIT 1;
  EXCEPTION WHEN others THEN v_ditolak := true; RAISE NOTICE 'ditolak (%): %', 'NEG-03', SQLERRM;
  END;
  IF NOT v_ditolak THEN RAISE EXCEPTION 'NEG-03 data_balikan_item NIK 15 digit ditolak tidak ditolak'; END IF;
END $neg$;
DO $neg$ DECLARE v_ditolak boolean := false; BEGIN
  BEGIN
    INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id, alokasi_jenis, alokasi_nilai, sifat, boleh_pinjam)
            VALUES ('976d59b6-5e1c-55c4-b67a-847113e0f429', '32191cc0-bd57-53b2-9f21-891d6b916dfd', 'exact', 70.00, 'reserved', false),
            ('8e290df3-0160-5acc-b7ea-ae385715907b', '32191cc0-bd57-53b2-9f21-891d6b916dfd', 'exact', 50.00, 'reserved', false);
            SET CONSTRAINTS ALL IMMEDIATE;
  EXCEPTION WHEN others THEN v_ditolak := true; RAISE NOTICE 'ditolak (%): %', 'NEG-04', SQLERRM;
  END;
  IF NOT v_ditolak THEN RAISE EXCEPTION 'NEG-04 alokasi reserved melebihi kuota upstream (U-64a: 70 + 50 > 100, DQ-41) tidak ditolak'; END IF;
END $neg$;
DO $neg$ DECLARE v_ditolak boolean := false; BEGIN
  BEGIN
    UPDATE request SET status = 'queued' WHERE request_id = (SELECT request_id FROM request WHERE status = 'done' LIMIT 1);
  EXCEPTION WHEN others THEN v_ditolak := true; RAISE NOTICE 'ditolak (%): %', 'NEG-05', SQLERRM;
  END;
  IF NOT v_ditolak THEN RAISE EXCEPTION 'NEG-05 transisi status request tidak sah (done -> queued, tg_request_transisi) tidak ditolak'; END IF;
END $neg$;
DO $neg$ DECLARE v_ditolak boolean := false; BEGIN
  BEGIN
    INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, downstream_pool_id, provider_id,
            provider_config_version, endpoint_config_version, mode, status, attempt_count, metadata, received_at,
            input_payload, prioritas, prioritas_efektif, ttl_expires_at)
            SELECT gen_random_uuid(), client_id, credential_id, downstream_endpoint_id, downstream_pool_id, provider_id,
            1, 1, 'async', 'queued', 0, '{}', received_at, '{"sintetis": true}', 1, 1, '2026-11-05T00:00:00+00:00'
            FROM request WHERE mode = 'sync' LIMIT 1;
            SET CONSTRAINTS ALL IMMEDIATE;
  EXCEPTION WHEN others THEN v_ditolak := true; RAISE NOTICE 'ditolak (%): %', 'NEG-06', SQLERRM;
  END;
  IF NOT v_ditolak THEN RAISE EXCEPTION 'NEG-06 request tanpa baris request_status_history ditolak saat COMMIT (T-04, trigger tertunda) tidak ditolak'; END IF;
END $neg$;
DO $neg$ DECLARE v_ditolak boolean := false; BEGIN
  BEGIN
    UPDATE pengguna_mfa SET dikonfirmasi_at = NULL
            WHERE pengguna_id = (SELECT pengguna_id FROM pengguna_mfa WHERE status = 'aktif' LIMIT 1);
            SET CONSTRAINTS ALL IMMEDIATE;
  EXCEPTION WHEN others THEN v_ditolak := true; RAISE NOTICE 'ditolak (%): %', 'NEG-07', SQLERRM;
  END;
  IF NOT v_ditolak THEN RAISE EXCEPTION 'NEG-07 pengguna_mfa aktif tanpa dikonfirmasi_at ditolak (chk_mfa_konfirmasi) tidak ditolak'; END IF;
END $neg$;
DO $neg$ DECLARE v_ditolak boolean := false; BEGIN
  BEGIN
    UPDATE pengguna_mfa SET langkah_terakhir = 5 WHERE pengguna_id = 'e0e43323-37f2-5b8b-9d45-97d9ba27ff17';
            UPDATE pengguna_mfa SET langkah_terakhir = 3 WHERE pengguna_id = 'e0e43323-37f2-5b8b-9d45-97d9ba27ff17';
  EXCEPTION WHEN others THEN v_ditolak := true; RAISE NOTICE 'ditolak (%): %', 'NEG-08', SQLERRM;
  END;
  IF NOT v_ditolak THEN RAISE EXCEPTION 'NEG-08 langkah_terakhir MFA turun ditolak (MF-03, tg_mfa_langkah_naik) tidak ditolak'; END IF;
END $neg$;
DO $neg$ DECLARE v_ditolak boolean := false; BEGIN
  BEGIN
    UPDATE kode_pemulihan_mfa SET dipakai_at = now()
            WHERE kode_id = (SELECT kode_id FROM kode_pemulihan_mfa WHERE dipakai_at IS NULL LIMIT 1);
            UPDATE kode_pemulihan_mfa SET dipakai_at = now()
            WHERE kode_id = (SELECT kode_id FROM kode_pemulihan_mfa WHERE dipakai_at IS NOT NULL LIMIT 1);
  EXCEPTION WHEN others THEN v_ditolak := true; RAISE NOTICE 'ditolak (%): %', 'NEG-09', SQLERRM;
  END;
  IF NOT v_ditolak THEN RAISE EXCEPTION 'NEG-09 kode pemulihan yang sudah dipakai tidak bisa dipakai ulang (kode_pemulihan_mfa_sekali) tidak ditolak'; END IF;
END $neg$;
DO $neg$ DECLARE v_ditolak boolean := false; BEGIN
  BEGIN
    INSERT INTO arsip_restore_permintaan (restore_id, diminta_oleh, audit_id, provider_id, rentang_mulai,
            rentang_selesai, nik_tersamar, alasan, status, diminta_at, mulai_at, siap_at, kedaluwarsa_at, jumlah_baris_pulih)
            SELECT gen_random_uuid(), diminta_oleh, gen_random_uuid(), provider_id, rentang_mulai, rentang_selesai,
            nik_tersamar, alasan, status, diminta_at, mulai_at, siap_at, kedaluwarsa_at, jumlah_baris_pulih
            FROM arsip_restore_permintaan LIMIT 1;
            SET CONSTRAINTS ALL IMMEDIATE;
  EXCEPTION WHEN others THEN v_ditolak := true; RAISE NOTICE 'ditolak (%): %', 'NEG-10', SQLERRM;
  END;
  IF NOT v_ditolak THEN RAISE EXCEPTION 'NEG-10 permintaan restore tanpa audit minta_restore yang cocok ditolak (DQ-49 a) tidak ditolak'; END IF;
END $neg$;
