# Runbook: lane build/test PoolRouter di Docker (SHAA-373)

Runbook ini menjelaskan cara menjalankan gate backend di Docker, menjaga ruang kosong, dan membersihkan resource. Dasar keputusan: arahan owner 2026-11-04, persetujuan Luca U-20261108-1 (`shaal-playbook` §6, pengecualian sementara).

- **Pemilik:** Tetsu (Platform & SRE), reviewer Ridley, Huan untuk runbook.
- **Tanggal:** 2026-11-04.
- **Status:** gate backend dan frontend lolos di sandbox instabox (lihat Pengukuran dan Pengukuran disk frontend).

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
- Saat di bawah ambang, `check` lebih dulu menjalankan `gc-checkouts` (SHAA-408): hanya `frontend/node_modules` dan `frontend/.cache` di checkout Multica (`$WORKSPACES_ROOT/*/shaa-*/workdir/PoolRouter`) yang issue-nya `done`/`cancelled` dan HEAD-nya ancestor `main`. Checkout dengan status lain atau status tidak terbaca tidak disentuh. Lalu ruang dicek ulang; push tetap ditolak bila masih di bawah ambang. Container frontend berjalan dengan `--user` host (Docker), jadi artefaknya bisa dihapus tanpa sudo.
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

### Cache bersama antar-checkout dan batas ukuran (SHAA-394)

Satu salinan per host, dipakai semua checkout. Tidak ada lagi toolchain atau registry di `backend/.cache` (`RUNNER=docker`).

| Isi | Lokasi (host) | Batas | Penjaga |
| --- | --- | --- | --- |
| rustup, registry/git Cargo, cargo-deny | `/data/workspace/shaal-build/cargo-home/{cargo,rustup}` | `SHARED_CARGO_HOME_LIMIT_MB` = 2560 MB (terukur 2026-11-04: 1315 MB) | `check-disk` berhenti, tidak menghapus apa pun |
| store pnpm frontend | tidak dibagi: tetap per checkout di `frontend/.cache/pnpm-store` | lihat keputusan di bawah | `make -C frontend clean` per checkout |
| `backend/target` | `/var/tmp/shaal-build/poolrouter-target` | lihat bagian SHAA-390 | `check-disk` |

Konkurensi:

- Penulisan registry Cargo memakai lock bawaan Cargo (`.package-cache`) pada cargo-home bersama. Dua container yang hanya membaca tidak saling merusak.
- Salinan toolchain rustup, `cargo fetch`, pemasangan cargo-deny, dan `pnpm install` jalan lewat `HEAVY`, yaitu di bawah kunci `shaal-build-heavy` (antre FIFO, SHAA-395). Pembaruan toolchain tidak bisa terjadi di luar kunci.
- Checkout kedua: `setup` menemukan `rustup/toolchains` dan `cargo-deny` sudah ada, `cargo fetch` tidak mengunduh apa pun. `du backend/.cache` hanya berisi sisa lokal (kunci/oasdiff).

Membersihkan (target eksplisit, terpisah dari `clean`):

```sh
# registry/git Cargo saja (toolchain dan cargo-deny tetap); setup berikutnya mengisi ulang
RUNNER=docker CONTAINER_LABEL=shaal.issue=SHAA-<n> make -C backend clean-cache-shared
# termasuk toolchain rustup dan cargo-deny
RUNNER=docker CONTAINER_LABEL=shaal.issue=SHAA-<n> make -C backend clean-cache-shared CLEAN_TOOLCHAIN=1
```

Kedua target backend jalan di bawah kunci heavy dan hanya menghapus path tetap di dalam mount cache bersama. `make -C backend clean` tetap hanya menghapus artefak checkout itu dan tidak menyentuh cache bersama.

Migrasi dari `backend/.cache` lama: cukup `make -C backend clean` per checkout (menghapus `.cache/cargo` dan `.cache/rustup` lama).

