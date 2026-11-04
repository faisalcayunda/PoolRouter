#!/usr/bin/env python3
"""Loader fixture F1 (SHAA-335): menghasilkan SQL pemuatan dari berkas di out/.

Dipanggil oleh load_f1.sh. Hanya stdlib Python 3, dijalankan dengan `python3 -I`.
Keluaran deterministik: UUID dari uuid5 dengan namespace tetap, waktu dasar tetap.

Pemetaan dan pemotongan dilakukan di sini, bukan di out/ (keputusan Ridley, SHAA-335):
- Berkas yang TIDAK dimuat (bukan data tabel, atau tabelnya tidak ada di skema F1):
    webhook_jadwal.csv, webhook_jadwal_cepat.csv, webhook_klasifikasi.csv  (tidak ada tabel jadwal retry;
                                                                            dipakai uji unit/property)
    sentinel_nik.csv, expected.json, manifest.json                         (bukan data tabel)
    arsip_log.csv                                                          (tidak ada tabel arsip_log di main;
                                                                            baris request-nya tetap dimuat)
    webhook_klien.csv                                                      (tidak ada tabel delivery; uji negatif
                                                                            isolasi memakai client fixture)
    mfa_totp_vektor.csv                                                    (vektor publik RFC 6238, dipakai U-79
                                                                            sebagai uji unit; bukan data tabel)
    webhook_log                                                            (tidak dimuat; hanya bila uji negatif
                                                                            membutuhkannya)
- Baris yang melanggar constraint dengan sengaja (U-64a Σ > Q, U-65 boleh_pinjam = false tanpa alokasi)
  TIDAK dimuat di jalur positif. Dibuat sebagai uji negatif di neg_f1.sql.

Keluaran: <outdir>/load_f1.sql (pemuatan positif, satu transaksi) dan <outdir>/neg_f1.sql (uji negatif).
"""

import csv
import datetime
import hashlib
import json
import os
import subprocess
import sys
import uuid

HERE = os.path.dirname(os.path.abspath(__file__))
MFA_HELPER = os.path.join(HERE, "mfa_envelope.mjs")
OUT_FIXTURE = os.path.join(HERE, "out")

NS = uuid.UUID("6f1d2c3a-0b5e-4e7a-9c2d-335000000001")
T0 = "2026-11-04T00:00:00+00:00"          # waktu dasar tetap
PROVIDER_NAMA = "dukcapil-sintetis-f1"
CLIENT_SEBAGAI_NAMA = {"client-a": "client-a", "client-b": "client-b"}


def uid(kind, key):
    return str(uuid.uuid5(NS, kind + ":" + key))


def q(v):
    """Literal SQL aman untuk nilai string/None/bool/angka."""
    if v is None or v == "":
        return "NULL"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return str(v)
    return "'" + str(v).replace("'", "''") + "'"


def read(name):
    with open(os.path.join(OUT_FIXTURE, name), newline="", encoding="utf-8") as fh:
        return list(csv.DictReader(fh))


def insert(table, cols, vals):
    return "INSERT INTO %s (%s) VALUES (%s);" % (table, ", ".join(cols), ", ".join(q(v) for v in vals))


