#!/usr/bin/env python3
"""Uji skrip Lua kuota terhadap Redis sungguhan.

Uji ini menjalankan berkas HASIL RAKITAN (`rakitan/`), yaitu bentuk yang sama
dengan yang dimuat Redis di produksi. Perakitan dilakukan `scripts/rakit-lua.sh`
saat uji dimulai, lalu hasilnya dibandingkan byte demi byte dengan `rakitan/`
yang di-commit, jadi rakitan yang basi gagal keras di sini. Sandbox Redis tidak
punya `dofile`, sehingga versi lama yang menyisipkan potongan sendiri lalu
menghapus baris `dofile` tidak pernah menguji byte produksi (review 778f9fd,
temuan blocking 2).

Keputusan Shu SHAA-195 (addendum ADR-0001 rev 15) yang diuji di sini:
  D1    gerbang M-02 adalah `kuota_ambil_token` (kuota + slot dalam satu EVAL),
        bukan intake; uji konkurensi menjalankan tiga konfigurasi dan empat
        jalur, tanpa toleransi pada asersi M-02.
  D1.9  slot yang pemiliknya mati tetap terhitung sampai TTL, pembersihannya
        hanya di `kuota_ambil_token`, dan percobaan yang telanjur macet tidak
        bisa menaikkan `attempts` sesudah slotnya kedaluwarsa.
  D2    GCRA benar; yang salah definisi jendela baca. Satu definisi: nomor
        ember absolut `floor(unix/600)`, jendela 145 ember, field `b<n>`,
        `Retry-After` dari ember tertua, `isi` bucket melaporkan `burst` saat
        penuh, dan TTL TAT `TAT_baru − now + 60 s`.

Anggaran alokasi (spec `f1-prioritas-alokasi`, ARC-108) diuji lewat
`potongan/alokasi.lua`: potongan itu belum disisipkan ke skrip kuota mana pun
sampai T2 menegakkannya, jadi harness memanggilnya langsung, dalam bentuk yang
sama dengan yang nanti disisipkan perakit.

Pemakaian: `python3 uji_lua.py [--host 127.0.0.1] [--port 6395] [--iterasi 300]`
(gate: `make -C backend uji-lua`, yang menjalankan ini di dalam container
Redis 7 supaya seri Redis-nya sama dengan produksi).
"""

from __future__ import annotations

import argparse
import difflib
import json
import os
import random
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
RAKITAN_DIR = HERE / "rakitan"
def _akar() -> Path:
    """Direktori repo: tempat `scripts/rakit-lua.sh` berada."""
    for kandidat in HERE.parents:
        if (kandidat / "scripts" / "rakit-lua.sh").is_file():
            return kandidat
    raise SystemExit("scripts/rakit-lua.sh tidak ditemukan di atas direktori uji")


AKAR = _akar()
RAKIT = AKAR / "scripts" / "rakit-lua.sh"

# Nama pendek -> berkas rakitan. Semua entrypoint D1 wajib ada di sini; skrip
# yang lupa dirakit akan gagal saat dimuat, bukan diam-diam dilewati.
NAMA_SKRIP = {
    "intake": "kuota_intake.lua",
    "intake_alokasi": "kuota_intake_alokasi.lua",
    "token": "kuota_ambil_token.lua",
    "token_alokasi": "kuota_ambil_token_alokasi.lua",
    "klaim": "slot_klaim.lua",
    "lepas": "slot_lepas.lua",
    "tambah": "attempts_tambah.lua",
    "baca": "kuota_baca.lua",
    "tolak": "penolakan_ambil.lua",
}

JUMLAH_EMBER = 145
MENIT_EMBER_DETIK = 600


class Redis:
    """Klien RESP minimal: cukup untuk skrip uji ini, tanpa dependensi."""

    def __init__(self, host: str, port: int):
        self.sock = socket.create_connection((host, port), timeout=10)
        self.buf = self.sock.makefile("rb")

    def close(self):
        self.sock.close()

    def _tulis(self, *bagian: str) -> None:
        out = [f"*{len(bagian)}\r\n".encode()]
        for b in bagian:
            b = str(b).encode()
            out.append(f"${len(b)}\r\n".encode() + b + b"\r\n")
        self.sock.sendall(b"".join(out))

    def _baca(self):
        baris = self.buf.readline()
        if not baris:
            raise ConnectionError("koneksi Redis tertutup")
        jenis, isi = baris[:1], baris[1:-2]
        if jenis == b"+":
            return isi.decode()
        if jenis == b"-":
            raise RuntimeError(isi.decode())
        if jenis == b":":
            return int(isi)
        if jenis == b"$":
            n = int(isi)
            if n < 0:
                return None
            data = self.buf.read(n + 2)
            return data[:-2].decode()
        if jenis == b"*":
            n = int(isi)
            if n < 0:
                return None
            return [self._baca() for _ in range(n)]
        raise RuntimeError(f"RESP tak dikenal: {baris!r}")

    def cmd(self, *bagian):
        self._tulis(*bagian)
        return self._baca()

    def evalsha(self, sha: str, keys: list[str], argv: list[str]):
        return self.cmd("EVALSHA", sha, len(keys), *keys, *argv)


class Skrip:
    """Satu skrip dalam bentuk yang dimuat ke Redis (berkas hasil rakitan)."""

    def __init__(self, redis: Redis, nama: str, sumber: str):
        self.nama = nama
        self.sumber = sumber
        self.sha = redis.cmd("SCRIPT", "LOAD", sumber)


def rakit_semua() -> dict[str, str]:
    """Rakit ulang dengan perakit produksi dan pastikan `rakitan/` tidak basi.

    Perakit dijalankan ke direktori sementara, lalu hasilnya dibandingkan
    dengan `rakitan/` yang di-commit. Kalau berbeda, uji berhenti: yang diuji
    harus persis byte yang dimuat ke Redis.
    """
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        hasil = subprocess.run(
            ["sh", str(RAKIT), "--keluaran", tmp],
            capture_output=True,
            text=True,
            check=False,
            env={**os.environ, "RAKIT_LUA_DIR": str(HERE)},
        )
        if hasil.returncode != 0:
            raise SystemExit(f"rakit-lua gagal:\n{hasil.stdout}\n{hasil.stderr}")
        baru = {p.name: p.read_bytes() for p in Path(tmp).glob("*.lua")}

    keluaran: dict[str, str] = {}
    for nama, isi in baru.items():
        terkunci_path = RAKITAN_DIR / nama
        if not terkunci_path.is_file():
            raise SystemExit(
                f"rakitan/{nama} belum ada; jalankan `make -C backend lua-rakit`."
            )
        terkunci = terkunci_path.read_bytes()
        if isi != terkunci:
            selisih = "".join(
                difflib.unified_diff(
                    terkunci.decode().splitlines(keepends=True),
                    isi.decode().splitlines(keepends=True),
                    fromfile=f"rakitan/{nama} (di-commit)",
                    tofile=f"{nama} (hasil rakit sekarang)",
                )
            )
            raise SystemExit(
                f"rakitan/{nama} basi atau tidak ada; jalankan "
                f"`make -C backend lua-rakit`.\n{selisih}"
            )
        teks = isi.decode()
        # Berkas yang dimuat Redis tidak boleh memuat berkas lain: sandbox
        # menolaknya, dan itu yang membuat potongan harus disisipkan saat build.
        for terlarang in ("dofile(", "require(", "loadstring("):
            if terlarang in teks:
                raise SystemExit(
                    f"rakitan/{nama} memuat `{terlarang}`; sandbox Redis menolaknya"
                )
        keluaran[nama] = teks
    return keluaran


