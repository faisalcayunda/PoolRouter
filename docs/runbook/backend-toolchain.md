## Ringkasan

Runbook ini menjelaskan cara menyiapkan, memeriksa, dan membersihkan toolchain gate backend (`make -C backend setup|test|lint`) di mesin dev. Toolchain berjalan di container Podman dengan image Rust yang di-pin. `cargo-deny` dipasang sebagai biner rilis resmi yang checksum SHA-256-nya di-pin di `backend/Makefile`, tidak di-compile dari source.

- **Pemilik:** Tetsu (Platform & SRE), reviewer Huan
- **Tanggal:** 2026-11-03
- **Status:** Gate lolos di VM 2 CPU / 3814 MiB atas keputusan owner, tertanggal 2026-11-03 (lihat bagian Batasan).

## Prasyarat

- Podman dengan `podman machine` yang sudah berjalan. Podman dan machine disediakan owner; jangan dipasang, di-upgrade, atau di-resize dari runbook ini.
- Tidak ada instalasi native. Jangan memasang Rust, `cargo-deny`, atau database di mesin owner.

## Menyiapkan dari nol

```sh
make -C backend setup CONTAINER_LABEL=shaal.issue=<ISSUE>
```

Yang dilakukan target `setup`:

1. `cargo fetch --locked` di dalam container Rust yang di-pin.
2. Jika `cargo deny --version` belum menunjukkan `CARGO_DENY_VERSION` (default `0.19.0`), container mengunduh tarball rilis resmi sesuai arsitektur (`aarch64` atau `x86_64`), memverifikasi SHA-256 terhadap nilai yang di-pin, lalu memasang biner ke `.cache/cargo/bin`.
3. Jika checksum tidak cocok, target berhenti dengan exit ≠ 0 dan biner tidak dipasang.

Cache per checkout ada di `backend/.cache/` (di-gitignore).

CI menjalankan `make setup test lint` dari akar repo dengan `RUNNER=native`. Target itu memanggil `make -C backend setup` untuk memasang `cargo-deny`; tidak ada cara pasang kedua.

## Kondisi VM saat ini

- VM `podman-machine-default` berjalan pada **2 CPU / 3814 MiB RAM** (disk tetap). Ini kondisi yang berlaku sekarang, atas keputusan owner tertanggal 2026-11-03.
- Akibatnya **hanya satu build Rust pada satu waktu**, dengan `JOBS=1`. Jangan menjalankan dua build Rust bersamaan (misalnya dua `make test` atau `make lint` dari issue berbeda).
- Sebelum build apa pun, cek container yang sedang jalan:

```sh
podman ps --filter label=shaal.issue --format '{{.Names}} {{.Labels}}'
```

  Jika ada build Rust lain yang masih jalan, tunggu sampai selesai dan koordinasikan di issue pemiliknya (label `shaal.issue=...` menunjukkan issue asalnya). Jangan menghentikan container milik orang lain.
- Service persisten (`shaal-hindsight`) ikut memakai memori VM yang sama.
- Resize hanya dijalankan **owner**, tidak oleh agent. Jangan mengubah konfigurasi `podman machine` dari runbook ini.

Rekomendasi jika owner meresize nanti (bukan kondisi saat ini): **6144 MiB RAM, 4 CPU**. Dengan VM sebesar itu, dua build Rust bersamaan dengan batas 3 GiB per container lebih aman. Langkah verifikasi setelah resize:

```sh
podman machine inspect --format '{{.Resources.Memory}} MiB, {{.Resources.CPUs}} CPU'   # harus 6144 MiB, 4 CPU
podman ps --filter label=shaal.service=hindsight                                       # shaal-hindsight harus Up dan sehat
```

## Batas memori

Setiap container toolchain dibatasi `--memory $(CONTAINER_MEMORY)` (default `3g`). Override bila perlu:

```sh
make -C backend test CONTAINER_MEMORY=4g
```

Pada VM 2 CPU / 3814 MiB, batas `3g` per container berarti hanya satu build Rust yang muat, ditambah `shaal-hindsight`. Menaikkan `CONTAINER_MEMORY` di VM ini tidak disarankan dan butuh keputusan owner.

## Kunci build berat (SHAA-194, K-41)

Aturan "satu build berat di VM" dikunci secara teknis, tidak hanya lewat prosedur. Kunci bersama ada di `scripts/build-lock.sh` (logika) dan `scripts/build-lock.mk` (include). Backend memakainya; frontend memakai include yang sama, tanpa implementasi kedua.

