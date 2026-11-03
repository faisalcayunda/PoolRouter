#!/usr/bin/env python3
"""Harness uji skrip Lua kuota terhadap Redis sungguhan (SHAA-195).

Tidak mengompilasi Rust. Memuat berkas `.lua` dari `src/kuota/lua/` ke Redis
lewat `redis-cli --eval`-style (SCRIPT LOAD + EVALSHA), lalu membuktikan
invarian ADR-0001 rev 15 §5a.1 dan property M-02 (§9).

Pakai:
    python3 tools/uji_lua.py [--host 127.0.0.1] [--port 6395] [--iterasi 300]

Exit code 0 = semua uji lolos; 1 = ada kegagalan.
"""

from __future__ import annotations

import argparse
import random
import socket
import sys

HERE = __import__("pathlib").Path(__file__).resolve().parent
# Skrip `.lua` berada di direktori yang sama dengan harness ini.
LUA_DIR = HERE

HARI = 24 * 3600
JEDA_EMBER = 600
JUMLAH_EMBER = 145


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
    """Skrip Lua + berkas pembantunya dimuat sebagai satu chunk (`dofile`)."""

    def __init__(self, redis: Redis, nama: str):
        nama_pembantu = {"siap.lua": "siap", "bersama.lua": "bersama"}
        inti = (LUA_DIR / nama).read_text()
        awal = ""
        for pembantu, alias in nama_pembantu.items():
            if pembantu != nama and f"dofile('{pembantu}')" in inti:
                isi = (LUA_DIR / pembantu).read_text()
                # Pembantu mengembalikan tabel di akhir berkas. `return` di
                # tingkat atas mengakhiri chunk, jadi tabelnya diikat ke nama
                # pembantu lebih dulu supaya `end)()` dibaca sebagai penutup.
                awal += f"local {alias} = (function()\n{isi}\nend)()\n"
                # Variabelnya lokal di skrip; nama berkas bertitik tidak sah
                # sebagai nama variabel Lua, jadi `dofile('siap.lua')` diganti
                # nama berkasnya sendiri tanpa kutip.
                inti = inti.replace(f"dofile('{pembantu}')", pembantu.split(".")[0])
        self.sha = redis.cmd("SCRIPT", "LOAD", awal + inti)
        self.sumber = awal + inti

    @staticmethod
    def _pakai(nama: str, pembantu: str) -> bool:
        return f"dofile('{pembantu}')" in (LUA_DIR / nama).read_text()