class Uji:
    def __init__(self, redis: Redis):
        self.r = redis
        # Alamat dipakai uji konkurensi untuk membuka koneksi baru.
        self.alamat = redis.sock.getpeername()[:2]
        sumber = rakit_semua()
        self.skrip = {
            pendek: Skrip(redis, bef, sumber[bef])
            for pendek, bef in NAMA_SKRIP.items()
        }
        self.lolos = 0
        self.gagal = 0

    # -- infrastruktur ----------------------------------------------------

    def periksa(self, nama: str, kondisi: bool, pesan: str = "") -> None:
        if kondisi:
            self.lolos += 1
            print(f"  ok   {nama}")
        else:
            self.gagal += 1
            print(f"  GAGAL {nama}: {pesan}")

    def reset(self) -> None:
        self.r.cmd("FLUSHALL")
        self.siapkan_run("run-1")

    def siapkan_run(self, run_id: str) -> None:
        self.r.cmd("SET", "pr:siap", run_id)

    def mikrodetik(self) -> int:
        detik, mikro = self.r.cmd("TIME")
        return int(detik) * 1000000 + int(mikro)

    def panggil(self, skrip: str, keys: list[str], argv: list[str]):
        return self.r.evalsha(self.skrip[skrip].sha, keys, argv)

    # -- anggaran alokasi (potongan/alokasi.lua) --------------------------

    def alokasi(self, q, daftar, rute=None, terpakai_rute=0, terpakai_pool=0):
        """Panggil potongan anggaran di dalam Redis, balasannya list.

        Potongan adalah BERKAS, bukan skrip: sandbox Redis tidak punya
        `dofile`. Pembungkus di bawah menyalin bentuk yang dipakai perakit
        (`local alokasi = (function() ... end)()`) di depan isi potongan, jadi
        yang diuji adalah fungsi yang benar-benar akan disisipkan ke skrip
        kuota, bukan salinannya.

        ARGV skrip: Q, lalu pasangan `jenis nilai` per alokasi, lalu — bila
        `rute is not False` — `terpakai_rute terpakai_pool jenis_rute
        [nilai_rute]`. `rute = None` berarti rute tanpa alokasi (AL-05).
        """
        sumber = (HERE / "potongan" / "alokasi.lua").read_text()
        pakai_rute = rute is not False
        pembungkus = [
            "local alokasi = (function()",
            sumber,
            "end)()",
            # ARGV[2..] adalah daftar datar; jumlah pasangan dihitung di sini,
            # sama seperti skrip kuota yang mengirim `n` di ARGV.
            "local awal = 2",
        ]
        if pakai_rute:
            argv = [str(q)]
            for pasangan in daftar:
                argv.extend([str(pasangan[0]), str(pasangan[1])])
            # Alokasi dihitung dari ARGV[2..2+2n-1]; ekor rute di belakangnya.
            jumlah = len(daftar)
            pembungkus.append(f"local n = {jumlah}")
            pembungkus.append(f"local ekor = awal + 2 * n")
            argv.extend([str(terpakai_rute), str(terpakai_pool)])
            argv.append("" if rute is None else str(rute[0]))
            if rute is not None:
                argv.append(str(rute[1]))
        else:
            argv = [str(q)]
            for pasangan in daftar:
                argv.extend([str(pasangan[0]), str(pasangan[1])])
            jumlah = len(daftar)
            pembungkus.append(f"local n = {jumlah}")
            pembungkus.append("local ekor = awal + 2 * n")
        pembungkus += [
            "local daftar, kode = alokasi.urai(ARGV, awal, n)",
            "if daftar == nil then return { 'galat', kode } end",
            "local a, kode2 = alokasi.hitung(ARGV[1], daftar)",
            "if a == nil then return { 'galat', kode2 } end",
            "local hasil = { 'ok', a.kuota, a.reserved, a.shareable }",
        ]
        if pakai_rute:
            pembungkus += [
                "local rute = nil",
                "if ARGV[ekor + 2] ~= nil and ARGV[ekor + 2] ~= '' then",
                "  rute = { jenis = ARGV[ekor + 2], nilai = ARGV[ekor + 3] }",
                "end",
                "hasil[#hasil + 1] = alokasi.batas_rute(a, rute, ARGV[ekor], ARGV[ekor + 1])",
            ]
        pembungkus.append("return hasil")
        sha = self.r.cmd("SCRIPT", "LOAD", "\n".join(pembungkus))
        return self.r.evalsha(sha, [], argv)

    def uji_alokasi_bagian(self) -> None:
        print("anggaran alokasi: reserved, shareable, dan batas rute (AL-01..AL-05)")
        h = self.alokasi(297, [["exact", 30], ["persen", 10]], rute=False)
        self.periksa("reserved = exact + floor(Q x persen)",
                     h == ["ok", 297, 59, 238], str(h))

        # `persen` dibulatkan KE BAWAH: 10% dari 297 = 29.7 -> 29.
        h = self.alokasi(297, [["persen", 10]], rute=False)
        self.periksa("persen dibulatkan ke bawah", h == ["ok", 297, 29, 268], str(h))

        # Tepat pas boleh: reserved == Q, tidak ada yang shareable.
        h = self.alokasi(297, [["exact", 297]], rute=False)
        self.periksa("reserved tepat sama dengan Q diterima",
                     h == ["ok", 297, 297, 0], str(h))

        # AL-01: total di atas Q adalah galat konfigurasi, bukan pemotongan.
        h = self.alokasi(297, [["exact", 298]], rute=False)
        self.periksa("reserved > Q: alokasi_melebihi_anggaran",
                     h == ["galat", "alokasi_melebihi_anggaran"], str(h))
        h = self.alokasi(297, [["exact", 200], ["persen", 40]], rute=False)
        self.periksa("gabungan exact + persen yang melebihi Q ditolak",
                     h == ["galat", "alokasi_melebihi_anggaran"], str(h))

        # Tanpa alokasi: seluruh Q shareable.
        h = self.alokasi(297, [], rute=False)
        self.periksa("tanpa alokasi: shareable = Q", h == ["ok", 297, 0, 297], str(h))

    def uji_alokasi_tidak_sah(self) -> None:
        print("anggaran alokasi: masukan tidak sah ditolak, bukan ditebak")
        for nama, daftar in [
            ("exact nol", [["exact", 0]]),
            ("exact negatif", [["exact", -3]]),
            ("exact pecahan", [["exact", 2.5]]),
            ("persen nol", [["persen", 0]]),
            ("persen negatif", [["persen", -1]]),
            ("persen di atas 100", [["persen", 100.01]]),
            ("jenis tidak dikenal", [["persentase", 10]]),
            ("nilai bukan angka", [["exact", "banyak"]]),
        ]:
            h = self.alokasi(297, daftar, rute=False)
            self.periksa(f"{nama}: alokasi_tidak_sah",
                         h == ["galat", "alokasi_tidak_sah"], str(h))

        # Q tidak sah adalah galat konfigurasi, bukan "kuota habis".
        for q in [0, -1, 2.5]:
            h = self.alokasi(q, [], rute=False)
            self.periksa(f"Q = {q}: kuota_efektif_tidak_sah",
                         h == ["galat", "kuota_efektif_tidak_sah"], str(h))

    def uji_alokasi_batas_rute(self) -> None:
        print("anggaran alokasi: batas rute menghormati reserved dan shareable")
        # Q = 297, reserved 30 exact + 29 persen = 59, shareable 238.
        a = [["exact", 30], ["persen", 10]]

        # Rute ber-alokasi yang sudah memakai 110 (30 reserved + 80 shareable),
        # sementara pool sudah 297: sisa shareable untuk rute ini 51.
        h = self.alokasi(297, a, rute=["exact", 30], terpakai_rute=110, terpakai_pool=297)
        self.periksa("rute ber-alokasi memakai sisa shareable",
                     h == ["ok", 297, 59, 238, 51], str(h))

        # `terpakai_pool` adalah percobaan SELURUH rute, termasuk rute yang
        # sedang dihitung: rute A yang memakai 30 (jatahnya) menyisakan 238
        # shareable utuh, karena bagian shareable yang terpakai adalah
        # 30 - 30 = 0.
        h = self.alokasi(297, a, rute=["exact", 30], terpakai_rute=30, terpakai_pool=30)
        self.periksa("pemakaian di dalam jatah tidak memakan shareable",
                     h == ["ok", 297, 59, 238, 238], str(h))

        # Rute tanpa alokasi hanya meminjam dari shareable: tidak ada jatah.
        h = self.alokasi(297, a, rute=None, terpakai_rute=20, terpakai_pool=110)
        self.periksa("rute tanpa alokasi dibatasi sisa Q",
                     h == ["ok", 297, 59, 238, 187], str(h))

        # Jatah rute habis dan rute lain memakai seluruh shareable: 0, bukan
        # angka negatif yang bisa terbaca sebagai jatah besar.
        h = self.alokasi(297, a, rute=["exact", 30], terpakai_rute=30, terpakai_pool=268)
        self.periksa("jatah habis dan shareable habis: 0", h[-1] == 0, str(h))
        h = self.alokasi(297, a, rute=None, terpakai_rute=0, terpakai_pool=9999)
        self.periksa("Q terlampaui: batas 0, bukan negatif", h[-1] == 0, str(h))

    # -- T2a: skrip alokasi + hook (ARC-169) ------------------------------

    def _pasangan(self, siap, **kw) -> tuple[list, list]:
        """Jalankan F0 dan skrip alokasi pada state yang di-seed identik."""
        self.reset()
        siap()
        return (self.token(skrip="token", **kw),
                self.token(skrip="token_alokasi", **kw))

    def uji_pasangan_alokasi(self) -> None:
        """AC-5: penolakan tahap global sama persis dengan F0 pada state sama."""
        print("pasangan alokasi: setiap penolakan global identik dengan F0")

        a, b = self._pasangan(lambda: self.r.cmd("DEL", "pr:siap"))
        self.periksa("belum_siap sama", a == b and a[0] == "belum_siap", str(a))

        def run_lain():
            self.siapkan_run("run-lain")

        a, b = self._pasangan(run_lain, run="run-1")
        self.periksa("belum_siap (run lain) sama",
                     a == b and a[0] == "belum_siap", str(a))

        def kuota_habis():
            self.siapkan_run("run-1")
            self.r.cmd("SET", self.k_attempts("p2", 0), "5")

        a, b = self._pasangan(kuota_habis, pool="p2", mode="kalender", kuota=5)
        self.periksa("kuota_habis sama", a == b and a[0] == "kuota_habis", str(a))

        def konkurensi_penuh():
            self.siapkan_run("run-1")
            skor = self.mikrodetik() + 7_200_000_000
            for i in range(3):
                self.r.cmd("ZADD", self.k_slot("p3"), skor, f"m{i}:1")

        a, b = self._pasangan(konkurensi_penuh, pool="p3", maks=3)
        self.periksa("konkurensi_penuh sama",
                     a == b and a[0] == "konkurensi_penuh", str(a))

        def retry_budget_habis():
            self.siapkan_run("run-1")

        a, b = self._pasangan(retry_budget_habis, nomor=2, rasio="0", retry_min=0)
        self.periksa("retry_budget_habis sama",
                     a == b and a[0] == "retry_budget_habis", str(a))

        def rate_limit():
            self.siapkan_run("run-1")
            self.r.cmd("SET", self.k_rl("w0", "p4"),
                       str(self.mikrodetik() + 1_000_000))

        a, b = self._pasangan(rate_limit, pool="p4", kuota=1000)
        # `tunggu` dihitung dari Redis TIME yang bergerak di antara dua panggilan;
        # selisih mikrodetik bukan penyimpangan perilaku.
        self.periksa("rate_limit sama (selain tunggu)",
                     a[0] == b[0] == "rate_limit"
                     and a[1:4] == b[1:4] and a[5:] == b[5:]
                     and abs(a[4] - b[4]) < 50_000, f"{a} vs {b}")

    def uji_hook_alokasi(self) -> None:
        """Hook opsional dipanggil sesudah semua gerbang F0 dan sebelum tulis."""
        print("hook alokasi: sesudah gerbang F0, sebelum tulis")
        sumber = ("local bersama = (function()\n"
                  + (HERE / "potongan" / "bersama.lua").read_text()
                  + "\nend)()\nlocal ambil_token = (function()\n"
                  + (HERE / "potongan" / "ambil_token.lua").read_text()
                  + "\nend)()\n")
        # Pembungkus uji: `o` berbentuk sama dengan milik skrip alokasi, ditambah
        # dua hook yang mencatat pemanggilannya. Yang diuji potongan asli, bukan
        # salinannya.
        pembungkus = """
local dipanggil = {}
local function sekarang()
  local t = redis.call('TIME')
  return tonumber(t[1]) * 1000000 + tonumber(t[2])
end
local o = {
  a = { mode = 'kalender', kuota_efektif = tonumber(ARGV[1]), jumlah_ember = 145, no_ember = 1000 },
  sync = '0', maks = 8, ttl = 80000000, rasio_retry = 0.1, retry_min = 0,
  porsi = 0.2, nomor = 1, request_id = 'hook-uji', sekarang = sekarang,
  jendela = { { t = 200000, burst = 5, kunci = KEYS[6], kunci_sync = KEYS[7] } },
}
o.pilih_sumber = function(KEYS, o)
  dipanggil[#dipanggil + 1] = 'pilih'
  if ARGV[2] == 'tolak' then
    return { kode = 'anggaran_habis', tabel = { 'anggaran_habis' } }
  end
end
o.kunci_slot_sumber = function(KEYS, o, member)
  dipanggil[#dipanggil + 1] = 'kunci'
end
local hasil = ambil_token.ambil_token(KEYS, o)
return { hasil.kode, table.concat(dipanggil, ','), redis.call('ZCARD', KEYS[4]),
         redis.call('EXISTS', KEYS[6]), redis.call('EXISTS', KEYS[5]) }
"""
        keys = ["pr:siap", self.k_attempts(), self.k_ember(), self.k_slot(),
                self.k_retry(), self.k_rl("w0")]
        sha = self.r.cmd("SCRIPT", "LOAD", sumber + pembungkus)

        def hook(kuota: int, aksi: str, terpakai: int = 0) -> list:
            self.reset()
            self.r.cmd("SET", keys[1], str(terpakai))
            return self.r.evalsha(sha, keys, [str(kuota), aksi])

        h = hook(10, "tolak", terpakai=10)
        self.periksa("gerbang F0 menolak: hook tidak dipanggil",
                     h[0] == "kuota_habis" and h[1] == "", str(h))
        h = hook(10, "tolak")
        self.periksa("hook menolak sesudah gerbang F0",
                     h[0] == "anggaran_habis" and h[1] == "pilih", str(h))
        self.periksa("penolakan hook tanpa tulis",
                     h[2] == 0 and h[3] == 0 and h[4] == 0, str(h))
        h = hook(10, "lolos")
        self.periksa("hook lolos: pilih lalu kunci",
                     h[0] == "ok" and h[1] == "pilih,kunci", str(h))
        self.periksa("hook lolos: tulis terjadi",
                     h[2] == 1 and h[3] == 1 and h[4] == 1, str(h))

    # -- T2b: tahap anggaran di skrip alokasi (ARC-170) --------------------

    def _beban(self, pool="p1") -> dict[str, int]:
        """`u_x` seperti yang dilihat skrip: commit di jendela + slot aktif."""
        u: dict[str, int] = {}
        isi = self.r.cmd("HGETALL", self.k_anggaran(pool)) or []
        for i in range(0, len(isi), 2):
            sumber = isi[i].rsplit(":", 1)[0]
            u[sumber] = u.get(sumber, 0) + int(isi[i + 1])
        sumber_slot = self.r.cmd("HGETALL", self.k_slot_sumber(pool)) or []
        for i in range(0, len(sumber_slot), 2):
            member = sumber_slot[i]
            sumber = sumber_slot[i + 1]
            if self.r.cmd("ZSCORE", self.k_slot(pool), member) is not None:
                u[sumber] = u.get(sumber, 0) + 1
        return u

    def uji_anggaran_pilih(self) -> None:
        print("anggaran: sumber pertama yang punya ruang, lalu anggaran_habis (AC-6)")
        self.reset()
        urutan = [("A", 10), ("B", 10)]

        h = self.token(skrip="token_alokasi", request_id="r1", urutan=urutan)
        self.periksa("token diberikan", h[0] == "ok", str(h))
        self.periksa("sumber pertama dicatat di slot_sumber",
                     self.r.cmd("HGET", self.k_slot_sumber(), "r1:1") == "A")

        # A penuh oleh commit: percobaan berikut pindah ke B.
        self.r.cmd("HSET", self.k_anggaran(), "A:1000", "10")
        h = self.token(skrip="token_alokasi", request_id="r2", urutan=urutan)
        self.periksa("A penuh: pindah ke B", h[0] == "ok"
                     and self.r.cmd("HGET", self.k_slot_sumber(), "r2:1") == "B",
                     str(h))

        # Keduanya penuh: anggaran_habis tanpa slot baru. Commit tertua
        # 1000 berada di jendela, sehingga Retry-After mengikuti jendela 24 jam.
        self.r.cmd("HSET", self.k_anggaran(), "B:1000", "10")
        sebelum = self.r.cmd("ZCARD", self.k_slot())
        h = self.token(skrip="token_alokasi", request_id="r3", urutan=urutan)
        self.periksa("semua penuh: anggaran_habis", h[0] == "anggaran_habis", str(h))
        self.periksa("Retry-After tidak negatif",
                     h[4] >= 0, str(h))
        self.periksa("penolakan anggaran tanpa slot baru",
                     self.r.cmd("ZCARD", self.k_slot()) == sebelum
                     and self.r.cmd("HGET", self.k_slot_sumber(), "r3:1") is None)

        # Gerbang F0 tetap lebih dulu: kuota habis, bukan anggaran_habis.
        h = self.token(skrip="token_alokasi", request_id="r4", urutan=urutan,
                       kuota=2)
        self.periksa("F0 lebih dulu daripada anggaran", h[0] == "kuota_habis", str(h))

    def uji_anggaran_jendela(self) -> None:
        print("anggaran: jendela 24 jam dan slot aktif menjadi beban (AC-6)")
        self.reset()
        self.r.cmd("HSET", self.k_anggaran(), "A:800", "99")
        h = self.token(skrip="token_alokasi", request_id="r1", urutan=[("A", 1)])
        self.tambah(member="r1:1", alokasi=True)
        self.periksa("commit di luar jendela dipangkas sesudah commit",
                     h[0] == "ok"
                     and self.r.cmd("HGET", self.k_anggaran(), "A:800") is None, str(h))

        # Slot r1:1 membebani A: batas 1 sudah terpakai, jadi pindah ke B.
        h = self.token(skrip="token_alokasi", request_id="r2", urutan=[("A", 1), ("B", 1)])
        self.periksa("slot aktif membebani sumbernya",
                     h[0] == "ok"
                     and self.r.cmd("HGET", self.k_slot_sumber(), "r2:1") == "B", str(h))

        # Slot kedaluwarsa dibersihkan lengkap dengan entri sumbernya.
        self.reset()
        self.r.cmd("ZADD", self.k_slot(), str(self.mikrodetik() - 1), "mati:1")
        self.r.cmd("HSET", self.k_slot_sumber(), "mati:1", "A")
        h = self.token(skrip="token_alokasi", request_id="r1", urutan=[("A", 1)])
        self.periksa("slot kedaluwarsa tidak lagi membebani",
                     h[0] == "ok" and h[1] >= 0, str(h))
        self.periksa("entri slot_sumber slot kedaluwarsa dihapus",
                     self.r.cmd("HGET", self.k_slot_sumber(), "mati:1") is None)

    def uji_anggaran_beban(self) -> None:
        print("anggaran: satu percobaan satu beban, commit menggantikan slot (AC-6/AC-7)")
        self.reset()
        self.token(skrip="token_alokasi", request_id="r1", urutan=[("A", 2)])

        h = self.tambah(member="r1:1", alokasi=True)
        self.periksa("attempts tetap tercatat", h[0] == "ok", str(h))
        self.periksa("commit = 1 beban di anggaran A:1000",
                     self.r.cmd("HGET", self.k_anggaran(), "A:1000") == "1")
        self.periksa("entri slot_sumber dihapus: beban tidak dobel",
                     self.r.cmd("HGET", self.k_slot_sumber(), "r1:1") is None)
        self.periksa("slot tetap aktif untuk konkurensi",
                     self.r.cmd("ZCARD", self.k_slot()) == 1)

        self.lepas(member="r1:1")
        self.periksa("beban bertahan sesudah slot dilepas",
                     self.r.cmd("HGET", self.k_anggaran(), "A:1000") == "1")

        self.r.cmd("HSET", self.k_anggaran(), "A:800", "5")
        self.token(skrip="token_alokasi", request_id="r2", urutan=[("A", 2)])
        self.tambah(member="r2:1", alokasi=True)
        self.periksa("commit kedua menambah beban dan memangkas jendela",
                     self.r.cmd("HGET", self.k_anggaran(), "A:1000") == "2"
                     and self.r.cmd("HGET", self.k_anggaran(), "A:800") is None)

        h = self.token(skrip="token_alokasi", request_id="r3", urutan=[("A", 2)])
        self.periksa("A di batas: anggaran_habis",
                     h[0] == "anggaran_habis", str(h))

        # Tanpa kunci alokasi, jalur F0 tidak menyentuh anggaran sama sekali.
        self.reset()
        self.token(skrip="token_alokasi", request_id="f0", urutan=[("A", 2)])
        self.tambah(member="f0:1")
        self.periksa("pool tanpa alokasi tidak membebani anggaran",
                     self.r.cmd("HLEN", self.k_anggaran()) == 0)
        self.periksa("pool tanpa alokasi tidak menyentuh slot_sumber",
                     self.r.cmd("HLEN", self.k_slot_sumber()) == 1)

        # HGET slot_sumber tanpa entri → false; harus jatuh ke ARGV[7] (pool).
        self.reset()
        self.token(skrip="token_alokasi", request_id="r9", urutan=[("A", 2)])
        self.r.cmd("HDEL", self.k_slot_sumber(), "r9:1")
        h = self.tambah(member="r9:1", alokasi=True)
        self.periksa("tanpa slot_sumber: ok", h[0] == "ok", str(h))
        self.periksa("tanpa slot_sumber: beban ke sumber cadangan p1:1000",
                     self.r.cmd("HGET", self.k_anggaran(), "p1:1000") == "1")

    def uji_properti_anggaran(self, iterasi: int) -> None:
        """AC-7: u_x <= a_x, satu beban per percobaan, sumber pertama beruang."""
        print(f"properti anggaran: {iterasi} urutan acak (AC-7)")
        rng = random.Random(170)
        batas = {"A": 3, "B": 2, "sisa": 2}
        urutan = list(batas.items())
        lancar = 0
        for _ in range(iterasi):
            self.reset()
            n = 0
            for _ in range(12):
                aksi = rng.choice(["token", "token", "commit", "lepas"])
                if aksi == "token":
                    n += 1
                    member = f"r{n}:1"
                    u = self._beban()
                    harap = next((s for s, a in urutan if u.get(s, 0) < a), None)
                    h = self.token(skrip="token_alokasi", request_id=f"r{n}",
                                   urutan=urutan, maks=1000, kuota=10_000,
                                   jendela=((1, 10**9),))
                    if harap is None:
                        if h[0] != "anggaran_habis":
                            self.periksa("tanpa ruang: anggaran_habis", False, str(h))
                            return
                    elif h[0] != "ok" or self.r.cmd(
                            "HGET", self.k_slot_sumber(), member) != harap:
                        self.periksa("sumber dengan ruang pertama menang", False,
                                     f"harap={harap} {h}")
                        return
                elif aksi == "commit":
                    kandidat = self.r.cmd("HKEYS", self.k_slot_sumber()) or []
                    if not kandidat:
                        continue
                    member = rng.choice(kandidat)
                    self.tambah(member=member, alokasi=True, kuota=10_000)
                else:
                    slot = [m for m, _ in self.slot()]
                    if not slot:
                        continue
                    self.lepas(member=rng.choice(slot))

                u = self._beban()
                for s, a in urutan:
                    if u.get(s, 0) > a:
                        self.periksa(f"u_{s} <= a_{s}", False, f"{u} batas {batas}")
                        return
                if sum(u.values()) > sum(batas.values()):
                    self.periksa("Sigma u <= Sigma a", False, str(u))
                    return
                lancar += 1
        self.periksa(f"invarian anggaran {lancar} langkah", True)

    # -- kunci (D1.7) -----------------------------------------------------

    @staticmethod
    def k_attempts(pool: str = "p1", periode: int = 0) -> str:
        return f"pr:attempts:{{{pool}}}:{periode}"

    @staticmethod
    def k_ember(pool: str = "p1") -> str:
        return f"pr:ember:{{{pool}}}"

    @staticmethod
    def k_antre(pool: str = "p1") -> str:
        return f"pr:antre:{{{pool}}}"

    @staticmethod
    def k_slot(pool: str = "p1") -> str:
        return f"pr:slot:{{{pool}}}"

    @staticmethod
    def k_retry(pool: str = "p1") -> str:
        return f"pr:retry:{{{pool}}}"

    @staticmethod
    def k_rl(jenis: str, pool: str = "p1", sync: bool = False) -> str:
        akhiran = ":sync" if sync else ""
        return f"pr:rl:{{{pool}}}:{jenis}{akhiran}"

    @staticmethod
    def k_anggaran(pool: str = "p1") -> str:
        return f"pr:anggaran:{{{pool}}}"

    @staticmethod
    def k_slot_sumber(pool: str = "p1") -> str:
        return f"pr:slot_sumber:{{{pool}}}"

    @staticmethod
    def k_tolak(client: str = "c1") -> str:
        return f"pr:tolak:{{{client}}}"

    # -- pemanggil per skrip ----------------------------------------------

    def tolak(self, *, client="c1", run="run-1", t=1_000_000, burst=20):
        """Limiter penolakan per client (ADR-0002 rev 5 D6, L-03/L-04)."""
        return self.panggil("tolak", ["pr:siap", self.k_tolak(client)],
                            [run, str(t), str(burst)])

    def intake(self, *, pool="p1", run="run-1", mode="bergulir", sync=False,
               kuota=100, no_ember=1000, periode=0):
        keys = ["pr:siap", self.k_attempts(pool, periode), self.k_ember(pool),
                self.k_antre(pool)]
        argv = [run, mode, "1" if sync else "0", "klien-1", str(kuota),
                str(JUMLAH_EMBER), str(no_ember)]
        return self.panggil("intake", keys, argv)

    def token(self, *, pool="p1", run="run-1", request_id="req-1", nomor=1,
              mode="bergulir", sync=False, kuota=100, no_ember=1000,
              maks=8, ttl=80_000_000, rasio="0.1", retry_min=20, share="0.2",
              jendela=((200_000, 5),), periode=0, skrip="token", urutan=None):
        keys = ["pr:siap", self.k_attempts(pool, periode), self.k_ember(pool),
                self.k_slot(pool), self.k_retry(pool)]
        for i, _ in enumerate(jendela):
            keys.append(self.k_rl(f"w{i}", pool))
            keys.append(self.k_rl(f"w{i}", pool, sync=True))
        argv = [run, request_id, str(nomor), mode, "1" if sync else "0",
                str(kuota), str(JUMLAH_EMBER), str(no_ember), str(maks),
                str(ttl), str(rasio), str(retry_min), share, str(len(jendela))]
        for t, burst in jendela:
            argv += [str(t), str(burst)]
        if skrip == "token_alokasi":
            # `urutan` = daftar (sumber, batas) sesuai keputusan Rust; bawaannya
            # satu sumber longgar supaya uji pasangan F0 tidak tersentuh anggaran.
            if urutan is None:
                urutan = [(pool, 10**9)]
            keys += [self.k_anggaran(pool), self.k_slot_sumber(pool)]
            argv.append(str(len(urutan)))
            for sumber, batas in urutan:
                argv += [str(sumber), str(batas)]
            argv.append(str(MENIT_EMBER_DETIK))
        return self.panggil(skrip, keys, argv)

    def tambah(self, *, member="req-1:1", pool="p1", run="run-1",
               mode="bergulir", kuota=100, no_ember=1000, periode=0, alokasi=False):
        keys = ["pr:siap", self.k_attempts(pool, periode), self.k_ember(pool),
                self.k_slot(pool)]
        argv = [run, member, mode, str(kuota), str(JUMLAH_EMBER), str(no_ember)]
        if alokasi:
            keys += [self.k_anggaran(pool), self.k_slot_sumber(pool)]
            argv.append(pool)
        return self.panggil("tambah", keys, argv)

    def klaim(self, *, member="req-1:1", pool="p1", run="run-1",
              ttl=80_000_000, dari_antre=True):
        keys = ["pr:siap", self.k_slot(pool), self.k_antre(pool)]
        argv = [run, member, str(ttl), "1" if dari_antre else "0"]
        return self.panggil("klaim", keys, argv)

    def lepas(self, *, member="req-1:1", pool="p1", run="run-1", asal="kas"):
        keys = ["pr:siap", self.k_slot(pool), self.k_slot_sumber(pool)]
        return self.panggil("lepas", keys, [run, member, asal])

    def baca(self, *, pool="p1", run="run-1", mode="bergulir", kuota=100,
             no_ember=1000, periode=0):
        keys = ["pr:siap", self.k_attempts(pool, periode), self.k_ember(pool),
                self.k_slot(pool)]
        argv = [run, mode, str(kuota), str(JUMLAH_EMBER), str(no_ember)]
        return self.panggil("baca", keys, argv)

    # -- bantu ------------------------------------------------------------

    def isi_ember(self, fields: dict[int, int], pool="p1") -> None:
        for nomor, nilai in fields.items():
            self.r.cmd("HSET", self.k_ember(pool), f"b{nomor}", str(nilai))

    def isi_attempts(self, nilai: int, pool="p1", periode=0) -> None:
        self.r.cmd("SET", self.k_attempts(pool, periode), str(nilai))

    def jumlah_ember(self, *, pool="p1", no_ember=1000) -> int:
        data = self.r.cmd("HGETALL", self.k_ember(pool)) or []
        batas = no_ember - (JUMLAH_EMBER - 1)
        total = 0
        for i in range(0, len(data), 2):
            nama = data[i]
            if not nama.startswith("b"):
                continue
            nomor = int(nama[1:])
            if batas <= nomor <= no_ember:
                total += int(data[i + 1])
        return total

    def slot(self, pool="p1") -> list[tuple[str, int]]:
        data = self.r.cmd("ZRANGE", self.k_slot(pool), "0", "-1", "WITHSCORES") or []
        return [(data[i], int(data[i + 1])) for i in range(0, len(data), 2)]

    # -- uji --------------------------------------------------------------

    def uji_gerbang_siap(self) -> None:
        print("gerbang kesiapan menjawab `belum_siap` di semua skrip (§5a.2 butir 2)")
        for run, label in ((None, "tanpa pr:siap"), ("run-2", "run_id berbeda")):
            self.r.cmd("FLUSHALL")
            if run is not None:
                self.siapkan_run(run)
            hasil = {
                "intake": self.intake()[0],
                "token": self.token()[0],
                "klaim": self.klaim()[0],
                "lepas": self.lepas()[0],
                "tambah": self.tambah()[0],
                "baca": self.baca()[0],
            }
            self.periksa(
                f"{label}: semua skrip menolak",
                all(h == "belum_siap" for h in hasil.values()),
                str(hasil),
            )

        # `belum_siap` juga harus tanpa mutasi: tidak ada slot, counter, atau TAT
        # yang tertinggal dari pemanggilan yang ditolak.
        self.r.cmd("FLUSHALL")
        self.token()
        self.tambah()
        self.periksa("penolakan tidak menulis apa pun",
                     self.r.cmd("DBSIZE") == 0, str(self.r.cmd("DBSIZE")))

    def uji_penolakan(self) -> None:
        print("limiter penolakan per client: burst 20, 1 token/detik, 429 bila habis (L-03…L-07)")
        self.reset()
        hasil = [self.tolak() for _ in range(21)]
        kode = [h[0] for h in hasil]
        self.periksa("21 penolakan beruntun: 20 lolos, ke-21 dibatasi",
                     kode == ["lolos"] * 20 + ["dibatasi"], str(kode))
        self.periksa("sisa token turun dari 19 ke 0",
                     [h[1] for h in hasil[:20]] == list(range(19, -1, -1)),
                     str([h[1] for h in hasil[:20]]))
        tunggu = hasil[20][1]
        self.periksa("Retry-After kira-kira satu token (0 < tunggu <= 1 s)",
                     0 < tunggu <= 1_000_000, str(tunggu))

        # `dibatasi` tidak boleh menggeser TAT: client yang terus ditolak tidak
        # dihukum lebih lama dari satu token per T.
        tat = self.r.cmd("GET", self.k_tolak())
        self.tolak()
        self.tolak()
        self.periksa("penolakan yang dibatasi tidak menulis TAT",
                     self.r.cmd("GET", self.k_tolak()) == tat)
        self.periksa("TTL kunci > 0 (tidak abadi)",
                     int(self.r.cmd("PTTL", self.k_tolak())) > 0)

        # Isolasi: bucket client lain, dan bucket upstream pool, tidak tersentuh.
        self.periksa("client lain tetap punya bucket penuh",
                     self.tolak(client="c2")[:2] == ["lolos", 19])
        self.periksa("tidak ada kunci pool yang ditulis",
                     self.r.cmd("KEYS", "pr:rl:*") == [] and self.r.cmd("KEYS", "pr:slot:*") == [])

        # Pulih satu token tiap T: T kecil supaya uji cepat.
        self.reset()
        self.periksa("burst 2: dua lolos, ketiga dibatasi",
                     [self.tolak(t=200_000, burst=2)[0] for _ in range(3)]
                     == ["lolos", "lolos", "dibatasi"])
        time.sleep(0.25)
        self.periksa("setelah satu T, satu token pulih",
                     self.tolak(t=200_000, burst=2)[0] == "lolos")

        # Kunci yang hilang (failover) dibaca sebagai bucket penuh.
        self.r.cmd("DEL", self.k_tolak())
        self.periksa("kunci hilang = bucket penuh",
                     self.tolak()[:2] == ["lolos", 19])

        # Fail-closed (L-07) dan argumen tidak sah.
        self.r.cmd("FLUSHALL")
        self.periksa("tanpa pr:siap: belum_siap", self.tolak()[0] == "belum_siap")
        self.siapkan_run("run-1")
        self.periksa("run_id berbeda: belum_siap", self.tolak(run="run-2")[0] == "belum_siap")
        self.periksa("T nol ditolak sebagai galat", self.tolak(t=0)[0] == "t_tidak_sah")
        self.periksa("burst nol ditolak sebagai galat", self.tolak(burst=0)[0] == "burst_tidak_sah")
        self.periksa("T bukan bilangan bulat ditolak", self.tolak(t="1.5")[0] == "t_tidak_sah")
        self.periksa("penolakan dan galat tidak menulis kunci client",
                     self.r.cmd("EXISTS", self.k_tolak()) == 0)

        # Konkurensi: 8 koneksi berebut bucket 20; tepat 20 yang lolos.
        self.reset()
        lolos = []
        kunci = threading.Lock()

        def kerja():
            r = Redis(*self.alamat)
            for _ in range(10):
                h = r.evalsha(self.skrip["tolak"].sha, ["pr:siap", self.k_tolak("ck")],
                              ["run-1", "60000000", "20"])
                with kunci:
                    lolos.append(h[0])

        benang = [threading.Thread(target=kerja) for _ in range(8)]
        for b in benang:
            b.start()
        for b in benang:
            b.join()
        self.periksa("80 penolakan serentak: tepat 20 lolos",
                     lolos.count("lolos") == 20 and lolos.count("dibatasi") == 60,
                     f"lolos={lolos.count('lolos')} dibatasi={lolos.count('dibatasi')}")

    def uji_argumen_tidak_sah(self) -> None:
        print("argumen tidak sah dilaporkan sebagai galat, bukan sebagai penolakan")
        self.reset()
        kasus = [
            ("intake", ["mode", "harian"], "mode_tidak_sah"),
            ("intake", ["kuota", "0"], "kuota_efektif_tidak_sah"),
            ("intake", ["kuota", "1.5"], "kuota_efektif_tidak_sah"),
            ("intake", ["no_ember", "b100"], "no_ember_tidak_sah"),
            ("intake", ["no_ember", "-2"], "no_ember_tidak_sah"),
            ("token", ["nomor", 0], "nomor_percobaan_tidak_sah"),
            ("token", ["maks", 0], "maks_konkurensi_tidak_sah"),
            ("token", ["ttl", 0], "ttl_slot_tidak_sah"),
            ("token", ["rasio", "1.5"], "rasio_retry_tidak_sah"),
            ("token", ["rasio", "-0.1"], "rasio_retry_tidak_sah"),
            ("token", ["rasio", "abc"], "rasio_retry_tidak_sah"),
            ("token", ["retry_min", -1], "retry_min_tidak_sah"),
            ("token", ["retry_min", "2.5"], "retry_min_tidak_sah"),
            ("token", ["share", "1.5"], "porsi_sync_tidak_sah"),
            ("token", ["jendela", ((200_000, 0),)], "jendela_tidak_sah"),
            ("tambah", ["member", ""], "member_tidak_sah"),
            ("baca", ["mode", "harian"], "mode_tidak_sah"),
            ("klaim", ["ttl", 0], "ttl_slot_tidak_sah"),
        ]
        for skrip, (field, nilai), harap in kasus:
            hasil = getattr(self, skrip)(**{field: nilai})[0]
            self.periksa(f"{skrip}.{field}={nilai!r}", hasil == harap,
                         f"harap {harap}, dapat {hasil}")

    def uji_batas_kuota(self) -> None:
        print("kuota habis = terpakai + slot aktif >= kuota_efektif (§5a.1 butir 3)")
        for mode in ("bergulir", "kalender"):
            self.reset()
            kuota = 5
            for i in range(kuota):
                h = self.token(nomor=1, request_id=f"req-{i}", mode=mode, kuota=kuota)
                self.tambah(member=f"req-{i}:1", mode=mode, kuota=kuota)
                self.lepas(member=f"req-{i}:1")
                self.periksa(f"{mode}: token ke-{i + 1} lolos", h[0] == "ok", str(h))
            h = self.token(request_id="req-lewat", mode=mode, kuota=kuota)
            self.periksa(f"{mode}: token ke-{kuota + 1} kuota_habis",
                         h[0] == "kuota_habis", str(h))
            self.periksa(f"{mode}: laporan terpakai = {kuota}", h[1] == kuota, str(h))

        # Slot yang belum dilepas menahan kapasitas walau `attempts` masih 0.
        self.reset()
        self.token(kuota=1)
        h = self.token(request_id="req-2", kuota=1)
        self.periksa("slot aktif ikut dihitung", h[0] == "kuota_habis", str(h))

    def uji_antrean(self) -> None:
        print("antre async: INCR di intake, DECR di klaim, cek `terpakai + antre`")
        self.reset()
        kuota = 3
        h1 = self.intake(kuota=kuota)
        h2 = self.intake(kuota=kuota)
        h3 = self.intake(kuota=kuota)
        self.periksa("tiga intake async lolos",
                     all(h[0] == "lolos" for h in (h1, h2, h3)), str((h1, h2, h3)))
        self.periksa("antre = 3", self.r.cmd("GET", self.k_antre()) == "3")
        h4 = self.intake(kuota=kuota)
        self.periksa("intake keempat antrean_penuh",
                     h4[0] == "antrean_penuh", str(h4))
        self.periksa("sync ditolak saat antre menghabiskan kuota",
                     self.intake(kuota=kuota, sync=True)[0] == "kuota_habis"
                     and self.r.cmd("GET", self.k_antre()) == "3")

        self.token(kuota=kuota)
        self.periksa("klaim mengeluarkan satu dari antre",
                     self.klaim()[0] == "ok"
                     and self.r.cmd("GET", self.k_antre()) == "2")
        self.periksa("slot bisa diterima lagi sesudah satu keluar",
                     self.intake(kuota=kuota)[0] == "lolos")
        self.periksa("antre tidak bisa negatif",
                     self.klaim(member="tidak-ada")[0] == "slot_tidak_ada")

    def uji_kalender_periode(self) -> None:
        print("mode kalender: terpakai dari counter periode, bukan ember")
        self.reset()
        self.isi_attempts(4)
        self.isi_ember({1000: 99})
        h = self.intake(mode="kalender", kuota=5)
        self.periksa("intake kalender memakai counter periode", h == ["lolos", 4, 1], str(h))
        self.periksa("token kalender lolos", self.token(mode="kalender", kuota=5)[0] == "ok")
        h = self.tambah(mode="kalender", kuota=5)
        self.periksa("tambah kalender menaikkan counter periode", h[1:] == [5, 5, 5], str(h))
        h = self.token(request_id="req-2", mode="kalender", kuota=5)
        self.periksa("token berikutnya kuota_habis", h[0] == "kuota_habis", str(h))
        self.periksa("ember tidak dihitung di mode kalender",
                     self.baca(mode="kalender", kuota=5)[1] == 5,
                     str(self.baca(mode="kalender", kuota=5)))

    def uji_jendela_ember(self) -> None:
        print("jendela bergulir: 145 ember bernomor absolut, dipangkas di tambah (D2.8/D2.9)")
        self.reset()
        no_ember = 1100
        batas = no_ember - (JUMLAH_EMBER - 1)
        self.isi_ember({batas - 1: 7, batas: 3, no_ember: 2, 1000: 4})
        h = self.intake(no_ember=no_ember)
        self.periksa("ember tepat di batas dihitung, satu di luarnya tidak",
                     h == ["lolos", 9, 1], f"harap 9, dapat {h}")
        h = self.baca(no_ember=no_ember)
        self.periksa("baca memakai jendela yang sama", h[1] == 9, str(h))

        self.token(no_ember=no_ember)
        h = self.tambah(no_ember=no_ember)
        self.periksa("tambah menaikkan ember berjalan", h[1:] == [1, 10, 100], str(h))
        sisa = self.r.cmd("HGETALL", self.k_ember()) or []
        nama = {sisa[i]: sisa[i + 1] for i in range(0, len(sisa), 2)}
        self.periksa("field di luar jendela dipangkas",
                     f"b{batas - 1}" not in nama, str(sorted(nama)))
        self.periksa("field di batas dipertahankan", f"b{batas}" in nama, str(sorted(nama)))
        self.periksa("tidak ada field `terpakai` di hash",
                     "terpakai" not in nama, str(sorted(nama)))

    def uji_gcra(self) -> None:
        print("GCRA: bucket penuh melaporkan burst, TAT ber-TTL, dua jendela (D2)")
        self.reset()
        h = self.token(jendela=((200_000, 5),), kuota=100)
        self.periksa("isi bucket penuh = burst", h[0] == "ok" and h[5] == 5, str(h))
        h = self.token(request_id="req-2", jendela=((200_000, 5),), kuota=100)
        self.periksa("isi sesudah satu token = burst − 1", h[5] == 4, f"harap 4, dapat {h}")

        # 1/s burst 2: dua panggilan lolos. Ini kasus yang dulu merah karena
        # harness lupa `reset()` sehingga TAT 5/s tersisa (keputusan Shu D2.2).
        self.reset()
        jendela = ((1_000_000, 2),)
        h1 = self.token(jendela=jendela)
        h2 = self.token(request_id="req-2", jendela=jendela)
        h3 = self.token(request_id="req-3", jendela=jendela)
        self.periksa("1/s burst 2: panggilan pertama lolos", h1[0] == "ok", str(h1))
        self.periksa("1/s burst 2: panggilan kedua lolos", h2[0] == "ok", str(h2))
        self.periksa("1/s burst 2: panggilan ketiga rate_limit", h3[0] == "rate_limit", str(h3))
        self.periksa("tunggu_mikro dari TAT yang menolak",
                     0 < h3[4] <= 1_000_000, str(h3))

        pttl = self.r.cmd("PTTL", self.k_rl("w0"))
        self.periksa("TTL TAT = TAT_baru − now + 60 s",
                     61_000 <= pttl <= 63_500, f"pttl={pttl}")

        # Dua jendela: yang paling ketat menentukan.
        self.reset()
        jendela = ((200_000, 5), (5_000_000, 2))
        h = self.token(jendela=jendela, kuota=100)
        h2 = self.token(request_id="req-2", jendela=jendela, kuota=100)
        h3 = self.token(request_id="req-3", jendela=jendela, kuota=100)
        self.periksa("dua jendela: dua token pertama lolos (burst jendela kedua 2)",
                     h[0] == "ok" and h2[0] == "ok", str((h, h2)))
        self.periksa("jendela kedua menolak walau jendela pertama masih punya token",
                     h3[0] == "rate_limit", str(h3))
        self.periksa("tunggu_mikro memakai jendela terketat",
                     h3[4] > 4_000_000, f"tunggu={h3[4]}")

    def uji_porsi_sync(self) -> None:
        print("porsi sync: burst × share, dan share 0 menolak sync (§5a.1 butir 4)")
        self.reset()
        jendela = ((200_000, 5),)
        h = self.token(sync=True, share="0.2", jendela=jendela, kuota=100)
        self.periksa("sync pertama lolos", h[0] == "ok", str(h))
        self.periksa("porsi sync melaporkan 1 token", h[6] == 1, str(h))
        h = self.token(request_id="req-2", sync=True, share="0.2", jendela=jendela, kuota=100)
        self.periksa("sync kedua rate_limit pada porsi sendiri",
                     h[0] == "rate_limit", str(h))
        self.periksa("async masih punya token global",
                     self.token(request_id="req-3", jendela=jendela, kuota=100)[0] == "ok")

        self.reset()
        h = self.token(sync=True, share="0", jendela=jendela, kuota=100)
        self.periksa("share 0: porsi_sync_habis", h[0] == "porsi_sync_habis", str(h))

    def uji_retry_budget(self) -> None:
        print("anggaran retry: jendela 1 jam, retry_jendela < max(retry_min, rasio × percobaan) (§5a.3)")
        luas = dict(kuota=1_000_000, maks=1000, jendela=((1, 1_000_000),))

        def retry(i, nomor=2, **kw):
            return self.token(request_id=f"rt-{i}", nomor=nomor, **luas, **kw)

        # retry_min dominan: rasio 0, tiga retry masuk, keempat ditolak.
        self.reset()
        hasil = [retry(i, rasio="0", retry_min=3)[0] for i in range(5)]
        self.periksa("retry_min 3, rasio 0: tiga lolos lalu retry_budget_habis",
                     hasil == ["ok", "ok", "ok", "retry_budget_habis", "retry_budget_habis"],
                     str(hasil))
        self.periksa("percobaan pertama tidak memakai anggaran",
                     self.token(request_id="baru-1", nomor=1, rasio="0", retry_min=3, **luas)[0] == "ok")

        # Tanpa nilai "tanpa anggaran": 0 dan 0 berarti tidak ada retry sama sekali.
        self.reset()
        self.periksa("rasio 0 dan retry_min 0: retry ditolak",
                     retry(0, rasio="0", retry_min=0)[0] == "retry_budget_habis")
        self.periksa("... tetapi percobaan pertama lolos",
                     self.token(request_id="p-1", nomor=1, rasio="0", retry_min=0, **luas)[0] == "ok")

        # Rasio dominan: 10 percobaan pertama, rasio 0,5 -> retry mengisi sampai
        # retry_jendela == percobaan_jendela / 2 (retry ikut dihitung sebagai percobaan).
        self.reset()
        for i in range(10):
            self.token(request_id=f"a-{i}", nomor=1, rasio="0.5", retry_min=0, **luas)
        hasil = [retry(i, rasio="0.5", retry_min=0)[0] for i in range(14)]
        self.periksa("rasio 0,5 dari 10 percobaan: tepat 10 retry lolos, sisanya ditolak",
                     hasil == ["ok"] * 10 + ["retry_budget_habis"] * 4, str(hasil))

        # Penolakan murni: tidak menulis hash anggaran.
        sebelum = self.r.cmd("HGETALL", self.k_retry())
        retry(99, rasio="0.5", retry_min=0)
        self.periksa("retry_budget_habis tidak menulis anggaran",
                     self.r.cmd("HGETALL", self.k_retry()) == sebelum)

        # Jendela bergulir 60 menit: bucket yang lebih tua dari 60 menit tidak
        # dihitung dan dipangkas saat pemberian token berikutnya.
        self.reset()
        menit = self.mikrodetik() // 60_000_000
        self.r.cmd("HSET", self.k_retry(), f"p{menit - 70}", "999", f"r{menit - 70}", "999",
                   f"p{menit - 59}", "1", f"r{menit - 59}", "1")
        self.periksa("bucket menit ke-71 tidak dihitung; ke-60 dihitung (retry_min 2 -> 1 slot sisa)",
                     retry(0, rasio="0", retry_min=2)[0] == "ok")
        self.periksa("retry kedua: r(ke-60)=1 + 1 = 2 >= 2 -> ditolak",
                     retry(1, rasio="0", retry_min=2)[0] == "retry_budget_habis")
        kunci = self.r.cmd("HKEYS", self.k_retry())
        self.periksa("bucket lama dipangkas",
                     f"p{menit - 70}" not in kunci and f"r{menit - 70}" not in kunci, str(kunci))
        self.periksa("hash anggaran punya TTL",
                     int(self.r.cmd("PTTL", self.k_retry())) > 0)

        # Retry ikut menghitung sebagai percobaan, percobaan pertama tidak ikut sebagai retry.
        self.reset()
        self.token(request_id="s-0", nomor=1, **luas)
        self.token(request_id="s-1", nomor=2, **luas)
        menit = self.mikrodetik() // 60_000_000
        isi = self.r.cmd("HGETALL", self.k_retry())
        peta = dict(zip(isi[0::2], isi[1::2]))
        total_p = sum(int(v) for k, v in peta.items() if k.startswith("p"))
        total_r = sum(int(v) for k, v in peta.items() if k.startswith("r"))
        self.periksa("dua token: percobaan_jendela 2, retry_jendela 1",
                     (total_p, total_r) == (2, 1), str(peta))

    def uji_retry_after(self) -> None:
        print("Retry-After bergulir dari ember tertua di jendela (D2.10)")
        self.reset()
        detik, _ = self.r.cmd("TIME")
        no_ember = int(detik) // MENIT_EMBER_DETIK
        tertua = no_ember - 100
        self.isi_ember({tertua: 1, no_ember: 1})
        h = self.baca(kuota=2, no_ember=no_ember)
        harap = (tertua + JUMLAH_EMBER) * MENIT_EMBER_DETIK - int(detik)
        self.periksa("baca melaporkan kuota_habis", h[1] == 2, str(h))
        self.periksa("Retry-After = (b_tertua + 145) × 600 − now",
                     abs(h[4] - harap) <= 2, f"harap {harap}, dapat {h[4]}")
        self.periksa("slot tidak ikut menahan retry_after",
                     h[3] == 0, str(h))

        self.reset()
        self.isi_attempts(2)
        h = self.baca(mode="kalender", kuota=2)
        self.periksa("mode kalender: Retry-After dihitung pemanggil (0)",
                     h[1] == 2 and h[4] == 0, str(h))

        self.reset()
        detik, _ = self.r.cmd("TIME")
        no_ember = int(detik) // MENIT_EMBER_DETIK
        self.isi_ember({no_ember: 1})
        h = self.baca(kuota=1, no_ember=no_ember)
        self.periksa("satu ember berjalan: Retry-After ≈ 24 jam 10 menit",
                     86_000 < h[4] <= 87_000, str(h))

    def uji_baca_tidak_mutasi(self) -> None:
        print("kuota_baca murni baca: slot kedaluwarsa & field tua tidak dibersihkan")
        self.reset()
        no_ember = 1000
        self.isi_ember({1: 5, no_ember: 2})
        self.r.cmd("ZADD", self.k_slot(), self.mikrodetik() - 1_000_000, "lama:1")
        sebelum = (
            self.r.cmd("HGETALL", self.k_ember()),
            self.r.cmd("ZRANGE", self.k_slot(), "0", "-1", "WITHSCORES"),
            self.r.cmd("GET", self.k_attempts()),
            self.r.cmd("DBSIZE"),
        )
        h = self.baca(no_ember=no_ember, kuota=5)
        sesudah = (
            self.r.cmd("HGETALL", self.k_ember()),
            self.r.cmd("ZRANGE", self.k_slot(), "0", "-1", "WITHSCORES"),
            self.r.cmd("GET", self.k_attempts()),
            self.r.cmd("DBSIZE"),
        )
        self.periksa("baca tidak mengubah apa pun", sebelum == sesudah,
                     f"{sebelum} != {sesudah}")
        self.periksa("slot kedaluwarsa tidak dihitung aktif", h[3] == 0, str(h))

    def uji_slot_bocor(self) -> None:
        print("D1.9(i): slot yang tidak dilepas menahan kapasitas sampai TTL-nya lewat")
        self.reset()
        kuota = 1
        ttl = 200_000
        h = self.token(kuota=kuota, ttl=ttl)
        self.periksa("slot dipesan", h[0] == "ok" and h[2] == 1, str(h))
        h = self.token(request_id="req-2", kuota=kuota, ttl=ttl)
        self.periksa("pemberian berikutnya menghitung slot itu",
                     h[0] == "kuota_habis" and h[2] == 1, str(h))
        time.sleep(0.25)
        h = self.token(request_id="req-2", kuota=kuota, ttl=ttl)
        self.periksa("sesudah TTL slot dibersihkan dan token lolos", h[0] == "ok", str(h))
        self.periksa("jumlah kedaluwarsa yang dilaporkan = 1", h[3] == 1, str(h))
        self.periksa("slot lama hilang dari semaphore",
                     [m for m, _ in self.slot()] == ["req-2:1"], str(self.slot()))

    def uji_hitung_dua_kali(self) -> None:
        print("percobaan dihitung dua kali antara attempts_tambah dan slot_lepas (disengaja)")
        self.reset()
        kuota = 10
        t = self.token(request_id="r1", kuota=kuota)
        self.periksa("token diberikan", t[0] == "ok", str(t))
        antara = self.baca(kuota=kuota)
        self.periksa("sebelum INCR: terpakai 0, slot 1",
                     antara[1] == 0 and antara[3] == 1, str(antara))
        self.tambah(member="r1:1", kuota=kuota)
        dua = self.baca(kuota=kuota)
        self.periksa("sesudah INCR, sebelum lepas: terpakai 1 dan slot 1 (dua kali)",
                     dua[1] == 1 and dua[3] == 1, str(dua))
        self.lepas(member="r1:1")
        satu = self.baca(kuota=kuota)
        self.periksa("sesudah lepas: terpakai 1, slot 0",
                     satu[1] == 1 and satu[3] == 0, str(satu))
        # Sisi konservatif: pada kuota 2, `terpakai + slot` = 2 sudah penuh
        # walau baru satu percobaan yang terjadi.
        self.reset()
        self.token(request_id="a", kuota=2)
        self.tambah(member="a:1", kuota=2)
        penuh = self.token(request_id="b", kuota=2)
        self.periksa("hitungan ganda menahan token berikutnya (konservatif)",
                     penuh[0] == "kuota_habis", str(penuh))

    def uji_slot_kedaluwarsa_tambah(self) -> None:
        print("D1.9(ii): proses macet yang lanjut sesudah TTL ditolak, `attempts` tetap")
        self.reset()
        h = self.token(request_id="req-macet", ttl=200_000, kuota=10)
        self.periksa("slot dipesan", h[0] == "ok", str(h))
        time.sleep(0.25)
        h = self.tambah(member="req-macet:1", kuota=10)
        self.periksa("attempts_tambah menjawab slot_kedaluwarsa",
                     h == ["slot_kedaluwarsa", 0], str(h))
        self.periksa("`attempts` tidak berubah",
                     self.r.cmd("GET", self.k_attempts()) is None)
        self.periksa("ember tidak bertambah", self.jumlah_ember() == 0)
        self.periksa("baca melaporkan terpakai 0", self.baca()[1] == 0)

    def uji_rebuild_slot(self) -> None:
        print("D1.9(iii): rebuild dari PostgreSQL memakai member yang sama")
        self.reset()
        ttl = 80_000_000
        now = self.mikrodetik()
        # Dua request yang di PostgreSQL berstatus `dispatched`/`in_flight`.
        for member in ("disp-1:1", "kerja-2:1"):
            self.r.cmd("ZADD", self.k_slot(), now + ttl, member)
        h = self.klaim(member="kerja-2:1", ttl=ttl, dari_antre=False)
        self.periksa("pemilik bisa memperpanjang member hasil rebuild",
                     h[0] == "ok", str(h))
        h = self.tambah(member="disp-1:1", kuota=10)
        self.periksa("member hasil rebuild bisa dipakai menaikkan attempts",
                     h[0] == "ok" and h[1] == 1, str(h))

        # Slot yatim tanpa jejak di PostgreSQL tidak dibangun ulang, jadi
        # klaim dan penambahan percobaannya ditolak.
        self.periksa("slot yatim tidak ada di semaphore",
                     "yatim-9:1" not in [m for m, _ in self.slot()])
        self.periksa("klaim slot yatim ditolak",
                     self.klaim(member="yatim-9:1")[0] == "slot_tidak_ada")
        self.periksa("percobaan slot yatim ditolak",
                     self.tambah(member="yatim-9:1", kuota=10)[0] == "slot_kedaluwarsa")

        # TAT yang ditulis rebuild = now + τ, jadi tidak ada burst sesudah
        # failover: hanya satu token, lalu laju normal.
        t, burst = 1_000_000, 3
        tau = (burst - 1) * t
        self.r.cmd("SET", self.k_rl("w0"), str(self.mikrodetik() + tau))
        jendela = ((t, burst),)
        h1 = self.token(request_id="pasca-1", jendela=jendela, kuota=100)
        h2 = self.token(request_id="pasca-2", jendela=jendela, kuota=100)
        self.periksa("rebuild TAT: satu token lolos", h1[0] == "ok", str(h1))
        self.periksa("rebuild TAT: tidak ada burst sesudahnya",
                     h2[0] == "rate_limit", str(h2))

    def uji_properti_m02(self, iterasi: int) -> None:
        print(f"property M-02 (berurutan) lewat jalur token: {iterasi} iterasi")
        acak = random.Random(20261103)
        for i in range(iterasi):
            mode = acak.choice(["bergulir", "kalender"])
            kuota = acak.randint(1, 5)
            awal = acak.randint(0, kuota - 1)
            self.reset()
            no_ember = 1000
            # `attempts` dan ember diisi bersama: keduanya cacah percobaan yang
            # sama, jadi invarian `attempts == awal + panggilan` bisa diuji.
            if mode == "kalender":
                self.isi_attempts(awal)
            elif awal:
                self.isi_attempts(awal)
                self.isi_ember({no_ember: awal})
            panggilan = 0
            attempts = awal
            for langkah in range(6):
                rid = f"r{i}-{langkah}"
                h = self.token(request_id=rid, mode=mode, kuota=kuota,
                               no_ember=no_ember)
                if h[0] != "ok":
                    continue
                if acak.random() < 0.7:
                    t = self.tambah(member=f"{rid}:1", mode=mode, kuota=kuota,
                                    no_ember=no_ember)
                    if t[0] in ("ok", "ok_terlampaui"):
                        panggilan += 1
                        attempts = t[1]
                    self.lepas(member=f"{rid}:1")
                else:
                    self.lepas(member=f"{rid}:1")
            total_ember = self.jumlah_ember(no_ember=no_ember)
            self.periksa(
                f"iterasi {i} ({mode}, kuota {kuota}, awal {awal})",
                attempts == awal + panggilan
                and attempts <= kuota
                and (mode == "kalender" or total_ember == attempts),
                f"panggilan={panggilan} attempts={attempts} ember={total_ember}",
            )

    # -- konkurensi -------------------------------------------------------

    def _pekerja(self, alamat, skrip, mode, kuota, no_ember, awal, seed,
                 semua_jalur, per_klien, hasil):
        acak = random.Random(seed)
        r = Redis(*alamat)
        u = Uji.__new__(Uji)
        u.r = r
        u.skrip = skrip
        panggilan = 0
        token_ok = 0
        galat = []
        # Jalur mati memakai TTL pendek supaya pembersihan slot ikut
        # berlomba di dalam uji, bukan hanya di uji D1.9 yang berurutan.
        ttl = 5_000 if semua_jalur else 80_000_000
        try:
            for i in range(per_klien):
                rid = f"k{seed}-{i}"
                jalur = acak.choices(["a", "b", "c", "d"], [0.6, 0.15, 0.15, 0.1])[0]
                if not semua_jalur:
                    jalur = "a" if jalur in ("a", "b") else "b"
                h = u.token(request_id=rid, mode=mode, kuota=kuota,
                            no_ember=no_ember, ttl=ttl,
                            jendela=((1, 1_000_000),))
                if h[0] != "ok":
                    continue
                token_ok += 1
                if jalur == "c":
                    # dispatcher mati sesudah memesan slot, sebelum memanggil:
                    # tidak ada log, tidak ada attempts, slot menunggu TTL.
                    continue
                if jalur == "b":
                    # panggilan upstream gagal sebelum baris log: slot dilepas
                    # dispatcher, tidak ada percobaan yang tercatat.
                    u.lepas(member=f"{rid}:1", asal="dispatcher")
                    continue
                t = u.tambah(member=f"{rid}:1", mode=mode, kuota=kuota,
                             no_ember=no_ember)
                if t[0] in ("ok", "ok_terlampaui"):
                    panggilan += 1
                if jalur == "d":
                    # worker mati sesudah commit log, sebelum melepas slot.
                    continue
                u.lepas(member=f"{rid}:1", asal="kas")
        except Exception as e:  # noqa: BLE001 - dilaporkan ke uji
            galat.append(repr(e))
        finally:
            r.close()
        hasil.append((panggilan, token_ok, galat))

    def uji_konkuren(self, klien: int = 8, per_klien: int = 50, kuota: int = 37) -> None:
        """Gerbang M-02 di jalur token, tiga konfigurasi, empat jalur (D1).

        Asersinya TANPA toleransi (D1 butir 1): `panggilan == attempts` dan
        `panggilan <= kuota − terpakai_awal`, termasuk saat proses mati dan
        slot kedaluwarsa. Asersi pemanfaatan hanya dijalankan pada varian tanpa
        jalur mati, karena slot yatim memang menahan kapasitas sampai TTL.
        """
        konfigurasi = [
            ("bergulir", "bergulir", 0),
            ("kalender", "kalender", 5),
            ("kalender transisi", "kalender", 5),
        ]
        for label, mode, awal in konfigurasi:
            for semua_jalur in (True, False):
                self.reset()
                no_ember = 1000
                if label == "kalender transisi":
                    # Ember dari periode sebelumnya masih berisi; mode kalender
                    # harus mengabaikannya (K-11).
                    self.isi_ember({no_ember - 1: 40, no_ember: 4})
                if mode == "kalender":
                    self.isi_attempts(awal)
                elif awal:
                    self.isi_ember({no_ember: awal})
                hasil: list = []
                benang = [
                    threading.Thread(
                        target=self._pekerja,
                        args=(self.alamat, self.skrip, mode, kuota, no_ember,
                              awal, 100 + i, semua_jalur, per_klien, hasil),
                    )
                    for i in range(klien)
                ]
                mulai = time.time()
                for b in benang:
                    b.start()
                for b in benang:
                    b.join()
                lama = time.time() - mulai
                galat = [g for _, _, gs in hasil for g in gs]
                panggilan = sum(p for p, _, _ in hasil)
                attempts = int(self.r.cmd("GET", self.k_attempts()) or 0)
                ember = self.jumlah_ember(no_ember=no_ember)
                batas = kuota - awal
                judul = f"{label}/{'4 jalur' if semua_jalur else 'tanpa mati'}"
                self.periksa(f"{judul}: tanpa galat skrip", not galat, str(galat[:3]))
                self.periksa(f"{judul}: attempts == awal + panggilan",
                             attempts == awal + panggilan,
                             f"awal={awal} panggilan={panggilan} attempts={attempts}")
                self.periksa(f"{judul}: panggilan <= {batas}",
                             panggilan <= batas, f"panggilan={panggilan} kuota={batas}")
                if mode == "bergulir":
                    self.periksa(f"{judul}: ember selaras dengan attempts",
                                 ember == attempts, f"ember={ember} attempts={attempts}")
                if not semua_jalur:
                    self.periksa(
                        f"{judul}: pemanfaatan >= {batas} − {klien}",
                        panggilan >= batas - klien,
                        f"panggilan={panggilan} target={batas}",
                    )
                print(f"       {judul}: {panggilan} panggilan, {batas} kapasitas, {lama:.2f} s")
        self.periksa(f"konkurensi {klien}×{per_klien}", True)

    def jalankan_semua(self, iterasi: int) -> int:
        self.uji_alokasi_bagian()
        self.uji_alokasi_tidak_sah()
        self.uji_alokasi_batas_rute()
        self.uji_pasangan_alokasi()
        self.uji_hook_alokasi()
        self.uji_anggaran_pilih()
        self.uji_anggaran_jendela()
        self.uji_anggaran_beban()
        self.uji_properti_anggaran(iterasi)
        self.uji_gerbang_siap()
        self.uji_penolakan()
        self.uji_argumen_tidak_sah()
        self.uji_batas_kuota()
        self.uji_antrean()
        self.uji_kalender_periode()
        self.uji_jendela_ember()
        self.uji_gcra()
        self.uji_porsi_sync()
        self.uji_retry_budget()
        self.uji_retry_after()
        self.uji_baca_tidak_mutasi()
        self.uji_slot_bocor()
        self.uji_slot_kedaluwarsa_tambah()
        self.uji_hitung_dua_kali()
        self.uji_rebuild_slot()
        self.uji_properti_m02(iterasi)
        self.uji_konkuren()
        print(f"\n{self.lolos} lolos, {self.gagal} gagal")
        return 0 if self.gagal == 0 else 1


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, default=6395)
    p.add_argument("--iterasi", type=int, default=300)
    a = p.parse_args()
    r = Redis(a.host, a.port)
    r.cmd("PING")
    return Uji(r).jalankan_semua(a.iterasi)


if __name__ == "__main__":
    sys.exit(main())