- **Cara kerja.** Setiap container build berat (`test-rust`, `openapi`, dan `clippy` di `lint`) dijalankan dengan nama tetap `shaal-build-heavy`, `--rm`, label `shaal.issue=<ISSUE>`, dan label `shaal.build=heavy`. Podman menolak nama ganda secara atomik, jadi run kedua langsung gagal. Tidak ada file kunci. Kunci lepas sendiri saat container selesai dan dihapus (`--rm`).
- **Yang dikunci (backend).** Hanya target compile dan hanya saat `RUNNER=podman`: `test-rust`, `openapi`, dan `clippy` di `lint`. `setup`, `fmt`, `clean`, `db-test`, dan `cargo deny` tidak dikunci. `RUNNER=native` (CI) tidak memakai kunci.
- **Yang dikunci (frontend).** Dengan `FRONTEND_RUNNER=podman`: `setup` (`pnpm install`), `test`, `build`, dan `e2e` dikunci. `lint`, `fmt`, `i18n-check`, dan `clean` tidak. `FRONTEND_RUNNER=local` (CI) tidak memakai kunci.
- **Kontrak teks podman.** Deteksi bentrok bergantung pada teks `container name "shaal-build-heavy" is already in use` dari podman. Versi yang diuji: podman 6.1.2. Bila versi podman mengubah teks ini, bentrok jatuh ke exit 125 (gagal aman, bukan 75). Setelah upgrade podman, uji ulang dengan dua run berurutan dan pastikan run kedua exit 75.
- **Exit 75 (`EX_TEMPFAIL`).** Bila kunci dipegang, target gagal dalam ≈0,4 detik (batas AC ≤ 5 detik) dengan exit 75 dan pesan yang menyebut `shaal.issue=` pemegang kunci. Target tidak menunggu. Jalankan pekerjaan non-compile (misalnya `setup` atau `fmt`), atau tunggu. Exit 75 hanya keluar saat podman melaporkan bentrok nama `shaal-build-heavy`. Kegagalan lain, misalnya image tidak ada atau VM mati, memakai exit code aslinya (contoh terukur: 125).
- **Kunci basi.** Bila container kunci sudah berhenti (`exited`, `stopped`, atau `dead`) tetapi belum terhapus, skrip menghapusnya sendiri (hanya bila berlabel `shaal.build=heavy`) lalu mencoba sekali lagi. Container yang masih jalan tidak pernah dihapus tanpa `-f`.
- **Yang tertahan saat `make` di-kill.** Bila `make` atau `podman` client di-kill dengan SIGKILL, container build tetap jalan sampai selesai. Kunci tertahan selama build itu, lalu lepas sendiri. Hasil uji (alpine, sleep 20 detik): client di-kill sekitar detik ke-3, kunci masih dipegang, dan lepas sekitar 16 detik kemudian saat container selesai dan dihapus. Untuk build Rust, kunci tertahan selama build berjalan. Bila perlu segera melepas, hentikan container `shaal-build-heavy` secara sengaja, bukan prosesnya di host.
- **Perintah.**

```sh
make -C backend build-lock-status                  # bebas atau dipegang (issue dan status)
make -C backend build-lock-clean                   # hapus kunci basi saja; kunci yang jalan tidak disentuh
podman ps -a --filter label=shaal.build=heavy --format '{{.Names}} {{.Status}} {{.Labels}}'
```

Jangan menghentikan container `shaal-build-heavy` milik issue lain tanpa koordinasi dengan pemilik `shaal.issue`-nya.

Bukti uji (2026-11-03, VM 2 CPU / 3814 MiB, alpine:3.20 untuk kontensi dan rust:1.99.1 untuk build):

