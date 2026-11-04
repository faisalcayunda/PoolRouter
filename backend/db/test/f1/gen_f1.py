#!/usr/bin/env python3
"""Generator data uji sintetis F1 PoolRouter (SHAA-335, K2 Quality).

SYNTHETIC ONLY. Tidak ada NIK asli. Tidak memakai data dari sistem sumber.
Dijalankan hanya untuk lingkungan dev/test/CI (contract v0.9.5-final §6.1).

Pemakaian (stdlib saja, tanpa instalasi):

    python3 backend/db/test/f1/gen_f1.py --out backend/db/test/f1/out

Keluaran deterministik: seed tetap 166 (contract §6.1), UUID dari uuid5, waktu
tetap. Menjalankan dua kali menghasilkan berkas identik (lihat manifest.json,
SHA-256 per berkas). Validasi aturan contract dijalankan sebelum menulis; bila
gagal, generator berhenti dengan kode keluar 1 dan tidak menulis apa pun.

Acuan: data-contract-poolrouter-log-v0.9.5-final.md (SHAA-312), BRD/US F1 v0.3
(SHAA-319), ADR-0006 rev 3. Angka harapan ada di expected.json.
"""

import argparse
import base64
import csv
import hashlib
import hmac
import json
import os
import random
import re
import struct
import sys
import uuid
from datetime import datetime, timedelta, timezone

SEED = 166                      # contract §6.1: seed tetap 166
NIK_PREFIX = "00"               # contract §6.1: kode provinsi tak terpakai
NIK_RE = re.compile(r"^00[0-9]{14}$")
EPOCH = datetime(2026, 11, 4, 0, 0, 0, tzinfo=timezone.utc)   # waktu tetap
NS = uuid.UUID("6f1c2a52-0000-4000-8000-00000000f1f1")       # namespace uuid5 tetap
# Kunci HMAC khusus fixture, bukan rahasia. Lingkungan nyata memakai secret storage.
HMAC_KEY_FIXTURE = b"SINTETIS-ONLY-f1-fixture-not-a-secret"

MAX_RETRY_ALLOWED = 6
# Sentinel NIK sintetis untuk gate PR (T-13-08, U-89). Generator tidak pernah
# menghasilkan NIK acak yang sama dengan sentinel.
SENTINEL_NIK = "0000000000009999"
# Batas retry jalur cepat (T-06B-01), fixture terpisah dari jadwal 6 percobaan.
BATAS_RETRY_CEPAT = 3
# Kapasitas upstream untuk uji skala (T-05B-01, T-05B-07).
Q_SKALA = 1000
# Tanggal lahir fiktif: tanggal tetap yang jelas bukan tanggal lahir orang nyata.
TGL_LAHIR_FIKTIF = ["1901-01-01", "1902-02-02", "1903-03-03", "1904-04-04"]
# Akun uji. Seed TOTP TIDAK dibuat generator; dibaca dari ${TOTP_TEST_SEED} saat runtime.
AKUN_UJI = [
    ("mfa-u1", "auditor", "auditor"),
    ("mfa-u2", "platform_admin_1", "platform_admin"),
    ("mfa-u3", "platform_admin_2", "platform_admin"),
    ("mfa-u4", "operator", "operator"),
    ("mfa-u5", "client_owner", "client_owner"),
]

# Kunci unik per berkas keluaran (contract: request_id, webhook_delivery_id, batch+request, dst).
KUNCI_UNIK = {
    "request_aging.csv": ["request_id"],
    "alokasi_rute.csv": ["upstream_pool_id", "downstream_pool_id"],
    "webhook_jadwal.csv": ["webhook_delivery_id"],
    "data_balikan_item.csv": ["batch_id", "request_id"],
    "data_balikan_batch.csv": ["batch_id"],
    "arsip_log.csv": ["request_id"],
    "arsip_restore_permintaan.csv": ["restore_id"],
    "mfa_pengguna.csv": ["pengguna_id"],
    "mfa_kode_pemulihan.csv": ["kode_id"],
    "mfa_kode_daftar.csv": ["kode_id"],
}