**Keputusan pnpm (lingkup 5, diukur 2026-11-04):** store pnpm tidak dibagi. Di checkout dengan store dan `node_modules` pada satu mount (`/work`), file `node_modules/.pnpm` punya link count 2 (hardlink ke store lokal), jadi total sekitar 440 MB per checkout tanpa duplikasi. Store bersama di mount terpisah gagal `link()` (EXDEV), pnpm menyalin file (link count 1), dan malah menambah sekitar 400 MB di `/data`. Angka "419 MB per checkout" adalah store yang sudah berbagi inode dengan `node_modules`, bukan duplikat. Pembersihan: `make -C frontend clean` di checkout selesai.

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

- Dengan 20G, lane dapat menjalankan gate backend dan frontend secara berurutan; rincian pemakaian disk tercatat pada tabel Pengukuran di atas dan Pengukuran disk frontend di bawah.
- Resize `/data` ke 40G masih rekomendasi untuk jangka panjang.

## Root Docker gate (SHAA-379)

Gate resmi (keputusan SHAA-380), dari akar repo:

```sh
make setup test lint RUNNER=docker FRONTEND_RUNNER=docker CONTAINER_LABEL=shaal.issue=SHAA-<n>
```

Push ke `main` (gate penuh dulu, lalu push fast-forward; variabel sama):

```sh
make push-main RUNNER=docker FRONTEND_RUNNER=docker CONTAINER_LABEL=shaal.issue=SHAA-<n>
```

Variabel juga dapat diletakkan di depan perintah (mis. `RUNNER=docker FRONTEND_RUNNER=docker CONTAINER_LABEL=shaal.issue=SHAA-<n> make push-main`). `push-main` adalah satu-satunya jalur push ke `main`. Ia menjalankan gate backend dan frontend (`setup`, `test`, `lint`) dan hanya push bila semua target exit 0.

### `make -n` aman sebagai bukti

Sejak SHAA-398, `make -n` pada gate di atas tidak memanggil `docker` atau `podman`.
GNU make tetap mengeksekusi baris resep yang memuat teks `$(MAKE)` walau dengan `-n`;
karena itu resep dengan efek samping container memakai `SUBMAKE = $(MAKE)` dan menulis
`$(SUBMAKE)`. Jangan menulis `$(MAKE)` langsung di resep seperti itu. Resep delegasi
akar (`$(MAKE) -C $$s $@`) sengaja tetap `$(MAKE)` agar `-n` diteruskan ke stack.

Uji regresi (tanpa Docker, stub pencatat `docker`/`podman` di `PATH`; gagal bila log
stub tidak kosong):

```sh
make test-dry-run
```

### Pengukuran disk frontend (2026-11-04, sandbox instabox, /data 20G)

| Komponen | Sebelum Gate | Sesudah Gate | Catatan |
|---|---|---|---|
| Ruang kosong `/data` | 8,3G (58%) | 3,6G (82%) | Terukur via `df -h /data`; penurunan mencakup kompilasi target backend serta aktivitas konkuren lane lain di host |
| Ruang kosong Docker root (`/var/lib/docker`) | 8,3G (58%) | 3,6G (82%) | Satu partisi (`/dev/vdb`) dengan `/data`; tetap di atas ambang minimum 3G |
| `frontend/node_modules` | 0 | 396M | 479 paket terinstal |
| Store pnpm (`frontend/.cache/pnpm-store`) | 0 | 22M | Store lokal per checkout (keputusan SHAA-394) |
| Cache frontend total (`frontend/.cache`) | 0 | 23M | Termasuk pnpm-store dan corepack |
| Image Node pinned (`node:24.22.2-bookworm-slim`) | 333M | 333M | Dipakai runner Docker frontend |
| Image Playwright pinned (`mcr.microsoft.com/playwright`) | Belum di-pull | Belum di-pull | Hanya ditarik saat target `e2e` dipanggil (ukuran manifest layer ~990MB terkompresi, estimasi uncompressed ~2,4GB) |

Penambahan disk lokal spesifik frontend adalah sekitar 420M (gabungan `node_modules` 396M dan `.cache` 23M). Total ruang kosong `/data` berkurang dari 8,3G ke 3,6G akibat kompilasi target backend bersamaan dengan aktivitas lane lain di host. Gate memeriksa ruang kosong sebelum target berat dijalankan dan menolak eksekusi bila di bawah ambang batas.