- Run kedua saat kunci dipegang: exit 75 dalam ≈0,4 detik, pesan menyebut `shaal.issue=SHAA-194`.
- **Dua worktree (2026-11-03, blocking K-41 dari Huan).** Worktree A (`PoolRouter`) menjalankan `make -C backend test-rust JOBS=1` dari bersih (`make clean` dahulu). Selagi A jalan dan kontainer `shaal-build-heavy` berstatus running, worktree B (`git worktree` detached di commit yang sama, setup sendiri) menjalankan `make -C backend test-rust`. Hasil: A exit 0 (34 test lulus, 0 gagal). B ditolak: pesan `kunci dipegang container shaal-build-heavy (shaal.issue=SHAA-194, status=running)` dan `make: *** [test-rust] Error 75`.
- **Catatan exit code make.** GNU make selalu keluar dengan status 2 bila resep gagal; exit code resep (75) hanya muncul di baris `Error 75`. Untuk skrip yang perlu exit 75 mentah, panggil `sh scripts/build-lock.sh run -- <perintah podman run ...>` langsung, bukan lewat `make`.
- Setelah run pertama selesai: kunci lepas tanpa langkah manual. Run berikutnya lulus dengan exit code aslinya (exit 3 diuji).
- Exit 125 karena registry tidak ada: tetap 125, bukan 75.
- Kunci basi (container `exited` tertinggal): dihapus otomatis, run lulus (exit 0).
- `make -C backend test-rust` saat kunci dipegang: exit 75. `make -C backend test-rust JOBS=1` tanpa kunci: lulus, semua suite `ok`.

## Memeriksa

```sh
podman machine inspect --format '{{.Resources.Memory}} MiB, {{.Resources.CPUs}} CPU'
podman ps --format '{{.Names}} {{.Labels}}'
make -C backend setup CONTAINER_LABEL=shaal.issue=<ISSUE>; echo "exit=$?"
make -C backend build-lock-status
```

Container toolchain harus berlabel `shaal.issue=<ISSUE>`. Container tanpa label dicek asal-usulnya sebelum dihapus.

## Label wajib

- Untuk `RUNNER=podman`, target `setup`, `test`, `lint`, `fmt`, dan `clean` gagal dengan exit 2 bila `CONTAINER_LABEL` kosong atau tidak berbentuk `shaal.issue=SHAA-<n>`. Tidak ada label default.
- `podman run` untuk toolchain hanya boleh lewat `make` dengan `CONTAINER_LABEL=shaal.issue=<ISSUE>`.
- Container tanpa label yang ditemukan dilaporkan di issue terkait. Jangan dihapus atau dihentikan sendiri tanpa koordinasi dengan pemiliknya.

## Membersihkan

- Container sementara dihapus otomatis (`podman run --rm`). Jika ada sisa, cek dengan `podman ps -a --filter label=shaal.issue=<ISSUE>` dan hapus hanya container dengan label issue sendiri.
- Membersihkan workdir yang sudah selesai: jalankan `make clean CONTAINER_LABEL=shaal.issue=<ISSUE>` dari akar checkout. Target ini menghapus `backend/target`, `backend/.cache`, `frontend/node_modules`, dan `frontend/.cache` serta direktori build frontend (`dist`, `coverage`, `test-results`, `playwright-report`) di dalam container berlabel yang hanya me-mount direktori `backend` atau `frontend` checkout itu. Artefak itu dibuat container sebagai root, jadi `rm` dari host gagal `Permission denied`; jangan menghapusnya dengan `rm` host atau `sudo`. Container-nya `--rm`, tidak ada image atau volume yang disentuh, dan target ini tidak memakai kunci build (boleh jalan saat `shaal-build-heavy` aktif). Setelah itu cek `podman ps -a --filter label=shaal.issue=<ISSUE>` kosong. `setup` berikutnya memasang ulang `cargo-deny`, crate, dan `node_modules`.
- Jangan menghentikan `podman machine` atau container yang tidak berlabel issue sendiri tanpa koordinasi dengan pemiliknya.

## Mengganti versi cargo-deny

1. Ambil SHA-256 tarball rilis baru untuk kedua arsitektur dari halaman rilis resmi.
2. Ubah `CARGO_DENY_VERSION`, `CARGO_DENY_SHA256_aarch64`, dan `CARGO_DENY_SHA256_x86_64` di `backend/Makefile` dalam satu commit.
3. Uji negatif: ganti satu checksum dengan nilai salah, pastikan exit ≠ 0, lalu kembalikan.

## Batasan

- VM Podman dipakai pada 2 CPU / 3814 MiB, bukan 4 CPU / 6144 MiB yang direkomendasikan. Keputusan owner (SHAA-222, 2026-11-03): resize tidak dilanjutkan, gate dijalankan dengan `JOBS=1`. Risiko OOM tetap ada; jika gate gagal dengan SIGKILL atau exit 137, jangan diulang dengan `CONTAINER_MEMORY` lebih besar tanpa keputusan owner.
- Gate penuh `make setup test lint` dengan `CONTAINER_LABEL=shaal.issue=SHAA-222 JOBS=1` di `main` `065aba7` lolos (exit 0, sekitar 371 detik) di kondisi VM di atas.