def uid(name):
    return str(uuid.uuid5(NS, name))


def ts(delta_seconds=0):
    return (EPOCH + timedelta(seconds=delta_seconds)).strftime("%Y-%m-%dT%H:%M:%SZ")


class Gen:
    def __init__(self):
        self.rng = random.Random(SEED)
        self.niks = {SENTINEL_NIK}
        self.tables = {}   # nama -> (kolom, [baris])
        self.jitter_cek = []  # (jadwal nominal, tunda hasil) per percobaan retry

    def nik(self):
        # 16 digit, prefiks 00, sisanya acak dengan seed tetap; tanpa duplikat.
        while True:
            n = NIK_PREFIX + "".join(str(self.rng.randint(0, 9)) for _ in range(14))
            if n not in self.niks:
                self.niks.add(n)
                return n

    def add(self, nama, kolom, baris):
        self.tables.setdefault(nama, (kolom, []))[1].extend(baris)


def nama_sintetis(i):
    depan = ["Budi", "Siti", "Agus", "Dewi", "Rudi", "Lestari", "Hana", "Yudi"]
    belakang = ["Contoh", "Uji", "Dummy", "Palsu", "Fiktif"]
    return f"{depan[i % len(depan)]} {belakang[(i // len(depan)) % len(belakang)]} {i:04d}"


# ---------------------------------------------------------------- D1 aging (AG-*)
def scen_d1(g):
    kol = ["request_id", "client_id", "mode", "prioritas", "prioritas_maks",
           "aging_ambang_detik", "prioritas_diubah_at", "status", "attempt_count",
           "nik", "nama_sintetis", "skenario"]
    rows = []
    # U-57: prioritas 5, ambang 600 s. U-58: ambang 0 (tidak pernah naik), dan 60.
    spec = [("U-57", 6, 600, 5), ("U-58a", 4, 0, 5), ("U-58b", 4, 60, 5)]
    for skenario, n, ambang, prio in spec:
        for i in range(n):
            rows.append([uid(f"req-{skenario}-{i}"), uid("client-x"), "async", prio, 5,
                         ambang, ts(0), "queued", 0, g.nik(), nama_sintetis(i), skenario])
    g.add("request_aging.csv", kol, rows)
    return {"request_aging": len(rows), "ambang_0_tidak_naik": 4}


# ---------------------------------------------------------- D2 alokasi (AL-*)
def scen_d2(g):
    kol_rute = ["upstream_pool_id", "downstream_pool_id", "alokasi_jenis", "alokasi_nilai",
                "sifat", "boleh_pinjam", "skenario"]
    rows = []
    q = 100
    # U-61: Q=100, A reserved exact 40, B shareable exact 30 (boleh_pinjam), C tanpa alokasi.
    rows.append([uid("up-Q"), uid("dp-A"), "exact", "40.00", "reserved", "false", "U-61"])
    rows.append([uid("up-Q"), uid("dp-B"), "exact", "30.00", "shareable", "true", "U-61"])
    rows.append([uid("up-Q"), uid("dp-C"), "", "", "", "true", "U-61"])
    g.add("alokasi_rute.csv", kol_rute, rows)

    # U-64: Σ a_i > Q harus 422 dengan jumlah kelebihan.
    kelebihan = [[uid("up-Q2"), uid("dp-A2"), "exact", "70.00", "reserved", "false", "U-64a"],
                 [uid("up-Q2"), uid("dp-B2"), "exact", "50.00", "reserved", "false", "U-64a"]]
    g.add("alokasi_rute.csv", kol_rute, kelebihan)

    # U-65: rute tanpa alokasi dengan boleh_pinjam = false -> 422 (CHECK ii).
    g.add("alokasi_rute.csv", kol_rute,
          [[uid("up-Q3"), uid("dp-D3"), "", "", "", "false", "U-65"]])

    # Skala Q = 1000 (T-05B-01, T-05B-07): dua downstream reserved, satu shareable.
    skala = [[uid("up-Q1000"), uid("dp-S1"), "exact", "400.00", "reserved", "false", "T-05B-SKALA"],
             [uid("up-Q1000"), uid("dp-S2"), "exact", "350.00", "reserved", "false", "T-05B-SKALA"],
             [uid("up-Q1000"), uid("dp-S3"), "exact", "200.00", "shareable", "true", "T-05B-SKALA"],
             [uid("up-Q1000"), uid("dp-S4"), "", "", "", "true", "T-05B-SKALA"]]
    g.add("alokasi_rute.csv", kol_rute, skala)

    # Expected results (dihitung di sini, bukan dari implementasi).
    reserved_exact = 40
    a_sisa = q - reserved_exact - 30   # sisa setelah B: C menghabiskan sisa
    return {
        "upstream_pool_Q": q,
        "upstream_pool_Q_skala": Q_SKALA,
        "reserved_exact_A": reserved_exact,
        "shareable_exact_B": 30,
        "a_sisa_u61": a_sisa,             # U-61: a_sisa = 30
        "kelebihan_u64": 120 - 100,       # Σ = 120, Q = 100 -> kelebihan 20
        "skala_reserved_total": 750,      # 400 + 350 <= 1000
    }