class Uji:
    def __init__(self, redis: Redis):
        self.r = redis
        self.skrip = {
            n: Skrip(redis, f)
            for n, f in [
                ("intake", "kuota_intake.lua"),
                ("token", "rate_limit_ambil.lua"),
                ("tambah", "attempts_tambah.lua"),
                ("baca", "kuota_retry_after.lua"),
            ]
        }
        self.gagal = 0
        self.lolos = 0

    # --- util -------------------------------------------------------------
    def periksa(self, nama: str, kondisi: bool, pesan: str = "") -> None:
        if kondisi:
            self.lolos += 1
        else:
            self.gagal += 1
            print(f"  GAGAL {nama}: {pesan}")

    def reset(self) -> None:
        self.r.cmd("FLUSHALL")
        self.siapkan_run("run-1")

    def siapkan_run(self, run_id: str) -> None:
        self.r.cmd("SET", "pr:siap", run_id)

    def panggil(self, skrip: str, keys: list[str], argv: list[str]):
        return self.r.evalsha(self.skrip[skrip].sha, keys, argv)

    def tambah(self, pool="p1", run="run-1", kuota=100, ember_mulai=100, mode="bergulir",
               periode="2026-11-03"):
        return self.panggil(
            "tambah",
            [],
            [
                run,
                mode,
                f"pr:attempts:{pool}:{periode}",
                f"pr:antre:{pool}",
                str(kuota),
                f"pr:ember:{pool}:{periode}",
                str(JUMLAH_EMBER),
                str(ember_mulai),
            ],
        )

    def intake(self, pool="p1", run="run-1", kuota=100, ember_mulai=100, mode="bergulir",
               sync=False, periode="2026-11-03", client="c1"):
        return self.panggil(
            "intake",
            [],
            [
                client,
                run,
                mode,
                "1" if sync else "0",
                f"pr:attempts:{pool}:{periode}",
                f"pr:antre:{pool}",
                str(kuota),
                f"pr:ember:{pool}:{periode}",
                str(JUMLAH_EMBER),
                str(ember_mulai),
            ],
        )

    def token(self, pool="p1", run="run-1", sync=False, share="0.2", jendela=((50.0, 50),)):
        argv = [run, pool, "1" if sync else "0", str(len(jendela)), share]
        for rate, burst in jendela:
            argv += [repr(rate), str(burst)]
        return self.panggil("token", [], argv)

    def baca(self, pool="p1", run="run-1", kuota=100, ember_mulai=100, mode="bergulir",
             now=None, periode="2026-11-03"):
        if now is None:
            detik, mikro = self.r.cmd("TIME")
            now = int(detik) * 1000000 + int(mikro)
        return self.panggil(
            "baca",
            [f"pr:attempts:{pool}:{periode}", f"pr:ember:{pool}:{periode}"],
            [run, mode, str(now), str(JUMLAH_EMBER), str(kuota), str(ember_mulai)],
        )

    # --- uji --------------------------------------------------------------
    def uji_gerbang_siap(self) -> None:
        print("gerbang kesiapan (§5a.2 butir 2)")
        self.reset()
        self.periksa("intake tanpa run_id", self.intake(run="")[0] == "belum_siap",
                     "run_id kosong harus belum_siap")
        self.periksa("intake run_id beda", self.intake(run="run-LAMA")[0] == "belum_siap",
                     "run_id lama harus belum_siap")
        self.r.cmd("DEL", "pr:siap")
        self.periksa("intake kunci siap hilang", self.intake()[0] == "belum_siap",
                     "tanpa pr:siap harus belum_siap")
        self.r.cmd("DEL", "pr:siap")
        self.periksa("tambah kunci siap hilang", self.tambah()[0] == "belum_siap",
                     "tanpa pr:siap harus belum_siap")
        self.r.cmd("DEL", "pr:siap")
        self.periksa("token kunci siap hilang", self.token()[0] == "belum_siap",
                     "tanpa pr:siap harus belum_siap")
        self.r.cmd("DEL", "pr:siap")
        self.periksa("baca kunci siap hilang", self.baca()[0] == "belum_siap",
                     "tanpa pr:siap harus belum_siap")
        self.siapkan_run("run-1")
        self.periksa("intake siap", self.intake()[0] == "lolos", "run_id benar harus lolos")

    def debug_periksa(self) -> None:
        print("DEBUG periksa")
        self.reset()
        for v in ("145", "100", "1.5", "abc"):
            h = self.panggil("intake", [], ["c", "run-1", "bergulir", "0",
                                           "pr:attempts", "pr:antre", v,
                                           "pr:ember", "145", "100"])
            print("  kuota", v, "->", h)

    def uji_argumen_tidak_sah(self) -> None:
        print("validasi argumen: tidak sah = galat, bukan penolakan")
        self.reset()
        self.periksa("kuota 0", self.intake(kuota=0)[0] == "kuota_efektif_tidak_sah", "kuota 0")
        self.periksa("kuota negatif", self.intake(kuota=-3)[0] == "kuota_efektif_tidak_sah", "-3")
        self.periksa("kuota pecahan", self.intake(kuota="1.5")[0] == "kuota_efektif_tidak_sah", "1.5")
        self.periksa("kuota bukan angka", self.intake(kuota="abc")[0] == "kuota_efektif_tidak_sah", "abc")
        self.periksa("jumlah ember 0", self.intake(ember_mulai=100) and
                     self.panggil("intake", [], ["c", "run-1", "bergulir", "0"])[0] != "lolos", "")
        self.periksa("mode tidak sah", self.intake(mode="bulanan")[0] == "mode_tidak_sah", "mode")
        self.periksa("sync tidak sah",
                     self.panggil("intake", [], ["c", "run-1", "bergulir", "2"])[0] == "sync_tidak_sah", "sync")
        self.periksa("tambah kuota 0", self.tambah(kuota=0)[0] == "kuota_efektif_tidak_sah", "kuota 0")
        self.periksa("token jumlah jendela 0",
                     self.panggil("token", [], ["run-1", "p1", "0", "0", "0.2"])[0]
                     == "jumlah_jendela_tidak_sah", "N=0")
        self.periksa("token jumlah jendela 4",
                     self.panggil("token", [], ["run-1", "p1", "0", "4", "0.2", "1", "1", "1", "1", "1", "1", "1", "1"])[0]
                     == "jumlah_jendela_tidak_sah", "N=4")
        self.periksa("token rate 0", self.token(jendela=((0.0, 5),))[0] == "parameter_rate_tidak_sah", "rate 0")
        self.periksa("token burst 0", self.token(jendela=((5.0, 0),))[0] == "parameter_rate_tidak_sah", "burst 0")
        self.periksa("token share 1", self.token(share="1.0")[0] == "share_tidak_sah", "share")
        self.periksa("token share negatif", self.token(share="-0.1")[0] == "share_tidak_sah", "share")

    def uji_batas_kuota(self) -> None:
        print("kuota habis = terpakai >= kuota_efektif, pada kedua mode (§5a.1 butir 1b)")
        for mode in ("bergulir", "kalender"):
            self.reset()
            kuota = 5
            hasil = [self.intake(mode=mode, kuota=kuota)[0] for _ in range(kuota)]
            self.periksa(f"{mode}: {kuota} intake pertama lolos", all(h == "lolos" for h in hasil), str(hasil))
            for _ in range(kuota):
                self.tambah(mode=mode, kuota=kuota)
            berikut = self.intake(mode=mode, kuota=kuota)
            self.periksa(f"{mode}: intake ke-{kuota + 1} ditolak", berikut[0] == "kuota_habis", str(berikut))
            self.periksa(f"{mode}: terpakai dilaporkan", int(berikut[1]) == kuota, str(berikut))
        # Satu percobaan di bawah kuota masih lolos (off-by-one arah aman).
        self.reset()
        for _ in range(4):
            self.tambah(kuota=5)
        self.periksa("terpakai = kuota-1 masih lolos", self.intake(kuota=5)[0] == "lolos",
                     "batas harus >=, bukan >")

    def uji_antrean(self) -> None:
        print("kapasitas antre (async) vs sync")
        self.reset()
        self.r.cmd("SET", "pr:antre:p1", "10")
        self.periksa("antrean penuh menolak async", self.intake(kuota=10)[0] == "antrean_penuh", "")
        self.periksa("antrean penuh tidak menolak sync", self.intake(kuota=10, sync=True)[0] == "lolos", "")

    def uji_kalender_periode(self) -> None:
        print("mode kalender: terpakai dari counter periode, bukan ember")
        self.reset()
        for _ in range(3):
            self.tambah(mode="kalender", kuota=10)
        # Tulis ember di hari lain: tidak boleh ikut dihitung di mode kalender.
        self.r.cmd("HSET", "pr:ember:p1:2026-11-03", "b50", "7")
        self.r.cmd("SET", "pr:attempts:p1:2026-11-03", "3")
        h = self.intake(mode="kalender", kuota=10)
        self.periksa("kalender mengabaikan ember", h[0] == "lolos" and int(h[1]) == 3, str(h))
        h = self.intake(mode="bergulir", kuota=10)
        self.periksa("bergulir menghitung ember", h[0] == "lolos" and int(h[1]) == 3 + 7, str(h))

    def uji_ember_keluar_jendela(self) -> None:
        print("ember keluar jendela 24 jam (145 ember)")
        self.reset()
        self.tambah(kuota=10, ember_mulai=1000)
        self.r.cmd("HINCRBY", "pr:ember:p1:2026-11-03", "b855", "9")  # 1000-145
        h = self.intake(kuota=10, ember_mulai=1000)
        self.periksa("ember b855 masih dihitung", h[0] == "kuota_habis", str(h))
        self.r.cmd("HINCRBY", "pr:ember:p1:2026-11-03", "b854", "9")  # 1000-146, di luar jendela
        h = self.intake(kuota=10, ember_mulai=1000)
        self.periksa("ember b854 di luar jendela", h[0] == "lolos" and int(h[1]) == 1, str(h))
        # Pemangkasan: setelah tambah, field lama hilang, `terpakai` tinggal.
        self.tambah(kuota=10, ember_mulai=1100)
        fields = set(self.r.cmd("HKEYS", "pr:ember:p1:2026-11-03"))
        self.periksa("field lama dipangkas", "b854" not in fields and "b1000" not in fields, str(fields))
        self.periksa("field terpakai dipertahankan", "terpakai" in fields, str(fields))

    def uji_rate_limit(self) -> None:
        print("rate limit GCRA (TPS/TPM/TPH satu skrip)")
        self.reset()
        h = [self.token(jendela=((5.0, 5),))[0] for _ in range(5)]
        self.periksa("burst terpenuhi lalu ditolak", h[:4] == ["lolos"] * 4 and h[4] in ("lolos", "rate_limit"), str(h))
        hasil = [self.token(jendela=((1.0, 2),))[0] for _ in range(4)]
        self.periksa("rate 1/s burst 2", hasil[0] == "lolos" and hasil[1] == "lolos"
                     and hasil[2] == "rate_limit", str(hasil))
        # Kegagalan satu jendela tidak memakan token jendela lain.
        self.reset()
        self.r.cmd("SET", "pr:rl:p1:tpm", "9999999999999999")  # TPM selalu penuh
        h = self.token(jendela=((1.0, 1), (1.0, 1)))
        self.periksa("tpm menolak", h[0] == "rate_limit" and h[2] == "tpm", str(h))
        self.periksa("tps tidak dipotong saat tpm menolak", self.r.cmd("GET", "pr:rl:p1:tps") is None,
                     "tidak boleh ada TAT tps")
        # Laporan isi bucket.
        self.reset()
        h = self.token(jendela=((1.0, 3),))
        self.periksa("laporan isi bucket", h[1].startswith("tps:") and len(h[1].split(":")) == 3, str(h))

    def uji_porsi_sync(self) -> None:
        print("porsi sync K-10")
        self.reset()
        share = "0.2"
        # Sync: bucket global + bucket sendiri (5*0.2 = 1 token).
        h1 = self.token(sync=True, share=share, jendela=((1.0, 5),))
        h2 = self.token(sync=True, share=share, jendela=((1.0, 5),))
        self.periksa("sync token pertama lolos", h1[0] == "lolos", str(h1))
        self.periksa("sync token kedua: porsi habis", h2[0] == "porsi_sync_habis", str(h2))
        # Async masih boleh memakai bucket global.
        h3 = self.token(sync=False, share=share, jendela=((1.0, 5),))
        self.periksa("async tetap lolos", h3[0] == "lolos", str(h3))
        # Pool tanpa porsi sync: sync ditolak, async bebas.
        self.reset()
        h = self.token(sync=True, share="0", jendela=((1.0, 5),))
        self.periksa("sync pada pool share 0 ditolak", h[0] == "porsi_sync_habis", str(h))
        h = self.token(sync=False, share="0", jendela=((1.0, 5),))
        self.periksa("async pada pool share 0 lolos", h[0] == "lolos", str(h))

    def uji_baca_tidak_mutasi(self) -> None:
        print("pembacaan kuota murni baca + Retry-After")
        self.reset()
        for _ in range(3):
            self.tambah(kuota=10, ember_mulai=100)
        sebelum = self.r.cmd("HGETALL", "pr:ember:p1:2026-11-03")
        now = 100 * JEDA_EMBER * 1000000
        h = self.baca(kuota=10, ember_mulai=100, now=now)
        self.periksa("baca ok", h[0] == "ok", str(h))
        self.periksa("baca terpakai", int(h[1]) == 3, str(h))
        self.periksa("baca attempts", int(h[2]) == 3, str(h))
        sesudah = self.r.cmd("HGETALL", "pr:ember:p1:2026-11-03")
        self.periksa("baca tidak mengubah state", sebelum == sesudah, str((sebelum, sesudah)))
        # Retry-After jendela bergulir = awal ember tertua + 24 jam - now.
        h = self.baca(kuota=10, ember_mulai=100, now=now + 10 * 1000000)
        harap = HARI - 10
        self.periksa("retry-after bergulir", int(h[4]) == harap, f"{h[4]} != {harap}")
        # baca tanpa ember di jendela: tidak ada yang ditunggu.
        h = self.baca(kuota=10, ember_mulai=1000, now=now)
        self.periksa("retry-after tanpa ember = 0", int(h[4]) == 0 and int(h[1]) == 0, str(h))

    def uji_properti_m02(self, iterasi: int) -> None:
        """M-02: tidak ada urutan konkuren yang membuat percobaan melebihi kuota."""
        print(f"property M-02: urutan acak percobaan vs intake ({iterasi} iterasi)")
        rnd = random.Random(20261103)
        for it in range(iterasi):
            self.reset()
            kuota = rnd.randint(1, 8)
            mode = rnd.choice(["bergulir", "kalender"])
            periode = "2026-11-03"
            dihitung = 0  # percobaan yang benar-benar dikirim upstream
            for _ in range(rnd.randint(1, 25)):
                langkah = rnd.random()
                if langkah < 0.5:
                    if self.intake(kuota=kuota, mode=mode, periode=periode)[0] == "lolos":
                        h = self.tambah(kuota=kuota, mode=mode, periode=periode)
                        self.periksa(
                            f"properti it{it}",
                            int(h[2]) <= kuota,
                            f"tambah membuat terpakai {h[2]} > kuota {kuota} (mode {mode})",
                        )
                        dihitung += 1
                else:
                    # Permintaan yang lolos harus selalu di bawah kuota.
                    h = self.intake(kuota=kuota, mode=mode, periode=periode)
                    if h[0] == "lolos":
                        self.periksa(f"properti it{it}", int(h[1]) < kuota,
                                     f"intake lolos dengan terpakai {h[1]} >= kuota {kuota}")
                        self.tambah(kuota=kuota, mode=mode, periode=periode)
                        dihitung += 1
                    else:
                        self.periksa(f"properti it{it}", h[0] in ("kuota_habis", "antrean_penuh"), str(h))
            terkirim = int(self.r.cmd("GET", f"pr:attempts:p1:{periode}") or 0)
            self.periksa(f"properti it{it} (DQ-05)", terkirim == dihitung and terkirim <= kuota,
                         f"mode {mode}: terkirim {terkirim}, dihitung {dihitung}, kuota {kuota}")

    def uji_konkuren(self, klien: int = 8, per_klien: int = 50, kuota: int = 37) -> None:
        print(f"konkurensi: {klien} koneksi x {per_klien} percobaan vs kuota {kuota}")
        self.reset()
        import threading

        hasil: list[int] = []
        kunci = threading.Lock()

        def jalankan():
            r = Redis(*self.r.sock.getpeername()[:2] if False else ("127.0.0.1", 6395))
            lokal = 0
            for _ in range(per_klien):
                if r.evalsha(self.skrip["intake"].sha, [],
                             ["c", "run-1", "bergulir", "0", "pr:attempts:p1:2026-11-03",
                              "pr:antre:p1", str(kuota), "pr:ember:p1:2026-11-03",
                              str(JUMLAH_EMBER), "100"])[0] == "lolos":
                    r.evalsha(self.skrip["tambah"].sha, [],
                              ["run-1", "bergulir", "pr:attempts:p1:2026-11-03", "pr:antre:p1",
                               str(kuota), "pr:ember:p1:2026-11-03", str(JUMLAH_EMBER), "100"])
                    lokal += 1
            r.close()
            with kunci:
                hasil.append(lokal)

        utas = [threading.Thread(target=jalankan) for _ in range(klien)]
        for t in utas:
            t.start()
        for t in utas:
            t.join()
        total = sum(hasil)
        terkirim = int(self.r.cmd("GET", "pr:attempts:p1:2026-11-03") or 0)
        self.periksa("konkuren: tidak melebihi kuota", terkirim <= kuota, f"{terkirim} > {kuota}")
        self.periksa("konkuren: counter = jumlah lolos", total == terkirim, f"{total} != {terkirim}")
        self.periksa("konkuren: memakai hampir semua kuota", terkirim >= kuota - klien,
                     f"hanya {terkirim} dari {kuota} (intake seharusnya tidak memasukkan apa pun)")

    def jalankan_semua(self, iterasi: int) -> int:
        self.uji_gerbang_siap()
        self.uji_argumen_tidak_sah()
        self.uji_batas_kuota()
        self.uji_antrean()
        self.uji_kalender_periode()
        self.uji_ember_keluar_jendela()
        self.uji_rate_limit()
        self.uji_porsi_sync()
        self.uji_baca_tidak_mutasi()
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
