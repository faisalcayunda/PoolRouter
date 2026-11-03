# PoolRouter: aturan git

Aturan ini mengikat semua agent dan kontributor. Standar teknis umum ada di skill `shaal-eng-standards`; kalau bertentangan, file ini yang berlaku.

## Branch dan push
- Satu branch per pekerjaan: `agent/<nama-agent>/<issue>`, contoh `agent/flik/shaa-200`.
- Push hanya ke branch milik sendiri. Push langsung ke `main` dilarang. Satu-satunya pengecualian adalah commit bootstrap awal repo ini.
- Dilarang: force push ke `main`, rebase atau reset branch milik orang lain, menghapus branch milik orang lain, dan menambah remote.

## Pull request dan merge
- Semua perubahan masuk ke `main` lewat PR ke `main`.
- PR di-approve Ridley pada hash yang akan di-merge. Setelah itu PR di-merge oleh pelaksana (Flik atau Viktor) dengan merge commit (`gh pr merge --merge`), bukan squash atau rebase.
- Sebelum merge: merge `main` terbaru ke branch PR, jalankan gate penuh repo, dan pastikan lolos (`shaal-eng-standards` §6). Kalau ada konflik atau gate gagal, perbaiki di branch PR, bukan di `main`.
- Board of Directors tidak commit, push, atau merge.

## Rahasia dan data
- Tidak ada secret di repo. Konfigurasi lewat environment variable, dan placeholder ditulis `${VAR}`. `.env` di-gitignore; yang boleh di-commit hanya template `*.example`.
- Data NIK hanya sintetis.

## Struktur dan stack
- Struktur folder, stack, dan gate (`Makefile`) ditetapkan lewat ADR SHAA-178 dan ditambahkan lewat PR.
- Kode, identifier, dan pesan commit ditulis dalam Bahasa Inggris. Dokumen di `docs/` ditulis dalam Bahasa Indonesia.