# ---------------------------------------------------- D3 retry webhook (WR-*)
def scen_d3(g):
    # Jadwal nominal 30 s, 2 m, 8 m, 32 m, 2 j; percobaan ke-6 gagal.
    # Jitter ilustratif, sumber belum diverifikasi: angka pasti menunggu pemilik
    # contract (SHAA-312) menetapkan jitter. Uji hanya memeriksa rentang dan
    # urutan monoton, bukan nilai tunda tertentu (keputusan Ridley, SHAA-335).
    base, cap, jitter_pct, batas = 30, 7200, 20, 6
    # Jitter diberi seed tetap per percobaan sehingga reproducible.
    jrng = random.Random(SEED + 3)
    kol = ["webhook_delivery_id", "percobaan_ke", "webhook_not_before", "hasil_tanggung_jawab"]
    rows = []
    t = 0
    jadwal = []
    for n in range(1, batas):           # percobaan 1..5 menghasilkan jadwal retry
        kasar = min(cap, base * (4 ** (n - 1)))
        jit = jrng.uniform(-jitter_pct / 100.0, jitter_pct / 100.0)
        tunda = max(1, round(kasar * (1 + jit)))
        t += tunda
        jadwal.append(tunda)
        g.jitter_cek.append((kasar, tunda))
        rows.append([uid(f"wh-{n}"), n, ts(t), "retry"])
    rows.append([uid("wh-6"), batas, "", "gagal"])   # webhook_not_before NULL setelah percobaan ke-6
    g.add("webhook_jadwal.csv", kol, rows)

    # Jalur cepat dengan batas retry 3 (T-06B-01): fixture terpisah, jadwal 6 percobaan tidak diubah.
    cepat = []
    for n in range(1, BATAS_RETRY_CEPAT):
        cepat.append([uid(f"wh-cepat-{n}"), n, ts(30 * n), "retry"])
    cepat.append([uid(f"wh-cepat-{BATAS_RETRY_CEPAT}"), BATAS_RETRY_CEPAT, "", "gagal"])
    g.add("webhook_jadwal_cepat.csv", kol, cepat)

    # Dua client webhook (T-06B-03, T-06B-09, T-06B-10): riwayat harus terisolasi per client.
    klien = []
    for cid, tag in (("client-a", "A"), ("client-b", "B")):
        for n in range(1, 3):
            klien.append([uid(f"wh-{tag}-{n}"), n, ts(30 * n), cid, "retry" if n == 1 else "gagal"])
    g.add("webhook_klien.csv", ["webhook_delivery_id", "percobaan_ke", "webhook_not_before",
                                "client_id", "hasil_tanggung_jawab"], klien)

    # Klasifikasi retry (U-67): 5xx/408/429/jaringan di-retry; sisanya tidak.
    kol_k = ["kelas", "status_contoh", "di_retry"]
    klas = [["gagal_http_5xx", "500", "true"], ["gagal_http_5xx", "503", "true"],
            ["timeout_408", "408", "true"], ["rate_429", "429", "true"],
            ["gagal_jaringan", "", "true"], ["gagal_http_400", "400", "false"],
            ["gagal_http_404", "404", "false"], ["gagal_http_410", "410", "false"],
            ["redirect_301", "301", "false"], ["ditolak_allowlist", "", "false"],
            ["dekripsi_gagal", "", "false"]]
    g.add("webhook_klasifikasi.csv", kol_k, klas)
    # Tidak mengunci nilai tunda: hanya rentang jitter dan jumlah percobaan.
    return {"percobaan_retry": len(jadwal), "jitter_rentang": [0.8, 1.2],
            "klasifikasi_baris": len(klas), "batas_retry_cepat": BATAS_RETRY_CEPAT,
            "client_webhook": 2}


