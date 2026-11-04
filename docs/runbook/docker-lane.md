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

Setiap target `HEAVY` (`test-rust`, `lint` clippy, `test`, dan `pnpm install` frontend) lewat kunci `scripts/build-lock.sh` dengan `BUILD_LOCK_ENGINE=docker`.

### Kunci build: antre FIFO (SHAA-395)

Jika kunci `shaal-build-heavy` dipegang, target **menunggu**, tidak gagal seketika:

- Run mengambil tiket di antrean host (`BUILD_LOCK_QUEUE_DIR`, default `${TMPDIR:-/tmp}/shaal-build-lock-queue`; nama tiket = nanodetik datang + PID). Hanya run dengan tiket paling depan yang boleh mencoba memulai container, jadi urutannya FIFO dan tidak ada yang direbut run yang datang belakangan.
- Pesan `build-lock: menunggu giliran (kunci dipegang ... shaal.issue=SHAA-<n> ...)` muncul saat mulai menunggu dan tiap 60 detik.
- Batas tunggu: `BUILD_LOCK_WAIT_SECS`, default **1800** (30 menit). Contoh: `BUILD_LOCK_WAIT_SECS=3600 make push-main ...`. Interval cek: `BUILD_LOCK_POLL_SECS` (default 2).
- Batas habis: exit **75** (`EX_TEMPFAIL`), pesan menyebut label `shaal.issue` pemegang kunci dan panjang antrean. Tidak ada build yang jalan; push-main berhenti tanpa push.
- Tiket milik proses yang mati (kill, VM restart) dibuang otomatis oleh run berikutnya, jadi antrean tidak tersumbat. Kunci container yang tertinggal (basi) tetap ditangani seperti sebelumnya (`build-lock-clean`; perbaikan lanjutan di SHAA-366).
- Kunci tetap hanya milik container build (`--rm`); antrean hanya mengatur giliran mencoba. Run di luar skrip (memakai nama container yang sama langsung) tidak antre, tetapi run yang antre menunggunya selesai.
- Batasan: tiket dianggap mati lewat `kill -0 <pid>`, jadi semua run harus satu host dan satu PID namespace (lane saat ini). Run dari container/PID namespace atau pengguna lain bisa dianggap mati dan tiketnya dibuang.
- Pastikan `timeout` job/agent Anda lebih besar dari `BUILD_LOCK_WAIT_SECS` plus lama build.

Uji manual (engine Docker, image apa saja yang punya `sleep`):

```sh
export BUILD_LOCK_ENGINE=docker
run() { sh scripts/build-lock.sh run -- docker run --rm --name shaal-build-heavy --label shaal.build=heavy --label shaal.issue=$1 --entrypoint sleep <image> $2; }
run SHAA-A 6 & sleep 1; run SHAA-B 3 & sleep 1; run SHAA-C 3 & wait      # selesai berurutan A, B, C
run SHAA-A 12 & sleep 2; (export BUILD_LOCK_WAIT_SECS=4; run SHAA-B 1; echo $?)    # 75
```

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

Cache repo di `backend/.cache/` dan `backend/target/` tinggal di checkout (di-gitignore).

### Batas cache

- Batas `backend/.cache/` dan `backend/target/` gabungan: **2G**. Di atas itu, jalankan `make -C backend clean` di checkout itu sebelum build berikutnya. Hasilnya build ulang penuh, jadi lakukan hanya saat ruang kritis.
- Pada pengukuran 2026-11-04, `target/` 1.1G dan `.cache/` 1.3G, jadi sudah di atas batas itu. Keputusan ini diterima dengan konsekuensi build ulang.

### Kepemilikan file

Container berjalan sebagai root, jadi file di `backend/target/` dan `backend/.cache/` dimiliki root. Akibatnya `make -C backend clean` berhasil lewat container, tetapi `rm -rf` langsung di host bisa gagal dengan izin. Pakai `make -C backend clean` untuk menghapus. Agent lain di checkout yang sama mungkin perlu `sudo` bila membersihkan manual.

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
