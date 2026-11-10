# Backend PoolRouter

Workspace Rust untuk webserver, controller, worker, dan `backend-cli`. Gate dan toolchain dijalankan
lewat `make` di container (lihat `Makefile`); tidak ada instalasi Rust native.

## `backend-cli`: pengguna dan peran admin

Pengguna dan peran admin F0 dikelola lewat CLI, tanpa layar (ADR-0003 §3.6c). Binary `backend-cli`
ada di image backend dan dijalankan di container backend lingkungan target:

```sh
podman exec -it <container-backend> backend-cli <perintah> [opsi]
```

CLI memakai role basis data `pr_admin_api` (bukan `poolrouter_owner`) dan membaca konfigurasi yang
sama dengan komponen lain: `POOLROUTER_ENV`, `POOLROUTER_DB_HOST`, `POOLROUTER_DB_PORT`,
`POOLROUTER_DB_NAME`, `POOLROUTER_DB_SSLMODE`, dan `POOLROUTER_DB_ADMIN_API_PASSWORD_FILE`. Secret
hanya boleh lewat berkas `<NAMA>_FILE`.

Aturan yang berlaku untuk semua perintah tulis:

- `--oleh` (username platform admin yang aktif) dan `--alasan` (10–500 karakter) wajib. Untuk peran
  auditor dan operator, isi alasan dengan rujukan SPTJM atau NDA (C-07).
- Password tidak pernah lewat argumen. Opsi `--password` tidak dikenal. Password dibaca dari TTY
  tanpa echo (diminta dua kali) atau, bila stdin bukan TTY, satu baris dari stdin. Minimal 15
  karakter dan tidak boleh password umum (R-11).
- Setiap perubahan tercatat. Buat, nonaktifkan, dan aktifkan pengguna menulis `audit_konfigurasi`
  (`entitas_jenis = admin_user`). Beri peran, cabut peran, nonaktifkan, dan aktifkan juga menulis
  satu baris `audit_akses` `ubah_peran` dan mencabut semua sesi target, dalam transaksi yang sama.
  Di `audit_akses`, `sumber_ip` dan `sesi_id` berisi `cli`. Target dan perubahannya ada di
  `filter_kueri`.
- Pelaku di CLI dinyatakan, tidak dibuktikan. Akses `exec` ke container dibatasi untuk petugas
  infrastruktur (risiko sisa ADR-0003 §3.6c, ditutup UI pengguna di F1).

Opsi ditulis `--nama nilai` atau `--nama=nilai`. Nilai yang diawali `--` hanya diterima dalam
bentuk `--nama=nilai`.

| Perintah | Keterangan |
|---|---|
| `bootstrap --username <u> --alasan <teks>` | Platform admin pertama. Hanya berhasil saat tabel pengguna kosong. |
| `pengguna buat --username <u> --oleh <admin> --alasan <teks> [--client <uuid>]` | Pengguna baru tanpa peran. `--client` wajib untuk calon client owner. |
| `pengguna nonaktifkan --username <u> --oleh <admin> --alasan <teks>` | Pengguna tidak bisa login; semua sesinya dicabut. Pelaku tidak bisa menonaktifkan dirinya sendiri. |
| `pengguna aktifkan --username <u> --oleh <admin> --alasan <teks>` | Mengaktifkan kembali pengguna. |
| `peran beri --username <u> --peran <peran> --oleh <admin> --alasan <teks> [--disetujui-oleh <admin lain>] [--berlaku-sampai <tanggal>]` | Peran: `platform_admin`, `client_owner`, `operator`, `auditor`. |
| `peran cabut --username <u> --peran <peran> --oleh <admin> --alasan <teks>` | Mencabut satu peran. |
| `daftar` | Username, status aktif, peran yang berlaku, dan client. Tanpa hash password. |
| `seed pengguna` | Pengguna sintetis per peran, hanya untuk `dev` dan `ci`. |
| `seed dukcapil` | Provider, upstream pool, downstream pool, dan endpoint sintetis, hanya untuk `dev` dan `ci`. Jalankan sesudah `seed pengguna`. |

Aturan peran (A-02, contract §8.1):

- Peran `auditor` dan `operator` wajib `--disetujui-oleh`, yaitu platform admin aktif yang berbeda
  dari pelaku dan dari target. Pelaku tidak bisa memberi dirinya sendiri peran auditor atau operator.