def sha256_hex(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def mfa_env():
    """Seed dan KEK uji dari environment (dibuat per run oleh run_f1.sh). Kosong = berhenti, exit non-nol."""
    seed = os.environ.get("TOTP_TEST_SEED", "")
    kek_hex = os.environ.get("F1_KEK_HEX", "")
    if not seed or not kek_hex:
        sys.stderr.write("TOTP_TEST_SEED atau F1_KEK_HEX kosong: loader berhenti (fail-closed). "
                         "Jalankan lewat run_f1.sh.\n")
        sys.exit(3)
    return seed, kek_hex


def encrypt_mfa(seed, kek_hex, pengguna_ids):
    """Enkripsi envelope lewat mfa_envelope.mjs. Input lewat stdin, bukan argumen proses."""
    payload = json.dumps({"seed": seed, "kek_hex": kek_hex, "kek_id": "kek-uji-f1", "pengguna_ids": pengguna_ids})
    res = subprocess.run(["node", MFA_HELPER], input=payload, capture_output=True, text=True, check=True)
    return json.loads(res.stdout)


def build():
    stmts = []
    add = stmts.append

    # ---------- parent seed (sintetis, UUID tetap) ----------
    # akun uji (pengguna dari fixture mfa_pengguna.csv): dibutuhkan sebagai pembuat (FK NOT NULL)
    pengguna = read("mfa_pengguna.csv")
    admin_seed = pengguna[0]["pengguna_id"]
    for n, r in enumerate(pengguna, 1):
        add(insert("admin_user", ["pengguna_id", "username", "password_hash", "aktif"],
                   [r["pengguna_id"], "uji-%s-%02d" % (r["skenario"].replace("_", "-"), n),
                    "$argon2id$SINTETIS-PLACEHOLDER", True]))

    client_ids = {}
    for nama in ("client-a", "client-b"):
        client_ids[nama] = uid("client", nama)
    # client yang dipakai request_aging memakai UUID dari fixture
    aging_rows = read("request_aging.csv")
    client_ids["client-req"] = aging_rows[0]["client_id"]

    for nama, cid in (("client-a", client_ids["client-a"]), ("client-b", client_ids["client-b"]),
                      ("client-req", client_ids["client-req"])):
        add(insert("client", ["client_id", "nama", "aktif"], [cid, "sintetis-" + nama, True]))

    cred_ids = {}
    for nama, cid in client_ids.items():
        cred = uid("cred", nama)
        cred_ids[nama] = cred
        add(insert("client_kredensial", ["credential_id", "client_id", "jenis", "kunci_hash", "status", "dibuat_oleh", "dibuat_at"],
                   [cred, cid, "api_key", "\\x" + sha256_hex("sintetis-" + nama), "aktif", admin_seed, T0]))

    prov = uid("provider", PROVIDER_NAMA)
    add(insert("provider", ["provider_id", "nama", "aktif", "wajib_log_nik", "data_balikan_aktif"],
               [prov, PROVIDER_NAMA, True, True, True]))
    add(insert("provider_config_versi",
               ["provider_id", "versi", "config_schema_version", "log_mode", "dokumen", "dibuat_oleh"],
               [prov, 1, 1, "metadata_only", "{}", admin_seed]))

    # upstream pool dari fixture alokasi_rute
    alokasi = read("alokasi_rute.csv")
    upstream_ids = sorted({r["upstream_pool_id"] for r in alokasi})
    for i, up in enumerate(upstream_ids):
        add(insert("upstream_pool", ["upstream_pool_id", "provider_id", "nama", "aktif", "kuota_harian"],
                   [up, prov, "upstream-sintetis-%02d" % (i + 1), True,
                    # Q dari fixture: skala 1000 untuk a4d6e226 (T-05B-SKALA), 100 untuk sisanya (U-61, U-64, U-65)
                    1000 if up == "a4d6e226-32e3-512a-a1e9-0a0909892bdd" else 100]))

    # downstream pool: semua id dari alokasi_rute dan request_aging (kalau ada)
    ds_ids = sorted({r["downstream_pool_id"] for r in alokasi})
    for i, ds in enumerate(ds_ids):
        add(insert("downstream_pool", ["downstream_pool_id", "nama", "aktif", "prioritas", "ttl_antrean_detik"],
                   [ds, "downstream-sintetis-%02d" % (i + 1), True, 1, 86400]))

    # downstream_pool_rute: hanya baris yang valid di jalur positif
    skip_neg = {"U-64a", "U-65"}
    for r in alokasi:
        if r["skenario"] in skip_neg:
            continue
        add(insert("downstream_pool_rute",
                   ["downstream_pool_id", "upstream_pool_id", "alokasi_jenis", "alokasi_nilai", "sifat", "boleh_pinjam"],
                   [r["downstream_pool_id"], r["upstream_pool_id"], r["alokasi_jenis"] or None,
                    r["alokasi_nilai"] or None, r["sifat"] or None, r["boleh_pinjam"] == "true"]))

    # endpoint: satu per downstream pool
    ep_of_pool = {}
    for ds in ds_ids:
        ep = uid("endpoint", ds)
        ep_of_pool[ds] = ep
        add(insert("downstream_endpoint",
                   ["downstream_endpoint_id", "nama", "provider_id", "downstream_pool_id", "aktif", "config_versi_aktif"],
                   [ep, "endpoint-" + ds[:8], prov, ds, True, 1]))
        add(insert("endpoint_config_versi",
                   ["downstream_endpoint_id", "versi", "config_schema_version", "log_mode", "dokumen", "dibuat_oleh"],
                   [ep, 1, 1, "metadata_only", "{}", admin_seed]))

    # ---------- request (satu per request_id unik) ----------
    main_pool = ds_ids[0]
    main_up = upstream_ids[0]

    for r in aging_rows:
        add(insert("request",
                   ["request_id", "client_id", "credential_id", "downstream_endpoint_id", "downstream_pool_id",
                    "provider_id", "provider_config_version", "endpoint_config_version", "mode", "prioritas",
                    "prioritas_efektif", "status", "attempt_count", "metadata", "received_at", "aging_ambang_detik",
                    "prioritas_diubah_at", "ttl_expires_at", "input_payload"],
                   [r["request_id"], r["client_id"], cred_ids["client-req"], ep_of_pool[main_pool], main_pool,
                    prov, 1, 1, r["mode"], int(r["prioritas"]), int(r["prioritas"]), r["status"],
                    int(r["attempt_count"]), "{}", T0, int(r["aging_ambang_detik"]),
                    r["prioritas_diubah_at"] or None, "2026-11-05T00:00:00+00:00",
                    '{"sintetis": true}']))
        add(insert("request_status_history", ["request_id", "seq", "status_dari", "status_ke", "aktor"],
                   [r["request_id"], 1, None, r["status"], "webserver"]))

    # data balikan: satu request sync done per item, dengan transaction_log
    items = read("data_balikan_item.csv")
    batches = read("data_balikan_batch.csv")
    batch_id = batches[0]["batch_id"]
    seen_req = {r["request_id"] for r in aging_rows}
    for it in items:
        rid = it["request_id"]
        kode = uid("kode", rid)
        if rid not in seen_req:
            seen_req.add(rid)
            # sync: masuk sebagai in_flight (trigger tg_request_transisi hanya menerima awal queued/in_flight),
            # lalu done dengan riwayat status yang sah (T-04). FK final_kode_transaksi ditunda sampai COMMIT.
            add(insert("request",
                       ["request_id", "client_id", "credential_id", "downstream_endpoint_id", "downstream_pool_id",
                        "provider_id", "provider_config_version", "endpoint_config_version", "mode", "status",
                        "attempt_count", "metadata", "received_at", "input_payload", "prioritas", "prioritas_efektif"],
                       [rid, client_ids["client-a"], cred_ids["client-a"], ep_of_pool[main_pool], main_pool,
                        prov, 1, 1, "sync", "in_flight", 1, "{}", T0, '{"sintetis": true}', 1, 1]))
            add(insert("request_status_history", ["request_id", "seq", "status_dari", "status_ke", "aktor"],
                       [rid, 1, None, "in_flight", "webserver"]))
            add("UPDATE request SET status = 'done', completed_at = %s, output_payload = '{\"sintetis\": true}', "
                "final_kode_transaksi = %s WHERE request_id = %s;" % (q(T0), q(kode), q(rid)))
            add(insert("request_status_history", ["request_id", "seq", "status_dari", "status_ke", "aktor"],
                       [rid, 2, "in_flight", "done", "worker"]))
        add(insert("transaction_log",
                   ["kode_transaksi", "request_id", "attempt_no", "provider_id", "upstream_pool_id", "client_id",
                    "downstream_endpoint_id", "provider_config_version", "endpoint_config_version", "nik",
                    "requested_at", "responded_at", "duration_ms", "status_akses", "provider_http_status",
                    "log_mode", "worker_id", "response_hash"],
                   [kode, rid, 1, prov, main_up, client_ids["client-a"], ep_of_pool[main_pool], 1, 1, it["nik"],
                    it["tanggal_request"] + "T00:00:00+00:00", T0, 100, "berhasil", 200, "metadata_only",
                    "loader-f1", sha256_hex("sintetis-" + rid)]))

    # request dan transaction_log untuk 50 baris arsip_log.csv (tabel arsip_log tidak dimuat; baris request-nya
    # tetap dimuat agar jumlah request sama dengan 314 request_id unik)
    for ar in read("arsip_log.csv"):
        rid = ar["request_id"]
        kode = ar["kode_transaksi"]
        if rid in seen_req:
            continue
        seen_req.add(rid)
        add(insert("request",
                   ["request_id", "client_id", "credential_id", "downstream_endpoint_id", "downstream_pool_id",
                    "provider_id", "provider_config_version", "endpoint_config_version", "mode", "status",
                    "attempt_count", "metadata", "received_at", "input_payload", "prioritas", "prioritas_efektif"],
                   [rid, client_ids["client-a"], cred_ids["client-a"], ep_of_pool[main_pool], main_pool,
                    prov, 1, 1, "sync", "in_flight", 1, "{}", T0, '{"sintetis": true}', 1, 1]))
        add(insert("request_status_history", ["request_id", "seq", "status_dari", "status_ke", "aktor"],
                   [rid, 1, None, "in_flight", "webserver"]))
        add("UPDATE request SET status = 'done', completed_at = %s, output_payload = '{\"sintetis\": true}', "
            "final_kode_transaksi = %s WHERE request_id = %s;" % (q(T0), q(kode), q(rid)))
        add(insert("request_status_history", ["request_id", "seq", "status_dari", "status_ke", "aktor"],
                   [rid, 2, "in_flight", "done", "worker"]))
        add(insert("transaction_log",
                   ["kode_transaksi", "request_id", "attempt_no", "provider_id", "upstream_pool_id", "client_id",
                    "downstream_endpoint_id", "provider_config_version", "endpoint_config_version", "nik",
                    "requested_at", "responded_at", "duration_ms", "status_akses", "provider_http_status",
                    "log_mode", "worker_id", "response_hash"],
                   [kode, rid, 1, prov, main_up, client_ids["client-a"], ep_of_pool[main_pool], 1, 1, ar["nik"],
                    ar["requested_at"], ar["requested_at"], 100, ar["status_akses"], 200, "metadata_only", "loader-f1",
                    sha256_hex("sintetis-" + rid)]))

    # batch data balikan dan itemnya
    b = batches[0]
    payload_hash = sha256_hex("batch-" + b["batch_id"])
    add(insert("data_balikan_batch",
               ["batch_id", "provider_id", "pemicu", "periode_mulai_at", "periode_selesai_at", "jumlah_item",
                "status", "jadwal_kirim_at", "attempt_count", "payload_hash", "config_ref_version"],
               [batch_id, prov, "jumlah_n", T0, T0, int(b["jumlah_item"]), "diterima", T0, 1, payload_hash, 1]))
    for it in items:
        add(insert("data_balikan_item",
                   ["batch_id", "request_id", "kode_transaksi", "nik", "field_input", "field_metadata", "requested_at"],
                   [batch_id, it["request_id"], uid("kode", it["request_id"]), it["nik"],
                    '{"nama": %s, "tanggal_lahir": %s}' % ('"' + it["nama_sintetis"] + '"', '"' + it["tanggal_lahir"] + '"'),
                    '{"status_penerima_manfaat": "sintetis"}', it["tanggal_request"] + "T00:00:00+00:00"]))

    # ---------- MFA (D7, ADR-0006 §9): secret TOTP dienkripsi envelope; seed dan KEK dari env ----------
    seed, kek_hex = mfa_env()
    ids = [r["pengguna_id"] for r in pengguna]
    for r, enc in zip(pengguna, encrypt_mfa(seed, kek_hex, ids)):
        add(insert("pengguna_mfa",
                   ["pengguna_id", "secret_ciphertext", "secret_dek_terbungkus", "secret_nonce", "kek_id", "status",
                    "langkah_terakhir", "didaftarkan_at", "dikonfirmasi_at"],
                   [r["pengguna_id"], "\\x" + enc["secret_ciphertext"], "\\x" + enc["secret_dek_terbungkus"],
                    "\\x" + enc["secret_nonce"], enc["kek_id"], "aktif", 0, T0, T0]))
    for r in read("mfa_kode_pemulihan.csv"):
        add(insert("kode_pemulihan_mfa", ["kode_id", "pengguna_id", "kode_hash", "dibuat_at", "dipakai_at"],
                   [r["kode_id"], r["pengguna_id"], r["kode_hash"], T0, r["dipakai_at"] or None]))
    # penerbit kode daftar harus orang lain (chk_kode_daftar_penerbit): platform admin kedua
    penerbit = pengguna[1]["pengguna_id"]
    for r in read("mfa_kode_daftar.csv"):
        kedaluwarsa = datetime.datetime.strptime(r["kedaluwarsa_at"], "%Y-%m-%dT%H:%M:%SZ")
        diterbitkan = (kedaluwarsa - datetime.timedelta(hours=24)).strftime("%Y-%m-%dT%H:%M:%S+00:00")
        gagal = int(r["gagal_count"])
        # indeks kode_daftar_mfa_aktif_uq: satu kode aktif per pengguna. Kode kedaluwarsa (U-83c) dan kode
        # yang gagal 5x (U-83d) ditandai gugur; kode aktif hanya U-83a.
        gugur_at = T0 if (gagal >= 5 or r["skenario"] == "U-83c") else None
        gugur_alasan = "gagal_5x" if gagal >= 5 else ("diganti" if gugur_at else None)
        add(insert("kode_daftar_mfa",
                   ["kode_id", "pengguna_id", "kode_hash", "diterbitkan_oleh", "diterbitkan_at", "kedaluwarsa_at",
                    "gagal_count", "gugur_at", "gugur_alasan"],
                   [r["kode_id"], r["pengguna_id"], r["kode_hash"], penerbit, diterbitkan,
                    kedaluwarsa.strftime("%Y-%m-%dT%H:%M:%S+00:00"), gagal, gugur_at, gugur_alasan]))

    # ---------- arsip_restore_permintaan (U-76, DQ-49): audit minta_restore dulu, lalu permintaan ----------
    auditor = pengguna[0]["pengguna_id"]
    for r in read("arsip_restore_permintaan.csv"):
        restore = uid("restore", r["restore_id"])
        audit = uid("audit-restore", r["restore_id"])
        add(insert("audit_akses",
                   ["audit_id", "pengguna_id", "peran", "aksi", "tampilan_nik", "jumlah_record", "alasan", "hasil",
                    "sumber_ip", "sesi_id", "target_restore_id", "terjadi_at"],
                   [audit, auditor, "auditor", "minta_restore", "tidak_ada", 0,
                    "uji restore sintetis " + r["skenario"], "diizinkan", "127.0.0.1", "loader-f1", restore, T0]))
        pulih = 0 if r["skenario"] == "U-76b" else 1      # fixture expected: restore_tidak_ada_baris = 0
        add(insert("arsip_restore_permintaan",
                   ["restore_id", "diminta_oleh", "audit_id", "provider_id", "rentang_mulai", "rentang_selesai",
                    "nik_tersamar", "alasan", "status", "diminta_at", "mulai_at", "siap_at", "kedaluwarsa_at",
                    "jumlah_baris_pulih"],
                   [restore, auditor, audit, prov, "2026-11-01", "2026-11-03", r["nik_tersamar"],
                    "uji restore sintetis " + r["skenario"], "siap", T0, T0, T0,
                    "2026-11-18T00:00:00+00:00", pulih]))

    load = ["BEGIN;", "SET LOCAL ROLE poolrouter_owner;"] + stmts + ["COMMIT;"]
    return "\n".join(load) + "\n", client_ids, cred_ids, ep_of_pool


# Uji negatif: setiap blok harus DITOLAK. Blok tanpa penolakan menggagalkan run.
# Baris yang sengaja melanggar (dari fixture) dipakai di sini, bukan di jalur positif.
NEG = [
    ("NEG-01 NIK 15 digit ditolak (CHECK nik ~ ^[0-9]{16}$)",
     """INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id,
        downstream_endpoint_id, provider_config_version, endpoint_config_version, nik, requested_at, responded_at,
        duration_ms, status_akses, provider_http_status, log_mode, worker_id, response_hash)
        SELECT gen_random_uuid(), request_id, 2, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
        1, 1, '123456789012345', requested_at, responded_at, 100, status_akses, 200, log_mode, worker_id, response_hash
        FROM transaction_log ORDER BY kode_transaksi LIMIT 1;"""),
    ("NEG-02 nik NULL ditolak untuk provider wajib_log_nik (C-01)",
     """INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id,
        downstream_endpoint_id, provider_config_version, endpoint_config_version, nik, requested_at, responded_at,
        duration_ms, status_akses, provider_http_status, log_mode, worker_id, response_hash)
        SELECT gen_random_uuid(), request_id, 3, provider_id, upstream_pool_id, client_id, downstream_endpoint_id,
        1, 1, NULL, requested_at, responded_at, 100, status_akses, 200, log_mode, worker_id, response_hash
        FROM transaction_log ORDER BY kode_transaksi LIMIT 1;"""),
    ("NEG-03 data_balikan_item NIK 15 digit ditolak",
     """INSERT INTO data_balikan_item (batch_id, request_id, kode_transaksi, nik, field_input, field_metadata, requested_at)
        SELECT batch_id, request_id, kode_transaksi, '12345678901234X', field_input, field_metadata, requested_at
        FROM data_balikan_item LIMIT 1;"""),
    ("NEG-04 alokasi reserved melebihi kuota upstream (U-64a: 70 + 50 > 100, DQ-41)",
     """INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id, alokasi_jenis, alokasi_nilai, sifat, boleh_pinjam)
        VALUES ('976d59b6-5e1c-55c4-b67a-847113e0f429', '32191cc0-bd57-53b2-9f21-891d6b916dfd', 'exact', 70.00, 'reserved', false),
        ('8e290df3-0160-5acc-b7ea-ae385715907b', '32191cc0-bd57-53b2-9f21-891d6b916dfd', 'exact', 50.00, 'reserved', false);
        SET CONSTRAINTS ALL IMMEDIATE;"""),
    ("NEG-05 transisi status request tidak sah (done -> queued, tg_request_transisi)",
     """UPDATE request SET status = 'queued' WHERE request_id = (SELECT request_id FROM request WHERE status = 'done' LIMIT 1);"""),
    ("NEG-06 request tanpa baris request_status_history ditolak saat COMMIT (T-04, trigger tertunda)",
     """INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, downstream_pool_id, provider_id,
        provider_config_version, endpoint_config_version, mode, status, attempt_count, metadata, received_at,
        input_payload, prioritas, prioritas_efektif, ttl_expires_at)
        SELECT gen_random_uuid(), client_id, credential_id, downstream_endpoint_id, downstream_pool_id, provider_id,
        1, 1, 'async', 'queued', 0, '{}', received_at, '{"sintetis": true}', 1, 1, '2026-11-05T00:00:00+00:00'
        FROM request WHERE mode = 'sync' LIMIT 1;
        SET CONSTRAINTS ALL IMMEDIATE;"""),
    ("NEG-07 pengguna_mfa aktif tanpa dikonfirmasi_at ditolak (chk_mfa_konfirmasi)",
     """UPDATE pengguna_mfa SET dikonfirmasi_at = NULL
        WHERE pengguna_id = (SELECT pengguna_id FROM pengguna_mfa WHERE status = 'aktif' LIMIT 1);
        SET CONSTRAINTS ALL IMMEDIATE;"""),
    ("NEG-08 langkah_terakhir MFA turun ditolak (MF-03, tg_mfa_langkah_naik)",
     """UPDATE pengguna_mfa SET langkah_terakhir = 5 WHERE pengguna_id = 'e0e43323-37f2-5b8b-9d45-97d9ba27ff17';
        UPDATE pengguna_mfa SET langkah_terakhir = 3 WHERE pengguna_id = 'e0e43323-37f2-5b8b-9d45-97d9ba27ff17';"""),
    ("NEG-09 kode pemulihan yang sudah dipakai tidak bisa dipakai ulang (kode_pemulihan_mfa_sekali)",
     """UPDATE kode_pemulihan_mfa SET dipakai_at = now()
        WHERE kode_id = (SELECT kode_id FROM kode_pemulihan_mfa WHERE dipakai_at IS NULL LIMIT 1);
        UPDATE kode_pemulihan_mfa SET dipakai_at = now()
        WHERE kode_id = (SELECT kode_id FROM kode_pemulihan_mfa WHERE dipakai_at IS NOT NULL LIMIT 1);"""),
    ("NEG-10 permintaan restore tanpa audit minta_restore yang cocok ditolak (DQ-49 a)",
     """INSERT INTO arsip_restore_permintaan (restore_id, diminta_oleh, audit_id, provider_id, rentang_mulai,
        rentang_selesai, nik_tersamar, alasan, status, diminta_at, mulai_at, siap_at, kedaluwarsa_at, jumlah_baris_pulih)
        SELECT gen_random_uuid(), diminta_oleh, gen_random_uuid(), provider_id, rentang_mulai, rentang_selesai,
        nik_tersamar, alasan, status, diminta_at, mulai_at, siap_at, kedaluwarsa_at, jumlah_baris_pulih
        FROM arsip_restore_permintaan LIMIT 1;
        SET CONSTRAINTS ALL IMMEDIATE;"""),
]


def build_neg():
    """Setiap blok: jalankan statement dalam subtransaksi; harus melempar error, kalau tidak run gagal."""
    out = ["-- Uji negatif F1 (SHAA-335). Setiap blok harus ditolak; penolakan yang tidak terjadi menggagalkan run.",
           "\\set ON_ERROR_STOP on"]
    for label, sql in NEG:
        out.append(
            "DO $neg$ DECLARE v_ditolak boolean := false; BEGIN\n"
            "  BEGIN\n"
            + "\n".join("    " + line for line in sql.strip().split("\n")) + "\n"
            "  EXCEPTION WHEN others THEN v_ditolak := true; RAISE NOTICE 'ditolak (%%): %%', '%s', SQLERRM;\n"
            "  END;\n"
            "  IF NOT v_ditolak THEN RAISE EXCEPTION '%s tidak ditolak'; END IF;\n"
            "END $neg$;" % (label.split(" ")[0], label))
    return "\n".join(out) + "\n"


def main():
    outdir = sys.argv[1]
    load_sql, _, _, _ = build()
    with open(os.path.join(outdir, "load_f1.sql"), "w", encoding="utf-8") as fh:
        fh.write(load_sql)
    print("load_f1.sql statements:", load_sql.count(";\n"))
    with open(os.path.join(HERE, "neg_f1.sql"), "w", encoding="utf-8") as fh:
        fh.write(build_neg())


if __name__ == "__main__":
    main()
