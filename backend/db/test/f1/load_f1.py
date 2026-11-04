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
    mfa_*.csv, webhook_log                                                 (lihat catatan MFA di load_f1.sh)
- Baris yang melanggar constraint dengan sengaja (U-64a Σ > Q, U-65 boleh_pinjam = false tanpa alokasi)
  TIDAK dimuat di jalur positif. Dibuat sebagai uji negatif di neg_f1.sql.

Keluaran: <outdir>/load_f1.sql (pemuatan positif, satu transaksi) dan <outdir>/neg_f1.sql (uji negatif).
"""

import csv
import hashlib
import os
import sys
import uuid

HERE = os.path.dirname(os.path.abspath(__file__))
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