- `client_owner` hanya untuk pengguna yang dibuat dengan `--client`.
- Di `POOLROUTER_ENV=poc`, peran auditor wajib `--berlaku-sampai` (kontrol kompensasi R-12).
  Format `YYYY-MM-DD` berarti sampai akhir hari itu WIB; format RFC 3339 juga diterima.

Kode keluar: `0` berhasil, `2` argumen atau konfigurasi salah, `1` ditolak atau gagal. Saat ditolak,
tidak ada perubahan yang tersimpan.

### Contoh

```sh
# CT = nama container backend di lingkungan target.
# Platform admin pertama di basis data baru.
podman exec -it "$CT" backend-cli bootstrap \
  --username admin-bgn --alasan "Bootstrap lingkungan PoC, BA-2026-001"

# Pengguna baru lalu peran auditor dengan persetujuan admin kedua.
podman exec -it "$CT" backend-cli pengguna buat \
  --username auditor-satu --oleh admin-bgn --alasan "Penugasan auditor, SPTJM-2026-014"
podman exec -it "$CT" backend-cli peran beri \
  --username auditor-satu --peran auditor --oleh admin-bgn --disetujui-oleh admin-dua \
  --berlaku-sampai 2026-12-31 --alasan "Penugasan auditor, SPTJM-2026-014"

# Password dari stdin (mis. dari pengelola secret), tanpa TTY.
printf '%s\n' "$PASSWORD_BARU" | podman exec -i "$CT" backend-cli pengguna buat \
  --username operator-satu --oleh admin-bgn --alasan "Penugasan operator, NDA-2026-007"

podman exec "$CT" backend-cli daftar
```

### Seed pengguna (dev dan ci)

`seed pengguna` hanya berjalan bila `POOLROUTER_ENV` adalah `dev` atau `ci`. Di lingkungan lain
perintah keluar non-nol tanpa perubahan. Password semua pengguna seed dibaca dari
`POOLROUTER_SEED_PASSWORD_FILE` (konfigurasi menolak berkas itu di luar `dev`/`ci`).

Pengguna yang dibuat, semuanya sintetis:

| Username | Peran |
|---|---|
| `seed-platform-admin` | `platform_admin` (pengaju semua peran seed) |
| `seed-penyetuju` | `platform_admin` (penyetuju peran auditor dan operator seed) |
| `seed-operator` | `operator` |
| `seed-auditor` | `auditor` |
| `seed-client-owner` | `client_owner`, terikat client `Client Sintetis Seed` |
| `seed-admin-auditor` | `platform_admin` dan `auditor` |

Seed idempoten: menjalankannya lagi tidak membuat baris baru.

### Seed dukcapil (dev dan ci)

`seed dukcapil` membuat satu rantai konfigurasi sintetis supaya jalur sync dan async bisa dicoba
tanpa mengisi konfigurasi lewat UI. Aturannya sama dengan `seed pengguna`: hanya `dev` dan `ci`,
idempoten, dan setiap objek baru tercatat di `audit_konfigurasi` atas nama `seed-platform-admin`,
sehingga `seed pengguna` harus dijalankan lebih dulu (kalau belum, perintah keluar non-nol).

| Objek | Nama |
|---|---|
| provider (versi 1 aktif) | `seed-dukcapil-provider` |
| upstream pool (kuota harian 1000) | `seed-dukcapil-upstream` |
| downstream pool, rute ke upstream di atas | `seed-dukcapil-downstream` |
| downstream endpoint (versi 1 aktif) | `seed-dukcapil-endpoint` |

Batasan: template provider tidak memakai `{{secret.*}}` (CLI tidak punya KEK), `base_url`-nya
`https://dukcapil-sintetis.example.test/api` (admin API mewajibkan https; arahkan ke mock lewat
`updateProvider` bila perlu), dan data balikan tidak diaktifkan (keputusan D-08 masih terbuka).

Di Compose dev: `make compose-seed ENV=dev` menjalankan kedua seed berurutan lewat profil `cli`.

## Uji

- Unit test CLI dan aturan A-02 berjalan di `make -C backend test-rust`.
- Uji PostgreSQL `crates/cli/tests/cli_bootstrap_pg.rs` dan `crates/cli/tests/cli_pg.rs` berjalan di
  `make -C backend uji-pg`. `cli_bootstrap_pg` sengaja dijalankan paling awal karena bootstrap hanya
  sah pada tabel pengguna kosong.
