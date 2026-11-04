# Runbook: lane build/test PoolRouter di Docker (SHAA-373)

Runbook ini menjelaskan cara menjalankan gate backend di Docker, menjaga ruang kosong, dan membersihkan resource. Dasar keputusan: arahan owner 2026-11-04, persetujuan Luca U-20261108-1 (`shaal-playbook` §6, pengecualian sementara).

- **Pemilik:** Tetsu (Platform & SRE), reviewer Ridley, Huan untuk runbook.
- **Tanggal:** 2026-11-04.
- **Status:** gate backend lolos di sandbox instabox (lihat Pengukuran). Gate frontend belum diukur.

## Prasyarat

- Docker sudah terpasang di host. Jangan memasang, meng-upgrade, atau mengubah konfigurasi Docker dari runbook ini.
- Tidak ada Podman di host instabox. Jika host lain memakai Podman, pakai `RUNNER=podman` seperti biasa.
- Akses repo: deploy key atau SSH key untuk akun runtime, dengan akses baca.

## Menjalankan gate backend

Dari akar repo, satu build berat per waktu. Label wajib `shaal.issue=SHAA-<n>`.

```sh
RUNNER=docker CONTAINER_LABEL=shaal.issue=SHAA-<n> make -C backend setup
RUNNER=docker CONTAINER_LABEL=shaal.issue=SHAA-<n> make -C backend test-rust
RUNNER=docker CONTAINER_LABEL=shaal.issue=SHAA-<n> make -C backend lint
RUNNER=docker CONTAINER_LABEL=shaal.issue=SHAA-<n> make -C backend test
```

`test` menjalankan `test-rust`, `db-test` (PostgreSQL 17 sementara), dan `uji-lua` (Redis sementara). Keduanya memakai engine Docker dari `DB_ENGINE` dan `LUA_ENGINE`, yang mengikuti `RUNNER`.

Setiap target `HEAVY` (`test-rust`, `lint` clippy, `test`) lewat kunci `scripts/build-lock.sh` dengan `BUILD_LOCK_ENGINE=docker`. Jika kunci dipegang, target keluar dengan exit 75. Tunggu, lalu ulangi.

## Ambang ruang kosong

Di `RUNNER=docker`, Makefile menjalankan `check-disk` (`scripts/docker-lane.sh check`) sebagai prasyarat `check-label`. Jadi setiap target yang memakai container (`setup`, `test-rust`, `lint`, `test`, `db-test`, `uji-lua`) berhenti dengan exit ≠ 0 sebelum container jalan, bila ruang di bawah ambang. Cek manual:

```sh
scripts/docker-lane.sh check
```

- Ambang: 3G untuk filesystem Docker root (`/var/lib/docker`), 1G untuk `/data`.
- Di bawah ambang: run berhenti dan melapor. Jangan diakali, jangan turunkan ambang tanpa keputusan Luca.
- Pakai `MIN_FREE_ROOT_GB` atau `MIN_FREE_DATA_GB` hanya dengan keputusan tertulis. Bukti guard: `MIN_FREE_DATA_GB=999 make -C backend test-rust RUNNER=docker CONTAINER_LABEL=shaal.issue=SHAA-<n>` keluar exit 2 sebelum container apa pun dibuat.

Selama build, pantau dengan `df -BG /data`. Jika ruang turun di bawah 3G, hentikan build (Ctrl-C, lalu `docker ps --filter label=shaal.issue=SHAA-<n>`), bersihkan (lihat di bawah), lalu laporkan angka puncak.

## Membersihkan

Pembersihan hanya berfilter label issue sendiri:

```sh
scripts/docker-lane.sh cleanup SHAA-<n>
```

Perintah ini menghapus container, volume, dan network berlabel `shaal.issue=SHAA-<n>`. Image dasar (`rust:1.99.1-bookworm`) dibiarkan karena dipakai bersama. Cache build dibatasi dengan:

```sh
scripts/docker-lane.sh builder-prune
```

Yang dilarang di lane ini: `docker system prune` dalam bentuk apa pun, `docker volume prune`, `docker image prune -a`, dan `docker rm` tanpa filter label. Container milik layanan lain di host (`shaal-9router-*`, `shaal-hermes-*`, `dokploy-traefik`, container tanpa label `shaal.issue`) tidak boleh disentuh.

### Target dan cargo-home bersama (SHAA-390)

Dengan `RUNNER=docker`, semua workdir memakai satu salinan cache, bergantian di bawah kunci `shaal-build-heavy`:

| Isi | Default (di host) | Variabel override | Disk |
| --- | --- | --- | --- |
| `CARGO_TARGET_DIR` (hasil build saja, boleh hilang saat mesin di-stop) | `/var/tmp/shaal-build/poolrouter-target` | `SHARED_TARGET_DIR` | `/` |
| `CARGO_HOME` dan `RUSTUP_HOME` (registry, toolchain, cargo-deny; persisten) | `/data/workspace/shaal-build/cargo-home/{cargo,rustup}` | `SHARED_CARGO_HOME` | `/data` |

