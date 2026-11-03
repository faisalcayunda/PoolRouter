# Runbook: build cache Cargo di SSD eksternal (SHAA-274)

Tujuan: folder `target` Cargo tidak lagi tumbuh di workdir task (3–4 GiB per folder) dan tidak mengisi disk utama mesin owner. Artefak dipindah ke SSD eksternal.

## Tata letak yang diusulkan

- Satu folder akar per repo: `${POOLROUTER_CARGO_CACHE}`, berisi `target/`.
- Satu `target` bersama untuk semua run PoolRouter. Alasan: build berat sudah diserialkan lewat kunci bersama (`scripts/build-lock.sh`, K-41), jadi tidak ada dua compile yang menulis bersamaan. Artefak antar-branch hidup berdampingan karena fingerprint Cargo per profil dan fitur. Ukuran total dibatasi di satu tempat, yaitu akar per repo.
- Alternatif yang ditolak: `target` per agent atau per branch. Setiap folder bisa 3–4 GiB, sehingga disk penuh lagi setelah beberapa branch. Ini penyebab masalah sekarang.
- `CARGO_HOME` (registry, `backend/.cache/cargo`) tidak dipindah di issue ini. Ukurannya kecil dibanding `target`, dan issue ini hanya mencakup `target`.

## Env yang dibutuhkan (diisi owner, bukan di repo)

| Variabel | Isi | Keterangan |
|---|---|---|
| `POOLROUTER_CARGO_CACHE` | `${SSD_ROOT}/shaal-cache/poolrouter` | Folder akar di SSD. Owner yang membuatnya. |
| `POOLROUTER_CARGO_MIN_FREE_KIB` | opsional, default `10485760` (10 GiB) | Sisa ruang minimum sebelum build ditolak. |

Nilai `${SSD_ROOT}` ditulis owner di env agent. Path SSD tidak ditulis di repo.

## Guard: `scripts/cargo-target-ssd.sh`

Pemakaian:

```sh
CARGO_TARGET_DIR="$(sh scripts/cargo-target-ssd.sh)" cargo build --locked
```

Script hanya mencetak path `target` bila semua syarat terpenuhi. Kalau tidak, script gagal cepat dengan exit code berikut:

| Exit | Arti | Tindakan |
|---|---|---|
| 2 | `POOLROUTER_CARGO_CACHE` kosong atau bukan path absolut | Isi env di env agent (lihat langkah owner). |
| 3 | Folder akar tidak ada | SSD belum ter-mount, atau owner belum membuat folder akar. Script tidak membuat folder ini, supaya disk utama tidak terisi. |
| 4 | Folder akar ada di disk boot | Sama dengan exit 3: SSD tidak ter-mount dan path jatuh ke disk utama. Build dibatalkan. |
| 5 | Sisa ruang di bawah batas | Jalankan pembersihan di bawah, lalu cek lagi. |

## Batas ukuran dan pembersihan

- Batas kerja: 40 GiB dipakai untuk `target` PoolRouter. Di atas itu, bersihkan sebelum build berikutnya.
- Cek ukuran: `du -sh "$POOLROUTER_CARGO_CACHE/target"` dan `df -h "$POOLROUTER_CARGO_CACHE"`.
- Pembersihan penuh (hanya setelah cek di bawah):

```sh
TARGET="$(realpath "${POOLROUTER_CARGO_CACHE:?}")/target"
case "$TARGET" in "$(realpath "${POOLROUTER_CARGO_CACHE:?}")"/target) ;; *) echo "path tidak aman" >&2; exit 1 ;; esac
ls -la -- "$TARGET" > ./logs/cargo-target-ls.txt
rm -rf -- "$TARGET"
```

  Sebelum `rm`, simpan hasil `ls` seperti di atas. Hapus hanya folder `target` di dalam akar cache. Jangan menghapus akar cache atau folder SSD lain. Folder `qh-cargo-targets` milik owner tidak disentuh.

## Batasan

- Tanpa `brew install` atau service native. Toolchain Rust tetap di container Podman.
- Podman machine tidak diubah oleh agent. Lihat "Blocker: mount SSD di Podman" di bawah.
- Tidak ada secret. Path owner hanya di env, tidak di repo.

## Blocker: mount SSD di Podman (keputusan owner)

Pada 2026-11-03, VM Podman (`podman-machine-default`, applehv) tidak melihat `/Volumes/Faisal SSD`:

```
Error: statfs /Volumes/Faisal SSD: no such file or directory
```

Build Rust di container tidak bisa menulis ke SSD selama path itu tidak diekspos ke VM. Menambahkan share ke Podman machine berarti membuat ulang machine, dan itu aksi owner (bukan agent). Dampaknya ke semua container di mesin owner, jadi perlu persetujuan Luca.

## Langkah siap tempel untuk owner (dijalankan owner, lewat Luca)

1. Buat folder akar di SSD (sekali):

   ```sh
   mkdir -p "/Volumes/Faisal SSD/shaal-cache/poolrouter"
   ```

2. Set env di env agent (bukan di repo):

   ```sh
   POOLROUTER_CARGO_CACHE="/Volumes/Faisal SSD/shaal-cache/poolrouter"
   ```

3. Keputusan terpisah, perlu Luca: agar container bisa menulis ke SSD, path `/Volumes/Faisal SSD` harus diekspos ke Podman machine. Ini mengharuskan `podman machine` dibuat ulang dengan opsi `-v`. Belum dikerjakan agent.

## Uji dari nol

1. `env -u POOLROUTER_CARGO_CACHE sh scripts/cargo-target-ssd.sh` → exit 2.
2. `POOLROUTER_CARGO_CACHE=/tmp sh scripts/cargo-target-ssd.sh` → exit 4.
3. `POOLROUTER_CARGO_CACHE="/Volumes/Faisal SSD/tidak-ada" sh scripts/cargo-target-ssd.sh` → exit 3, dan folder itu tidak dibuat.
4. Setelah langkah owner (folder akar ada, env di-set, mount tersedia): script mencetak path `.../target` dengan exit 0.
5. Build: `CARGO_TARGET_DIR=... cargo build` dari dalam container (belum bisa dijalankan sampai blocker di atas selesai).