# ------------------------------------------------------- D4 data balikan (DB-*)
def scen_d4(g):
    n = 250   # contract §6.1: ukuran batch fixture N = 250
    kol = ["batch_id", "request_id", "nik", "tanggal_request", "nama_sintetis", "tanggal_lahir", "skenario"]
    batch = uid("batch-1")
    rows = []
    for i in range(n):
        rows.append([batch, uid(f"db-item-{i}"), g.nik(), "2026-11-03", nama_sintetis(i),
                     TGL_LAHIR_FIKTIF[i % len(TGL_LAHIR_FIKTIF)], "U-74"])
    g.add("data_balikan_item.csv", kol, rows)
    g.add("data_balikan_batch.csv",
          ["batch_id", "status", "jumlah_item", "config_schema_version"],
          [[batch, "menunggu_ack", n, 2]])
    return {"batch_item": n}


# -------------------------------------------------------- D6 arsip/restore (AR-*)
def scen_d6(g):
    kol = ["request_id", "kode_transaksi", "nik", "nik_hmac", "kunci_hmac_versi",
           "requested_at", "status_akses", "skenario"]
    rows = []
    for i in range(50):
        nik = g.nik()
        mac = hmac.new(HMAC_KEY_FIXTURE, nik.encode(), hashlib.sha256).hexdigest()
        rows.append([uid(f"ar-req-{i}"), uid(f"ar-kt-{i}"), nik, mac, 1, ts(i), "berhasil", "U-76"])
    g.add("arsip_log.csv", kol, rows)
    # Permintaan restore: NIK yang ada (baris 0) dan NIK yang tidak ada (U-76).
    ada = rows[0][2]
    g.add("arsip_restore_permintaan.csv",
          ["restore_id", "nik_tersamar", "status", "skenario"],
          [[uid("rst-ada"), ada[-4:], "selesai", "U-76a"],
           [uid("rst-tidak-ada"), "9999", "selesai", "U-76b"]])
    # Sentinel NIK untuk gate PR (T-13-08, U-89): satu baris, sengaja tidak dibuat acak.
    g.add("sentinel_nik.csv", ["nik", "skenario"], [[SENTINEL_NIK, "T-13-08"]])
    return {"arsip_log": len(rows), "restore_ada_nik": ada,
            "restore_tidak_ada_baris": 0, "sentinel_nik": SENTINEL_NIK}


