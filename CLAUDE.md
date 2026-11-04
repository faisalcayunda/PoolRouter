# PoolRouter: aturan git

Aturan ini mengikat semua agent dan kontributor. Standar teknis umum ada di skill `shaal-eng-standards`; kalau bertentangan, file ini yang berlaku.

Dasar wewenang: keputusan owner K-34 dan K-35 (2026-11-03, dicatat Fitcher di SHAA-179 dan SHAA-182). Squad Engineering boleh commit, push, dan membuat branch sendiri di repo ini, termasuk push langsung ke `main` tanpa PR. Konfirmasi owner per aksi git (`shaal-eng-standards` §0/§2) tidak diperlukan. Setiap push ke `main` di-review Ridley post-hoc. Force-push ke `main` dan secret di repo tetap dilarang.

## Branch dan push
- Satu branch per pekerjaan: `agent/<nama-agent>/<issue>`, contoh `agent/flik/shaa-200`.
- Push ke branch milik sendiri atau langsung ke `main` (fast-forward, tanpa force). Branch orang lain tidak disentuh.
- Dilarang: force push ke `main`, rebase atau reset branch milik orang lain, menghapus branch milik orang lain, dan menambah remote.

## Pull request dan merge
- PR ke `main` tidak wajib. Perubahan boleh di-push langsung ke `main`.
- Ridley me-review post-hoc hash commit di `main` setelah pelaksana (Flik atau Viktor) me-mention. Temuan blocking diperbaiki dengan push lanjutan ke `main`.
- Sebelum push ke `main`: rebase atau merge `main` terbaru ke branch kerja (`shaal-eng-standards` §6).
- Push ke `main` hanya lewat `make push-main CONTAINER_LABEL=shaal.issue=SHAA-<n>`. Gate penuh `make setup test lint` wajib exit 0 pada hash persis yang di-push. `cargo check`, test parsial, dan `git diff --check` bukan pengganti gate.
- Gate tidak bisa dijalankan (kunci, timeout, tool tidak ada) berarti tidak push. Pekerjaan tetap di branch, lalu dilaporkan di issue.
- Laporan push ke `main` wajib mengutip blok bukti dari `make push-main`. Push tanpa bukti adalah temuan blocking di review post-hoc Ridley.
- `main` merah: pemilik commit pemicu memperbaikinya dengan prioritas di atas pekerjaan lain. Pengecualian satu-satunya: commit perbaikan untuk `main` yang merah boleh di-push bila gate pada hash itu tidak menambah kegagalan baru dibanding `main`. Issue perbaikannya disebut di pesan commit, dan pengecualian ini dicatat di laporan.
- Board of Directors tidak commit, push, atau merge.

## Rahasia dan data
- Tidak ada secret di repo. Konfigurasi lewat environment variable, dan placeholder ditulis `${VAR}`. `.env` di-gitignore; yang boleh di-commit hanya template `*.example`.
- Data NIK hanya sintetis.

## Struktur dan stack
- Struktur folder, stack, dan gate (`Makefile`) ditetapkan lewat ADR SHAA-178 dan ditambahkan lewat commit ke `main`.
- Kode, identifier, dan pesan commit ditulis dalam Bahasa Inggris. Dokumen di `docs/` ditulis dalam Bahasa Indonesia.
