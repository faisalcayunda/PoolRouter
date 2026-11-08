---
feature: takeover-shaal
product: poolrouter
output_type: website
risk: high
status: approved
approved_by: Faisal Nugraha Cayunda (Operator)
approved_at: 5 November 2026, 05.51 WIB
---

## Tujuan

Arctiva mengambil alih pengembangan PoolRouter (repo `faisalcayunda/PoolRouter`) dari workspace Shaal yang sudah obsolete. Yang dikerjakan: repo beralih ke aturan Arctiva, pekerjaan Shaal yang belum di-merge diselamatkan lewat PR baru (SHAA-198, SHAA-199, SHAA-202), dan branch Shaal lainnya diarsipkan.

Keputusan Operator (Faisal Nugraha Cayunda, 5 November 2026 WIB): lihat decision log `products/poolrouter/grilling/2026-11-05-takeover-shaal.md`.

## Kondisi awal (diperiksa 5 November 2026 WIB)

- `main` di `20a07ee`, tanpa branch protection.
- 65 branch remote selain `main`. 22 di antaranya sudah ter-merge ke `main`, 43 belum.
- `CLAUDE.md` di repo memuat aturan Shaal: push langsung ke `main`, review post-hoc oleh Ridley, `make push-main`.
- `check-label` di `backend/Makefile` dan `frontend/Makefile` hanya menerima `CONTAINER_LABEL=shaal.issue=SHAA-<n>`. Tiket ARC tidak bisa menjalankan gate Podman. Nama kunci build (`shaal-build-heavy`) dan path cache (`/var/tmp/shaal-build`, `/data/workspace/shaal-build`) juga masih berprefiks shaal.
- Sumber yang akan diselamatkan:

| SHAA | Branch | Commit | Isi | Diff | Merge ke main |
|---|---|---|---|---|---|
| 199 | `agent/flik/e050bf1448d4` | `3c9c04a` | `core/src/retry.rs`: retry full-jitter + tabel `error_class`, 8 unit test | +347 | bersih |
| 199 | `agent/flik/e050bf1448d4` | `f20c65c` | subcommand `healthcheck` di `core/src/runtime.rs` | +40 | bersih |
| 198 | `agent/flik/a62ee1a1e75a` | `45b5a4e` | WIP `intake.rs` (validasi field/metadata, Idempotency-Key, sidik), `gerbang.rs` (urutan penolakan, pemetaan 400/422/429/503). Belum dipanggil handler, gate belum pernah jalan | +1130 | bersih |
| 202 | `agent/viktor/a7f1ba11e6e5` | `02c3b09`, `8d6b423` | client `openapi-fetch` di atas `schema.gen.ts`, msw dev dependency, regen schema | +484 / −115 | bersih |
| 202 | `agent/viktor/a7f1ba11e6e5` | `c255a16`, `70f88d7` | layar login, dialog peringatan sesi, routing sadar sesi, test skip link | +759 / −21 | bersih |

- `agent/flik/0525d1f3f578` (SHAA-198: probe admin, GCRA, layer problem+json) konflik dengan `main`. Isinya sudah masuk `main` dalam bentuk lain (`3a2e0fa`), jadi branch ini diarsipkan, tidak diselamatkan.

## Scope

1. Aturan repo Arctiva: `CLAUDE.md` baru, `check-label` menerima `arctiva.issue=ARC-<n>`, dan `make push-main` tidak lagi menjadi jalur resmi.
2. Arsip branch: setiap branch remote selain `main` dan tiga branch sumber di atas mendapat tag `archive/shaal/<nama-branch>` di commit ujungnya, lalu branch dihapus. Inventaris ditulis di `docs/runbook/arsip-branch-shaal.md`.
3. Penyelamatan SHAA-199: retry policy dan subcommand healthcheck (bila belum duplikat dengan `main`).
4. Penyelamatan SHAA-198: `intake.rs` dan `gerbang.rs` sebagai modul ber-test dengan gate hijau, belum dipasang ke handler.
5. Penyelamatan SHAA-202: client API, lalu login dan sesi.
6. Setelah PR penyelamatan dibuat, branch sumbernya juga diarsipkan dengan tag yang sama.