Semua path diatur di `backend/Makefile` (blok `SHARED_*`), tidak ada yang di-hardcode di tempat lain. Direktori di-bind ke container di `/cache/{target,cargo,rustup}`, dan container berjalan dengan `--user <uid>:<gid>` host, jadi semua file dimiliki user host dan bisa dihapus tanpa sudo. Pergantian branch antar-lane membangun ulang crate workspace, tetapi dependensi dipakai ulang.

**Penjaga `/`** (`check-disk`, sebelum setiap target container):

1. Ukuran target bersama > `SHARED_TARGET_LIMIT_MB` (2560 MB): `make clean-shared` (`cargo clean` pada target bersama, di bawah kunci heavy). Bila kunci dipegang build lain, penjaga keluar 75 dan tidak membersihkan apa pun.
2. Bebas `/` harus >= `ROOT_MIN_FREE_MB` (2048 MB, sisa untuk sistem) ditambah ruang tumbuh target sampai batasnya (batas dikurangi ukuran sekarang). Di bawah itu: exit non-nol, build tidak jalan.

**`clean` vs `clean-shared`:**

- `make -C backend clean` (aturan akhir run Luca) hanya membersihkan sisa lokal workdir itu (`backend/target`, `backend/.cache/{cargo,rustup,oasdiff}`). Target dan cargo-home bersama tidak disentuh.
- `make -C backend clean-shared RUNNER=docker CONTAINER_LABEL=shaal.issue=SHAA-<n>` mengosongkan target bersama dengan `cargo clean`, di bawah kunci. Dipanggil penjaga ukuran; jangan dipanggil di akhir run. Cargo-home bersama tidak pernah dihapus oleh Makefile.

**Migrasi per pemilik workdir** (jangan menyentuh workdir orang lain):

1. Pastikan tidak ada build berjalan di workdir-mu (`make -C backend build-lock-status RUNNER=docker`).
2. Ambil commit ini, lalu di workdir-mu jalankan `RUNNER=docker CONTAINER_LABEL=shaal.issue=SHAA-<n> make -C backend clean`. Ini menghapus `backend/target` dan `backend/.cache` lama (root-owned, dihapus lewat container root yang hanya me-mount workdir itu).
3. Jalankan gate seperti biasa. Run pertama setelah target bersama kosong adalah build dingin; run berikutnya dari workdir mana pun memakai ulang dependensi.

Sebelum migrasi, `backend/target` dan `backend/.cache` lama ada di `/data` dan berlipat per workdir; sesudahnya hanya ada satu cargo-home di `/data` dan satu target di `/`.

### Kepemilikan file

Container gate (cargo, rustup, cargo-deny) berjalan dengan uid:gid host, jadi isi target dan cargo-home bersama dimiliki user host dan bisa dihapus tanpa sudo. Sisa lama yang dimiliki root (dari sebelum SHAA-390) dihapus lewat `make -C backend clean` di workdir pemiliknya.

## Pengukuran (2026-11-04, sandbox instabox, /data 20G)

| Langkah | Bebas `/data` | Catatan |
| --- | --- | --- |
| Awal | 8G | 94% sebelumnya, lalu owner menaikkan ruang |
| Setelah pull `rust:1.99.1-bookworm` | 6G | image 2.2G |
| Setelah `setup` | 5G | cargo fetch, cargo-deny |
| Setelah `test-rust` | 4G | `target/` 1.1G, `.cache/` 1.3G; 50 test lulus |
| Setelah `lint` | 4G | clippy dan cargo-deny lulus |
| Setelah `test` (termasuk db-test dan uji-lua) | 3G | 416 test lulus, 0 gagal |

Puncak pemakaian: 5G (image 2.2G, target dan cache sekitar 2.5G). Angka ini pengukuran nyata, bukan perkiraan.

## Batasan

- Dengan 20G, lane hanya muat satu gate backend. Gate frontend (`pnpm install`) belum diukur dan butuh ruang lebih.
- Resize `/data` ke 40G masih rekomendasi untuk jangka panjang.
- Gate penuh `make setup test lint` di akar repo belum ada target setup/test/lint di Makefile akar. Gate yang dijalankan adalah target backend di atas.

## Root Docker gate (SHAA-379)

Gate akar (menjalankan seluruh lane dan push fast-forward):

```sh
RUNNER=docker FRONTEND_RUNNER=docker CONTAINER_LABEL=shaal.issue=SHAA-379 make push-main
```

This is the only root gate command. It runs the backend and frontend gates and
pushes only after every target exits zero.

Catat sebelum dan sesudah gate: `df -h /data`; ukuran `frontend/node_modules` dan
store pnpm; ukuran image Node pinned dan image Playwright pinned (`docker images`),
serta ruang kosong `/data`. Gate memeriksa ruang kosong sebelum target berat dan
menolak eksekusi bila di bawah ambang.
