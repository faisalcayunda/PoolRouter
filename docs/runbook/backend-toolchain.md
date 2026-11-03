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

## Memeriksa

```sh
podman machine inspect --format '{{.Resources.Memory}} MiB, {{.Resources.CPUs}} CPU'
podman ps --format '{{.Names}} {{.Labels}}'
make -C backend setup CONTAINER_LABEL=shaal.issue=<ISSUE>; echo "exit=$?"
```

Container toolchain harus berlabel `shaal.issue=<ISSUE>`. Container tanpa label dicek asal-usulnya sebelum dihapus.

## Label wajib

- Untuk `RUNNER=podman`, target `setup`, `test`, `lint`, `fmt`, dan `clean` gagal dengan exit 2 bila `CONTAINER_LABEL` kosong atau tidak berbentuk `shaal.issue=SHAA-<n>`. Tidak ada label default.
- `podman run` untuk toolchain hanya boleh lewat `make` dengan `CONTAINER_LABEL=shaal.issue=<ISSUE>`.
- Container tanpa label yang ditemukan dilaporkan di issue terkait. Jangan dihapus atau dihentikan sendiri tanpa koordinasi dengan pemiliknya.

## Membersihkan

- Container sementara dihapus otomatis (`podman run --rm`). Jika ada sisa, cek dengan `podman ps -a --filter label=shaal.issue=<ISSUE>` dan hapus hanya container dengan label issue sendiri.
- Hapus cache checkout dari akar repo: `rm -rf -- backend/.cache` (akan memasang ulang `cargo-deny` dan crate saat `setup` berikutnya).
- Jangan menghentikan `podman machine` atau container yang tidak berlabel issue sendiri tanpa koordinasi dengan pemiliknya.

## Mengganti versi cargo-deny

1. Ambil SHA-256 tarball rilis baru untuk kedua arsitektur dari halaman rilis resmi.
2. Ubah `CARGO_DENY_VERSION`, `CARGO_DENY_SHA256_aarch64`, dan `CARGO_DENY_SHA256_x86_64` di `backend/Makefile` dalam satu commit.
3. Uji negatif: ganti satu checksum dengan nilai salah, pastikan exit ≠ 0, lalu kembalikan.

## Batasan

- VM Podman dipakai pada 2 CPU / 3814 MiB, bukan 4 CPU / 6144 MiB yang direkomendasikan. Keputusan owner (SHAA-222, 2026-11-03): resize tidak dilanjutkan, gate dijalankan dengan `JOBS=1`. Risiko OOM tetap ada; jika gate gagal dengan SIGKILL atau exit 137, jangan diulang dengan `CONTAINER_MEMORY` lebih besar tanpa keputusan owner.
- Gate penuh `make setup test lint` dengan `CONTAINER_LABEL=shaal.issue=SHAA-222 JOBS=1` di `main` `065aba7` lolos (exit 0, sekitar 371 detik) di kondisi VM di atas.