## Non-goals

- Desain UI v2 di Penpot dan konflik antara v2 dan ADR-0003 §7. Ini spec terpisah yang menunggu keputusan Operator.
- Handler `enqueueAsync`/`submitSync`, controller, worker, dan antrean apalis/RabbitMQ end-to-end.
- Mengubah atau menutup issue di workspace Shaal (read-only).
- Mengubah ADR. Rujukan "Shu sebagai lead Engineering" dan nama agen Shaal di ADR tetap sebagai riwayat.
- Mengaktifkan branch protection GitHub. Ini pengaturan repo yang dilakukan Operator, dicatat sebagai rekomendasi.
- Deploy (G3).

## Acceptance criteria

- AC-1: `CLAUDE.md` memuat aturan Arctiva: branch `agent/<ID>-<slug>`, PR wajib ke `main`, merge hanya oleh Operator (G2), gate `make setup test lint` wajib hijau di CI dan lokal sebelum PR siap review, tanpa force-push, tanpa secret, NIK hanya sintetis.
- AC-2: `make -C backend check-label` dan `make -C frontend check-label` lolos untuk `CONTAINER_LABEL=arctiva.issue=ARC-<n>` dan gagal (exit 2) untuk label lain, termasuk `shaal.issue=SHAA-<n>`. Ada test regresi di `scripts/test-dry-run.sh` atau yang setara.
- AC-3: `make push-main` keluar dengan pesan yang merujuk alur PR (tidak melakukan push).
- AC-4: Setelah arsip, `git ls-remote --heads origin` hanya berisi `main`, branch sumber yang PR-nya belum dibuat, dan branch kerja ARC. Setiap branch yang dihapus punya tag `archive/shaal/<nama>` yang menunjuk ke SHA ujung yang sama (diverifikasi dari inventaris).
- AC-5: PR SHAA-199 berisi `retry.rs` dan unit test-nya, dengan gate hijau.
- AC-6: PR SHAA-198 berisi `intake.rs` dan `gerbang.rs` dengan unit test untuk setiap kode masalah yang dipetakan (`validasi_gagal`, `metadata_tidak_valid`, `idempotency_key_tidak_didukung`, `idempotency_key_konflik`, `penolakan_berlebih`, `objek_nonaktif`, `layanan_tidak_tersedia`), dan tanpa `allow(dead_code)` di level crate.
- AC-7: PR SHAA-202 lolos `make -C frontend test lint build e2e`. Login, peringatan sesi, dan perpanjangan sesi punya test komponen dan uji axe.
- AC-8: Tidak ada commit yang langsung masuk `main` tanpa PR sejak AC-1 berlaku.

## Risiko

- `risk:high`: menghapus 62 branch remote (penghapusan data), serta menyentuh auth (login, sesi) dan alur penolakan request.
  Mitigasi: tag arsip dibuat dan diverifikasi sebelum penghapusan, dan inventaris SHA dicatat di repo.
- Kode SHAA-198 adalah WIP yang belum pernah melewati gate dan belum pernah di-review, jadi mungkin butuh perbaikan lebih dari sekadar cherry-pick.
- Diff penyelamatan melebihi target 400 baris (SHAA-198 sekitar 1.130, SHAA-202 sekitar 1.240). Tiket dipecah menjadi beberapa bagian, tetapi beberapa PR tetap di atas 400 karena kodenya sudah ada.
- Tanpa branch protection, AC-8 hanya ditegakkan oleh disiplin agen.

## Asumsi

- Token GitHub Executor bisa membuat tag dan menghapus branch di repo.
- Layar login SHAA-202 memakai gaya M3 v1 sampai keputusan desain v2 keluar.
- Belum ada tiket deploy, jadi tiket changelog dibuat bersama spec deploy nanti.
- Repo KB belum ditemukan. Spec dan decision log ini disimpan sebagai draft dan dipindah lewat PR begitu repo KB tersedia.
