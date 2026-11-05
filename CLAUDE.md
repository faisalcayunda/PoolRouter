# PoolRouter: aturan repo Arctiva

Aturan ini mengikat semua agent dan kontributor Arctiva di repo ini, dan menggantikan aturan
workspace Shaal yang sudah obsolete. Standar teknis Arctiva ada di skill `arctiva-eng-standards`;
kalau bertentangan, file ini yang berlaku.

Dasar wewenang: keputusan Operator (Faisal Nugraha Cayunda) pada 5 November 2026 WIB — Arctiva
mengambil alih pengembangan PoolRouter dari workspace Shaal. Spec: `takeover-shaal`.

Beda pokok dengan aturan lama: **tidak ada push langsung ke `main`.** Setiap perubahan masuk lewat
pull request yang sudah lulus gate. Konfirmasi owner per aksi git tidak diperlukan di luar aturan ini;
force-push ke `main` dan secret di repo tetap dilarang.

## Branch dan PR
- Satu branch per pekerjaan, format `agent/<ID>-<slug>`: `<ID>` adalah id tiket Arctiva (`ARC-<n>`)
  dan `<slug>` ringkas dengan tanda hubung. Contoh: `agent/ARC-2-aturan-repo-arctiva`.
- Nama branch, judul PR, dan pesan commit memakai id issue Arctiva (`ARC-<n>`), bukan id Shaal.
- **PR wajib ke `main`** untuk setiap perubahan; tidak ada pengecualian untuk perubahan kecil.
  `main` hanya menerima merge commit dari PR yang sudah lulus gate. Push langsung ke `main`,
  termasuk `git push origin HEAD:main`, ditolak oleh reviewer dan harus di-revert.
- Merge hanya oleh Operator (G2). Agent tidak merge PR-nya sendiri dan tidak memindahkan tiket ke
  `done`; `done` terjadi saat Operator merge. PR siap review hanya setelah gate penuh hijau.
- Force-push hanya boleh di branch kerja milik sendiri, tidak pernah di `main`.
- Sebelum PR: rebase atau merge `main` terbaru ke branch kerja.

## Gate wajib
- Gate penuh `make setup`, `make test`, `make lint` wajib exit 0 pada hash persis yang di-PR-kan.
  `cargo check`, test parsial, dan `git diff --check` bukan pengganti gate.
- Gate dijalankan dengan engsel kontainer yang dipin:
  `make setup test lint RUNNER=docker FRONTEND_RUNNER=docker CONTAINER_LABEL=arctiva.issue=ARC-<n>`.
- PR tidak di-merge kalau gate merah, konflik belum selesai, atau diff memuat secret.

## Label gate `arctiva.issue`
- Setiap container, volume, dan network yang dibuat untuk satu issue **wajib** berlabel
  `arctiva.issue=ARC-<n>` (satu label per issue). Label `arctiva.issue` menggantikan label Shaal
  `shaal.issue=SHAA-<n>`; label lama tidak dipakai lagi.
- `make -C backend ...` dan `make -C frontend ...` dengan runner kontainer gagal di target
  `check-label` kalau `CONTAINER_LABEL` kosong atau tidak berbentuk `arctiva.issue=ARC-<n>`.
- Kunci build bersama memakai label `poolrouter.build=heavy` di container `poolrouter-build-heavy`;
  jalankan satu build berat per waktu.
- Pembersihan hanya boleh menyasar resource dengan label milik issue ini
  (`--filter label=arctiva.issue=ARC-<n>`). Prune global (`docker system prune`,
  `docker volume prune`, `docker image prune -a`) dan `docker rm` tanpa filter label dilarang.
- Resource milik layanan lain di host (mis. `arctiva-hindsight` berlabel `arctiva.service=hindsight`,
  `dokploy-traefik`) tidak boleh disentuh.

## Hal yang tidak boleh dilakukan
- Push langsung ke `main`. `make push-main` sengaja dipertahankan sebagai penjaga: ia menolak
  dengan exit 2 dan mencetak alur PR; jangan menghidupkan lagi jalur push-nya.
- Force-push ke `main`, menulis secret ke repo atau ke log CI.
- Menghapus atau merename branch milik agent lain tanpa kesepakatan dengan pemiliknya.
- Memakai data pribadi asli di fixture, test, atau log. Data NIK hanya sintetis.

## Bahasa dan gaya
- Komentar kode, pesan commit, dan dokumen repo memakai bahasa Indonesia untuk teks penjelas;
  istilah teknis, nama target, dan path tetap bahasa Inggris.
- Pesan commit: `feat|fix|test|docs|chore(<scope>): <ringkas> (ARC-<n>)`.
