# Data uji sintetis F1 (SHAA-335)

Data uji F1 untuk skenario D1 (aging), D2 (alokasi), D3 (retry webhook), D4 (data balikan), D6 (arsip dan restore), dan D7 (MFA). **Seluruh data sintetis.** Tidak ada NIK asli, nama asli, atau secret di sini.

## Membuat ulang

```sh
python3 backend/db/test/f1/gen_f1.py --out backend/db/test/f1/out
```

Hanya stdlib Python 3. Tidak ada instalasi di mesin owner. Keluaran deterministik: seed tetap `166` (contract v0.9.5-final §6.1), UUID dari `uuid5` dengan namespace tetap, dan waktu tetap. Jalankan dua kali lalu bandingkan `manifest.json` (SHA-256 per berkas) untuk membuktikan keluaran identik.

## Pola identitas sintetis

- NIK: 16 digit, prefiks `00` (kode provinsi tak terpakai), 14 digit sisanya acak dengan seed `166`. Divalidasi generator terhadap `^00[0-9]{14}$` dan tanpa duplikat. Cek DQ-14 contract berlaku.
- Nama: `Budi Contoh 0001`, dan seterusnya. Kata "Contoh", "Uji", "Dummy", "Palsu", "Fiktif" tidak pernah dipakai sebagai nama orang nyata.
- HMAC NIK memakai kunci fixture `SINTETIS-ONLY-...` (bukan secret; lingkungan nyata memakai secret storage).
- Hash kode pemulihan dan pendaftaran MFA adalah placeholder berlabel `SINTETIS-PLACEHOLDER`, bukan hash argon2id sungguhan.
- Vektor TOTP `GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ` adalah vektor publik RFC 6238 Lampiran B (label `RFC6238-PUBLIC-VECTOR`), bukan secret.
- Seed TOTP akun uji tidak dibuat generator. Kolom `secret_ciphertext_palsu` di `mfa_pengguna.csv` berisi referensi `${TOTP_TEST_SEED}`, dibaca dari environment saat runtime.
- Tanggal lahir fiktif berasal dari daftar tetap (`1901-01-01`, `1902-02-02`, dst.) dan divalidasi generator.

## Isi

| Berkas | Skenario | Isi |
|---|---|---|
| `request_aging.csv` | U-57, U-58 | 14 request async, ambang 600 s, dan ambang 0 / 60 s |
| `alokasi_rute.csv` | U-61, U-64, U-65, T-05B-01, T-05B-07 | Q = 100 (A 40 reserved, B 30 shareable, C tanpa alokasi); Σ > Q; boleh_pinjam = false tanpa alokasi; skala Q = 1000 |
| `webhook_jadwal.csv` | U-68 | Jadwal retry 6 percobaan dengan jitter seed tetap; percobaan ke-6 tanpa jadwal |
| `webhook_klasifikasi.csv` | U-67 | Kelas error yang di-retry dan yang tidak |
| `data_balikan_batch.csv`, `data_balikan_item.csv` | U-74, TA-D4-01 | Batch dengan N = 250 item; tanggal lahir fiktif |
| `sentinel_nik.csv` | T-13-08, U-89 | Sentinel NIK `0000000000009999`; generator tidak pernah membuat NIK acak yang sama |
| `webhook_jadwal_cepat.csv` | T-06B-01 | Jalur cepat batas retry 3, fixture terpisah |
| `webhook_klien.csv` | T-06B-03, T-06B-09, T-06B-10 | Dua client webhook (`client-a`, `client-b`) dengan riwayat terpisah |
| `arsip_log.csv`, `arsip_restore_permintaan.csv` | U-76 | 50 baris log dengan NIK sintetis dan `nik_hmac`; restore NIK ada dan tidak ada |
| `mfa_*.csv` | U-79, U-81, U-83 | Vektor TOTP RFC 6238 (dihitung ulang dan dicocokkan), kode pemulihan, kode pendaftaran kedaluwarsa, gagal ≥ 5; `mfa_pengguna.csv` berisi lima akun uji (auditor, dua platform admin, operator, client owner) |
| `expected.json` | semua | Angka harapan yang ditulis sebelum implementasi |
| `manifest.json` | semua | Jumlah baris dan SHA-256 per berkas |

## Status

- Berkas ini belum dimuat ke PostgreSQL Podman. Skema F1 (SHAA-333, migrasi aditif) belum ada di `main`. Pemuatan dan verifikasi constraint menyusul setelah migrasi itu masuk.
- Stub webhook, provider data balikan, dan MinIO bukan keluaran generator; konfigurasi compose menjadi dependensi pemilik (kemungkinan Platform & SRE).
- Validasi di generator adalah cek aturan contract di level data (format NIK, kunci unik, jadwal monoton, N = 250, vektor TOTP). Cek constraint database belum bisa dijalankan.
