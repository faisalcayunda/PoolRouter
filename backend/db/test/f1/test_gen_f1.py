#!/usr/bin/env python3
"""Uji generator data uji F1 (SHAA-335): determinisme dan rentang jitter.

Jalankan dengan: python3 -I backend/db/test/f1/test_gen_f1.py
Hanya stdlib. Tidak menulis ke folder `out/` yang ter-commit.
"""

import filecmp
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
GEN = os.path.join(HERE, "gen_f1.py")
OUT = os.path.join(HERE, "out")


def regenerasi(dir_tujuan):
    proc = subprocess.run(
        [sys.executable, "-I", GEN, "--out", dir_tujuan],
        capture_output=True, text=True, check=False,
    )
    return proc


class RegenerasiManifest(unittest.TestCase):
    def test_regenerasi_identik_dengan_manifest(self):
        with tempfile.TemporaryDirectory() as tmp:
            proc = regenerasi(tmp)
            self.assertEqual(proc.returncode, 0, proc.stderr)
            # manifest.json harus sama byte per byte dengan yang ter-commit.
            self.assertTrue(filecmp.cmp(os.path.join(tmp, "manifest.json"),
                                        os.path.join(OUT, "manifest.json"), shallow=False))
            with open(os.path.join(OUT, "manifest.json"), encoding="utf-8") as f:
                manifest = json.load(f)
            self.assertTrue(manifest["sintetis"])
            for nama, info in manifest["berkas"].items():
                with open(os.path.join(tmp, nama), "rb") as f:
                    sha = hashlib.sha256(f.read()).hexdigest()
                self.assertEqual(sha, info["sha256"], nama)

    def test_expected_tidak_mengunci_tunda_jitter(self):
        with open(os.path.join(OUT, "expected.json"), encoding="utf-8") as f:
            exp = json.load(f)
        d3 = exp["d3_webhook"]
        self.assertNotIn("jadwal_detik_per_percobaan", d3)
        self.assertEqual(d3["percobaan_retry"], 5)
        self.assertEqual(d3["jitter_rentang"], [0.8, 1.2])

    def test_generator_menolak_jitter_di_luar_rentang(self):
        # Validasi harus menangkap tunda di luar nominal x [0,8; 1,2].
        import importlib.util
        spec = importlib.util.spec_from_file_location("gen_f1", GEN)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        g = mod.Gen()
        mod.scen_d3(g)
        self.assertFalse([e for e in mod.validasi(g) if "webhook" in e])
        g.jitter_cek.append((1920, 3000))  # 3000 > 1920 x 1,2 + 1
        self.assertTrue(any("di luar rentang" in e for e in mod.validasi(g)))


if __name__ == "__main__":
    unittest.main()
