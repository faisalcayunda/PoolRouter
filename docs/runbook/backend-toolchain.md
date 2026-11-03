## Ringkasan

Runbook ini menjelaskan cara menyiapkan, memeriksa, dan membersihkan toolchain gate backend (`make -C backend setup|test|lint`) di mesin dev. Toolchain berjalan di container Podman dengan image Rust yang di-pin. `cargo-deny` dipasang sebagai biner rilis resmi yang checksum SHA-256-nya di-pin di `backend/Makefile`, tidak di-compile dari source.

- **Pemilik:** Tetsu (Platform & SRE), reviewer Huan
- **Tanggal:** 2026-11-03
- **Status:** Draf untuk review Huan. Langkah VM Podman belum dijalankan (lihat bagian Batasan).

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

CI memakai target `make -C backend setup` yang sama, tidak ada cara pasang kedua untuk `cargo-deny`.

## Ukuran VM

- Target: **6144 MiB RAM, 4 CPU** (disk tetap). Alasan: dua build Rust berjalan bersamaan dengan batas 3 GiB per container, ditambah service persisten (`shaal-hindsight`).
- Resize hanya dijalankan **owner**, tidak oleh agent. Langkah siap jalan dikirim owner lewat Luca di SHAA-222 (`podman machine stop`, `set --memory 6144 --cpus 4`, `start`).
- Verifikasi setelah resize:

```sh
podman machine inspect --format '{{.Resources.Memory}} MiB, {{.Resources.CPUs}} CPU'   # harus 6144 MiB, 4 CPU
podman ps --filter label=shaal.service=hindsight                                       # shaal-hindsight harus Up dan sehat
```

## Batas memori

Setiap container toolchain dibatasi `--memory $(CONTAINER_MEMORY)` (default `3g`). Override bila perlu:

```sh
make -C backend test CONTAINER_MEMORY=4g
```

Batas ini hanya berguna bila VM Podman cukup besar. Dengan VM 2 GiB, dua build Rust bersamaan tetap bisa membuat VM kehabisan memori.

## Memeriksa

```sh
podman machine inspect --format '{{.Resources.Memory}} MiB, {{.Resources.CPUs}} CPU'
podman ps --format '{{.Names}} {{.Labels}}'
make -C backend setup CONTAINER_LABEL=shaal.issue=<ISSUE>; echo "exit=$?"
```

Container toolchain harus berlabel `shaal.issue=<ISSUE>`. Container tanpa label dicek asal-usulnya sebelum dihapus.

## Membersihkan

- Container sementara dihapus otomatis (`podman run --rm`). Jika ada sisa, cek dengan `podman ps -a --filter label=shaal.issue=<ISSUE>` dan hapus hanya container dengan label issue sendiri.
- Hapus cache checkout dari akar repo: `rm -rf -- backend/.cache` (akan memasang ulang `cargo-deny` dan crate saat `setup` berikutnya).
- Jangan menghentikan `podman machine` atau container yang tidak berlabel issue sendiri tanpa koordinasi dengan pemiliknya.

## Mengganti versi cargo-deny

1. Ambil SHA-256 tarball rilis baru untuk kedua arsitektur dari halaman rilis resmi.
2. Ubah `CARGO_DENY_VERSION`, `CARGO_DENY_SHA256_aarch64`, dan `CARGO_DENY_SHA256_x86_64` di `backend/Makefile` dalam satu commit.
3. Uji negatif: ganti satu checksum dengan nilai salah, pastikan exit ≠ 0, lalu kembalikan.

## Batasan

- Resize VM Podman (6 GiB, 4 CPU) belum dilakukan. Itu langkah owner, dikoordinasikan lewat Luca (SHAA-222). Bagian ini dihapus setelah resize.
- Gate penuh `make setup test lint` belum dijalankan ulang di VM 2 GiB karena build Rust berisiko membuat VM kehabisan memori.