# ---------------------------------------------------------------- D7 MFA (MF-*)
def totp(secret_b32, t_detik, langkah=30, digit=6):
    # RFC6238-PUBLIC-VECTOR: RFC 6238 dengan HMAC-SHA1 (U-79, vektor Lampiran B).
    key = base64.b32decode(secret_b32)
    counter = int(t_detik // langkah)
    digest = hmac.new(key, struct.pack(">Q", counter), hashlib.sha1).digest()
    off = digest[-1] & 0x0F
    code = (struct.unpack(">I", digest[off:off + 4])[0] & 0x7FFFFFFF) % (10 ** digit)
    return str(code).zfill(digit)


def scen_d7(g):
    # RFC6238-PUBLIC-VECTOR: "12345678901234567890" dalam base32, nilai publik
    # Lampiran B RFC 6238 (bukan secret). Hanya untuk membuktikan algoritma (U-79).
    secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"
    kol_u = ["pengguna_id", "status", "langkah_terakhir", "secret_ciphertext_palsu", "skenario"]
    # Akun uji (auditor, dua platform admin, operator, client owner). Seed TOTP
    # akun-akun ini TIDAK ditulis di sini: kolom berisi referensi ${TOTP_TEST_SEED}.
    g.add("mfa_pengguna.csv", kol_u,
          [[uid(pid), "aktif", 0, "${TOTP_TEST_SEED}", peran] for pid, _nama, peran in AKUN_UJI])
    # Vektor RFC 6238 SHA-1 (T = 59, 1111111109, ...) dihitung ulang dan dibandingkan
    # dengan nilai standar RFC: 94287082, 07081804, 14050471, 89005924, 69279037, 65353130.
    vektor = [(59, "94287082"), (1111111109, "07081804"), (1111111111, "14050471"),
              (1234567890, "89005924"), (2000000000, "69279037"), (20000000000, "65353130")]
    rows = []
    for t, kode8 in vektor:
        rows.append([t, totp(secret, t, digit=8), kode8, "U-79"])
    g.add("mfa_totp_vektor.csv", ["t_detik", "kode_hitung_8digit", "kode_rfc6238", "skenario"], rows)
    # Kode pemulihan (U-81) dan kode pendaftaran (U-83): hanya placeholder hash, tanpa nilai asli.
    g.add("mfa_kode_pemulihan.csv", ["kode_id", "pengguna_id", "kode_hash", "dipakai_at", "skenario"],
          [[uid(f"pulih-{i}"), uid("mfa-u1"), "$argon2id$SINTETIS-PLACEHOLDER", "", "U-81"]
           for i in range(10)])
    g.add("mfa_kode_daftar.csv", ["kode_id", "pengguna_id", "kode_hash", "kedaluwarsa_at", "gagal_count", "skenario"],
          [[uid("daftar-1"), uid("mfa-u1"), "0" * 64, ts(24 * 3600), 0, "U-83a"],
           [uid("daftar-2"), uid("mfa-u1"), "1" * 64, ts(-1), 0, "U-83c"],
           [uid("daftar-3"), uid("mfa-u1"), "2" * 64, ts(24 * 3600), 5, "U-83d"]])
    return {"totp_vektor": len(vektor), "pemulihan": 10}


# ---------------------------------------------------------------- validasi
def validasi(g):
    errs = []
    # NIK sintetis: 16 digit, prefiks 00, di setiap tabel yang punya kolom nik.
    for nama, (kol, rows) in g.tables.items():
        if "nik" in kol:
            idx = kol.index("nik")
            for r in rows:
                if not NIK_RE.match(r[idx]):
                    errs.append(f"{nama}: NIK bukan sintetis: {r[idx][:4]}...")
                    break
        # Kunci unik per tabel (sesuai contract). Tabel tanpa kunci tunggal tidak diperiksa di sini.
        kunci = KUNCI_UNIK.get(nama)
        if kunci:
            idx = [kol.index(k) for k in kunci]
            vals = [tuple(r[i] for i in idx) for r in rows]
            if len(vals) != len(set(vals)):
                errs.append(f"{nama}: kunci {kunci} tidak unik")
    # Sentinel harus tepat satu di berkas sentinel_nik.csv dan tidak muncul di tempat lain.
    sen = g.tables.get("sentinel_nik.csv", ([], []))[1]
    if [r[0] for r in sen] != [SENTINEL_NIK]:
        errs.append("sentinel: harus tepat satu baris di sentinel_nik.csv")
    for nama, (kol, rows) in g.tables.items():
        if nama == "sentinel_nik.csv":
            continue
        for r in rows:
            if SENTINEL_NIK in r:
                errs.append(f"sentinel: muncul di {nama}")
                break
    # Tanggal lahir harus dari daftar fiktif.
    for nama, (kol, rows) in g.tables.items():
        if "tanggal_lahir" in kol:
            idx = kol.index("tanggal_lahir")
            if any(r[idx] not in TGL_LAHIR_FIKTIF for r in rows):
                errs.append(f"{nama}: tanggal lahir bukan fiktif")
    # Alokasi: Σ exact reserved <= Q pada U-61 (40+30 = 70 <= 100).
    # Jitter: tunda berada dalam nominal × [0,8; 1,2] (toleransi pembulatan 1 s).
    for kasar, tunda in g.jitter_cek:
        if not (0.8 * kasar - 1 <= tunda <= 1.2 * kasar + 1):
            errs.append(f"webhook: tunda {tunda} di luar rentang nominal {kasar} x [0,8; 1,2]")
            break
    # Jadwal retry: webhook_not_before harus naik monoton, percobaan terakhir tanpa jadwal.
    jad = [r[2] for r in g.tables.get("webhook_jadwal.csv", ([], []))[1] if r[2]]
    if jad != sorted(jad) or len(set(jad)) != len(jad):
        errs.append("webhook: jadwal tidak naik monoton")
    # Batch N = 250 (§6.1).
    if len(g.tables.get("data_balikan_item.csv", ([], []))[1]) != 250:
        errs.append("data balikan: N != 250")
    # Vektor TOTP harus cocok dengan nilai RFC (8 digit).
    for r in g.tables.get("mfa_totp_vektor.csv", ([], []))[1]:
        if r[1] != r[2]:
            errs.append(f"TOTP RFC 6238 tidak cocok pada t={r[0]}: {r[1]} != {r[2]}")
    # Kebocoran NIK: tidak ada NIK 16-digit non-00 di seluruh teks keluaran.
    return errs


def tulis(g, out_dir, ringkasan):
    os.makedirs(out_dir, exist_ok=True)
    manifest = {"seed": SEED, "contract": "data-contract-poolrouter-log-v0.9.5-final",
                "sintetis": True, "berkas": {}}
    for nama in sorted(g.tables):
        kol, rows = g.tables[nama]
        path = os.path.join(out_dir, nama)
        with open(path, "w", newline="", encoding="utf-8") as f:
            w = csv.writer(f, lineterminator="\n")
            w.writerow(kol)
            w.writerows(rows)
        with open(path, "rb") as f:
            sha = hashlib.sha256(f.read()).hexdigest()
        manifest["berkas"][nama] = {"baris": len(rows), "sha256": sha}
    with open(os.path.join(out_dir, "expected.json"), "w", encoding="utf-8") as f:
        json.dump(ringkasan, f, indent=2, sort_keys=True)
        f.write("\n")
    with open(os.path.join(out_dir, "manifest.json"), "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2, sort_keys=True)
        f.write("\n")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("--out", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "out"))
    args = ap.parse_args()

    g = Gen()
    ringkasan = {
        "d1_aging": scen_d1(g),
        "d2_alokasi": scen_d2(g),
        "d3_webhook": scen_d3(g),
        "d4_data_balikan": scen_d4(g),
        "d6_arsip": scen_d6(g),
        "d7_mfa": scen_d7(g),
    }
    errs = validasi(g)
    if errs:
        for e in errs:
            print("GAGAL:", e, file=sys.stderr)
        return 1
    tulis(g, args.out, ringkasan)
    print(f"ok: {len(g.tables)} berkas ditulis ke {args.out} (seed {SEED}, sintetis)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
