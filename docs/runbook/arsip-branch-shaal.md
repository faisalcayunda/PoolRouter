## Ringkasan

Runbook ini mencatat arsip branch Shaal di repo PoolRouter. Pengembangan PoolRouter diambil alih Arctiva dari workspace Shaal yang sudah obsolete (spec `takeover-shaal`, approved 5 November 2026 WIB). Sebelum branch Shaal dihapus dari remote, setiap branch diarsipkan sebagai tag `archive/shaal/<nama-branch>` sehingga commit-nya tetap bisa diambil kembali kapan saja.

- **Pemilik:** Flik (Executor), auditor Klaus
- **Tanggal:** 5 November 2026 WIB
- **Status:** Arsip selesai dijalankan dan terverifikasi di remote.
- **Rujukan:** spec `products/poolrouter/specs/takeover-shaal/spec.md` dan decision log `products/poolrouter/grilling/2026-11-05-takeover-shaal.md`.

## Kondisi sebelum operasi

- `main` di `20a07eedc971a464d1deaf9e0663e0a8e5c9fd37`, tanpa branch protection.
- 65 branch remote selain `main`: 22 sudah ter-merge ke `main`, 43 belum.
- Belum ada tag di repo.

## Hasil akhir

| Butir | Nilai |
|---|---|
| Tag arsip dibuat | 65 (`62` branch dihapus + `3` branch sumber penyelamatan) |
| Branch remote dihapus | 62 |
| Branch remote dipertahankan | 3 (sumber pekerjaan penyelamatan) + `main` |
| `main` setelah operasi | tidak berubah pada saat operasi (`20a07ee`) |

Urutan operasi selalu **tag dulu, verifikasi, baru hapus branch**. Tag dibuat dalam satu `git push --atomic` supaya tidak ada arsip setengah jadi.

## Pemulihan

Ambil kembali isi satu branch yang sudah dihapus:

```sh
git fetch origin 'refs/tags/archive/shaal/*:refs/tags/archive/shaal/*'
git switch -c pulihkan/agent-flik-shaa-182 archive/shaal/agent/flik/shaa-182
```

Verifikasi satu tag menunjuk commit yang benar:

```sh
git rev-parse 'archive/shaal/agent/flik/shaa-182^{commit}'
```

Daftar seluruh tag arsip:

```sh
git ls-remote --tags origin | grep 'refs/tags/archive/shaal/'
```

## Prosedur verifikasi

Dijalankan sebelum branch dihapus, dan diulang setelahnya sebagai bukti akhir.

1. **Sumber kebenaran SHA.** Ambil SHA setiap branch langsung dari remote sebelum operasi: `git ls-remote --heads origin`. Hasilnya disimpan sebagai inventaris (tabel di bawah) dan menjadi pembanding tetap.
2. **Tag dibuat, lalu diverifikasi.** Untuk setiap branch, tag `archive/shaal/<nama-branch>` harus ada di remote **dan** SHA-nya sama persis dengan inventaris. Perbandingan dilakukan terhadap `git ls-remote --tags origin`, bukan terhadap ref lokal, supaya yang diperiksa benar-benar keadaan server.
3. **Hapus branch hanya bila langkah 2 lulus penuh.** Bila satu tag hilang atau SHA-nya berbeda, operasi berhenti dan tidak ada branch yang dihapus.
4. **Verifikasi ulang setelah penghapusan.** `git fetch` dari GitHub lalu `git rev-parse '<tag>^{commit}'` untuk setiap tag. Ini membuktikan objek commit masih ada setelah branch-nya hilang, bukan sekadar ref tag-nya.

Semua tag adalah lightweight tag yang menunjuk langsung ke commit branch (bukan tag anotasi), sehingga `^{commit}` selalu resolve ke commit aslinya. Pemeriksaan akhir: 65 tag, 0 tidak cocok.

## Inventaris branch

Kolom `status` menunjukkan apakah branch sudah menjadi ancestor `main` pada saat operasi. Kolom `tag` adalah tag arsip yang menunjuk commit tersebut. Tiga baris terakhir adalah branch sumber penyelamatan yang branch remote-nya **tidak** dihapus.

| branch | sha | status | tag | commit terakhir |
|---|---|---|---|---|
| `agent/camus/c5f7db72ffce` | `6e67bbfb6c2ae355e2eb6f36ec59b85ae786c95e` | merged | `archive/shaal/agent/camus/c5f7db72ffce` | 2026-11-04 — merge: origin/main into agent/camus/c5f7db72ffce (SHAA-389) |
| `agent/camus/f4737470f8b5` | `ef64ed4c087c2bd0a80a3cb9be418b290af67c06` | unmerged | `archive/shaal/agent/camus/f4737470f8b5` | 2026-11-04 — docs(runbook): record disk measurement provenance and concurrent lane... |
| `agent/camus/shaa-196` | `1af6a309eaf98e2b019a4989a7d808547a0f82bb` | unmerged | `archive/shaal/agent/camus/shaa-196` | 2026-11-04 — Merge remote-tracking branch 'origin/main' into agent/camus/cfc1446aa344 |
| `agent/camus/shaa-196-seed` | `23f1a45b396370eed9406806db7c2f29ba3552bc` | unmerged | `archive/shaal/agent/camus/shaa-196-seed` | 2026-11-04 — feat(core): idempotent dev/ci seed with one synthetic user per role (... |
| `agent/flik/0525d1f3f578` | `bded93126111c91a1eaa7017235a92b37d6bb1e9` | unmerged | `archive/shaal/agent/flik/0525d1f3f578` | 2026-11-04 — feat(webserver): admin probes, healthcheck subcommand, GCRA throttle,... |
| `agent/flik/0ad1d020f0d4` | `898c13e40912e9de740b7f9acbcbb6890f7e87dd` | merged | `archive/shaal/agent/flik/0ad1d020f0d4` | 2026-11-03 — Drop the internal F0 stage marker from client-facing text (SHAA-192) |
| `agent/flik/1de07cd9f988` | `0aea560f296a71e1065a2c2be8d9ef8fbc09d65e` | unmerged | `archive/shaal/agent/flik/1de07cd9f988` | 2026-11-03 — fix(db): grant EXECUTE on fn_set_status_kredensial to pr_admin_api (S... |
| `agent/flik/50fe785795f9` | `8baced37482dd95a1c4f8235e7c84c9da8803762` | merged | `archive/shaal/agent/flik/50fe785795f9` | 2026-11-03 — docs(openapi): fix 422 examples and client-facing descriptions (SHAA-... |
| `agent/flik/54849fc37929` | `370c69255b7591681fcb8ca5ee4453a4050e22b7` | merged | `archive/shaal/agent/flik/54849fc37929` | 2026-11-04 — feat(lua): reserve the quota slot in the token script (SHAA-195) |
| `agent/flik/7d3a41146cef` | `de886f2ceddea9c91abaf31a7f2cdc0f2644e607` | unmerged | `archive/shaal/agent/flik/7d3a41146cef` | 2026-11-04 — Merge origin/main into agent/flik/7d3a41146cef (SHAA-391) |
| `agent/flik/8ab2c9a87f87` | `5dcf4853c0d5f13f10230a27ffc3aef4e36aa9a0` | merged | `archive/shaal/agent/flik/8ab2c9a87f87` | 2026-11-03 — fix(db): grant EXECUTE on fn_set_status_kredensial to pr_admin_api (S... |
| `agent/flik/960984443c55` | `8b4365cdc765672af15117227b482478388c008d` | unmerged | `archive/shaal/agent/flik/960984443c55` | 2026-11-03 — Add OpenAPI contract design inventory and complete failure/cancel rea... |
| `agent/flik/b31a3409c044` | `9e766ceb3056476525a9121b567ddc9f3a6b12f6` | merged | `archive/shaal/agent/flik/b31a3409c044` | 2026-11-04 — chore(openapi): regenerate admin-api.yaml without kuota/rate maximums... |
| `agent/flik/cb185e0aaf6e` | `44dcdf2057a32315361920949c95cf5e02766c10` | merged | `archive/shaal/agent/flik/cb185e0aaf6e` | 2026-11-04 — ci(security): gitleaks allowlist uses the full literal synthetic JWT ... |
| `agent/flik/shaa-182` | `0dfc8e65aebb1aebdc56f30f51618818d1ad5cd2` | merged | `archive/shaal/agent/flik/shaa-182` | 2026-11-03 — Add owner decision K-34 reference to CLAUDE.md (SHAA-182) |
| `agent/flik/shaa-192-contract` | `76e5d396dbfa6b7c3122de5d7ed361a2670fdb53` | unmerged | `archive/shaal/agent/flik/shaa-192-contract` | 2026-11-03 — Add docs/masalah.md problem-code doc with ProblemKode anchor check (S... |
| `agent/flik/shaa-193` | `026e7d9991ce7e0defb657ce970d4bcc50dd81f6` | unmerged | `archive/shaal/agent/flik/shaa-193` | 2026-11-03 — Add function tests, db-test target, and schema mapping doc (SHAA-193) |
| `agent/flik/shaa-195-a` | `47c83a66900808b601a7f252b6b150a722de8105` | merged | `archive/shaal/agent/flik/shaa-195-a` | 2026-11-04 — feat(kuota): module metrics from the section 5a.9 catalogue (SHAA-195) |
| `agent/flik/shaa-280` | `17199c6ad6cd8bd99c7bc5d98d8c8fcc329a4803` | merged | `archive/shaal/agent/flik/shaa-280` | 2026-11-03 — fix(admin): match UpstreamPoolConfig to Shu's final KP-01 table (SHAA... |
| `agent/gengen/6cc00d19cdc9` | `8a504b7350db55891382142c46eeb58147c74af1` | unmerged | `archive/shaal/agent/gengen/6cc00d19cdc9` | 2026-11-04 — test(f1): note that the envelope format is a test-only shape (SHAA-33... |
| `agent/hans/8111494a9b77` | `c6829a25dff8cb964e8e4156a46ff6f4dcc5fd21` | merged | `archive/shaal/agent/hans/8111494a9b77` | 2026-11-04 — Sync Cargo.lock with queue-apalis uuid dependency (SHAA-320) |
| `agent/jowy/646f525076e6` | `6bee283a3a4f240a5bec6bd310818822a774754f` | unmerged | `archive/shaal/agent/jowy/646f525076e6` | 2026-11-03 — test(db): T-04 negative test must be rejected by T-04 itself, add con... |
| `agent/jowy/6dbf2b4e15ed` | `5fa8f2de6fa727c500728e3c2111eb4ff11b4a2f` | unmerged | `archive/shaal/agent/jowy/6dbf2b4e15ed` | 2026-11-04 — feat(db): apply design ruling on SHAA-354 (ttl_polling NULL on reject... |
| `agent/jowy/94bace323fe9` | `fc1431d27ca35ab498e897bf7b93c24de2266e09` | merged | `archive/shaal/agent/jowy/94bace323fe9` | 2026-11-04 — Merge origin/main into F3 schema branch (SHAA-354) |
| `agent/jowy/a8c4a53a4f39` | `1429eaf74a4e94cadd9a12e62d01dd8eea8c8f54` | merged | `archive/shaal/agent/jowy/a8c4a53a4f39` | 2026-11-04 — fix(db): remove webhook_not_before from pr_api/worker/controller; col... |
| `agent/jowy/b6b1977a5652` | `7e15e0e764b9375c691a2915437e0d169f824a2b` | merged | `archive/shaal/agent/jowy/b6b1977a5652` | 2026-11-03 — test(db): T-04 negative test must be rejected by T-04 itself, add con... |
| `agent/jowy/e7ac7c53762f` | `7291197ae7af1016ce4d7cf9f89abbb0c57dbd27` | unmerged | `archive/shaal/agent/jowy/e7ac7c53762f` | 2026-11-03 — fix(db): T-04 trigger compares history against OLD/TG_OP; add committ... |
| `agent/nanami/shaa-330` | `4b67916497b68a809fb12f37470a431b3405e507` | unmerged | `archive/shaal/agent/nanami/shaa-330` | 2026-11-04 — docs(i18n): apply approved v0.15 copy decisions |
| `agent/shiro/987f3b06ae9c` | `f665a8da6063743743f685b3bf4a1249bee34387` | unmerged | `archive/shaal/agent/shiro/987f3b06ae9c` | 2026-11-04 — chore(monitoring): move F1 alert tests under compose monitoring dir (... |
| `agent/shiro/shaa-361` | `2a23135ba6a2b7e7415e201238c4dbfc59d27b4c` | unmerged | `archive/shaal/agent/shiro/shaa-361` | 2026-11-04 — fix(monitoring): point archive alert runbook anchors to arsip-kek.md ... |
| `agent/sid/398ee6eb84c7` | `f22dcabe2f0c1410cf69f2b6aef427a9312fd3e2` | unmerged | `archive/shaal/agent/sid/398ee6eb84c7` | 2026-11-03 — Add backend Containerfile and .dockerignore (SHAA-207) |
| `agent/sid/772b0fa03c74` | `a53920befc81cb31fa5e20e2a2cb40620ef77e14` | unmerged | `archive/shaal/agent/sid/772b0fa03c74` | 2026-11-04 — Merge origin/main into SHAA-381 branch |
| `agent/sid/b9460b1aff16` | `d2a4460a14e5ed270447cf512ee7a172d915e68e` | unmerged | `archive/shaal/agent/sid/b9460b1aff16` | 2026-11-04 — fix(compose): rotasi job reads old KEK private key from per-id direct... |
| `agent/sid/d33e00eb2846` | `1fddce1985b24cd5553fbfeebc51fd462b950067` | unmerged | `archive/shaal/agent/sid/d33e00eb2846` | 2026-11-04 — ops(lane): fix docker frontend gate under host uid and compare GC aga... |
| `agent/stallion/7380f6ff0a7f` | `584a2c6e789cba4ed5e6ddcc4c653826ecca28ac` | unmerged | `archive/shaal/agent/stallion/7380f6ff0a7f` | 2026-11-03 — perf: measure actual mock latency for overhead and split 502/504/429 ... |
| `agent/templeton/19d89955cd77` | `b30d876f4e2c558007210b7eb0478139f0e422e3` | unmerged | `archive/shaal/agent/templeton/19d89955cd77` | 2026-11-04 — docs(panduan): ganti pemilik dan status untuk pembaca, hapus rujukan ... |
| `agent/tessai/5a1e30c1f6c1` | `bc794835a00518ca4336b48671efef1ff3a17051` | merged | `archive/shaal/agent/tessai/5a1e30c1f6c1` | 2026-11-04 — Merge origin/main into agent/tessai/5a1e30c1f6c1 (SHAA-395): keep FIF... |
| `agent/tessai/5a960af434bd` | `ffa75fa8992fc097ce3d3adfa077962af1d22836` | unmerged | `archive/shaal/agent/tessai/5a960af434bd` | 2026-11-04 — build(gate): setup under heavy lock, shared cargo-home size guard, cl... |
| `agent/tessai/5d2d34069511` | `d87c3c0b7a9ff8dbfb76521c1c79d8160242014d` | unmerged | `archive/shaal/agent/tessai/5d2d34069511` | 2026-11-04 — fix(ci): size_mb toleran berkas hilang saat du memindai, selalu satu ... |
| `agent/tessai/76ca4e0bc651` | `774f13a72a06897e7d7508d861c21053295ef35e` | unmerged | `archive/shaal/agent/tessai/76ca4e0bc651` | 2026-11-03 — ci(backend): add backend-image workflow for GHCR push (SHAA-207) |
| `agent/tessai/a13d98830b1e` | `9e5b5f02a6d7d6a5891c557b53399dc2cf7231fa` | merged | `archive/shaal/agent/tessai/a13d98830b1e` | 2026-11-03 — fix(ci): keep env OASDIFF_BASE_SOURCE via ?= with := default label (S... |
| `agent/tessai/c797ffa46bd4` | `59f7d75e87e7cb288d386ceb726922bc8a42873c` | merged | `archive/shaal/agent/tessai/c797ffa46bd4` | 2026-11-04 — feat(ci): add make push-main, push to main only after the full gate (... |
| `agent/tessai/d8490edad481` | `86dc9552864027e56bb3d1058cbbef35e17288ab` | unmerged | `archive/shaal/agent/tessai/d8490edad481` | 2026-11-04 — fix(gate): cargo target in subdir of mount so clean-shared works; pro... |
| `agent/tessai/shaa-398` | `008cb510e406fb4d5c922f31b22f9b302c385990` | merged | `archive/shaal/agent/tessai/shaa-398` | 2026-11-04 — fix(gate): make -n tidak menjalankan container; SUBMAKE, uji regresi,... |
| `agent/tessai/shaa-400` | `faec9318a506cf7b3943ea0214a2c7e9891a8635` | merged | `archive/shaal/agent/tessai/shaa-400` | 2026-11-04 — Merge origin/main into agent/tessai/a1a310500b0e (SHAA-400) |
| `agent/tessai/shaa-401` | `a8c64161bee453c04d220f0ce6bf77d21fcc93a8` | unmerged | `archive/shaal/agent/tessai/shaa-401` | 2026-11-04 — fix(build): isolasi artefak crate workspace antar-lane Docker dengan ... |
| `agent/tetsu/3e25f3a9e1bb` | `479512be0743a0cc003b40a446a41f2810fc4cb8` | unmerged | `archive/shaal/agent/tetsu/3e25f3a9e1bb` | 2026-11-04 — build: portable free-space check in docker-lane (SHAA-373) |
| `agent/tetsu/543ac48b86e2` | `b1c22988ad6de1c9116c403648d166d795450cee` | unmerged | `archive/shaal/agent/tetsu/543ac48b86e2` | 2026-11-03 — Use webserver healthcheck subcommand in compose (SHAA-191) |
| `agent/tetsu/9678a34101da` | `76de01762a78a0544f0480e8ef548dd5d206df18` | unmerged | `archive/shaal/agent/tetsu/9678a34101da` | 2026-11-04 — feat(compose): worker IAM without delete and egress placeholder (SHAA... |
| `agent/tetsu/b728b287d398` | `a0f01ad760366cc291450935e7a767253c68499d` | merged | `archive/shaal/agent/tetsu/b728b287d398` | 2026-11-03 — Update toolchain runbook: VM size section, owner-only resize, CI alig... |
| `agent/tetsu/e9bbd3deaf45` | `ddc1f031a47133a0ad88d4660c1bba6bbef3cdae` | unmerged | `archive/shaal/agent/tetsu/e9bbd3deaf45` | 2026-11-04 — feat(compose): split archive and restore workers with KEK secrets (SH... |
| `agent/tetsu/f0a501f76b77` | `e9d78b4eef16d448b23a9eb2c2c2788b871f851f` | unmerged | `archive/shaal/agent/tetsu/f0a501f76b77` | 2026-11-03 — Add guarded SSD cargo target helper and runbook (SHAA-274) |
| `agent/tetsu/label-guard` | `9666b962b9de840cfa46cec5a2688920e1afb4f0` | merged | `archive/shaal/agent/tetsu/label-guard` | 2026-11-03 — Require shaal.issue label for podman toolchain containers (SHAA-222) |
| `agent/tetsu/shaa-373-rebase` | `a917be3f631ea8897c364e253aae2e62c6c50933` | unmerged | `archive/shaal/agent/tetsu/shaa-373-rebase` | 2026-11-04 — Merge remote-tracking branch 'origin/main' into agent/tetsu/shaa-373-... |
| `agent/viktor/3c53de4782d7` | `ca0bfd09548e124c1e3aa2a49303ea78110e051d` | merged | `archive/shaal/agent/viktor/3c53de4782d7` | 2026-11-04 — Regenerate schema.gen.ts from admin-api.yaml (SHAA-281) |
| `agent/viktor/62cc36204e49` | `8c41c83498d8b93068be1871351b514fc21a2a94` | unmerged | `archive/shaal/agent/viktor/62cc36204e49` | 2026-11-03 — WIP: frontend scaffold, temporary M3 tokens, i18n core (unverified, i... |
| `agent/viktor/646384e7223c` | `3942bd1ae9dd395301c2cbb8565f332e3761b1c5` | unmerged | `archive/shaal/agent/viktor/646384e7223c` | 2026-11-04 — test(frontend): drop wall-clock coupling in nik-auditor.test.tsx (SHA... |
| `agent/viktor/67aad2c46333` | `c44fa328575bebec2d915baa9945f7a187089b5c` | unmerged | `archive/shaal/agent/viktor/67aad2c46333` | 2026-11-04 — fix(frontend): keep wide tables inside the viewport at 1024 (SHAA-203) |
| `agent/viktor/af2400712fb0` | `8433ec41817435ac6bc3a4108e64a34e9d4bcf47` | unmerged | `archive/shaal/agent/viktor/af2400712fb0` | 2026-11-04 — test(frontend): cover the S11 auditor payload path and payload no-sto... |
| `agent/yuzu/23fde654d73f` | `fa3a03c22e3c8339f272f0f933ed25fd3bd2f1fd` | unmerged | `archive/shaal/agent/yuzu/23fde654d73f` | 2026-11-04 — test(f1): bind webhook clients to test owners and refresh fixtures (S... |
| `agent/yuzu/shaa-204-k20-fixture` | `9e7f9e9eb0eeeef4cb976c54b1925145df6d5b6f` | unmerged | `archive/shaal/agent/yuzu/shaa-204-k20-fixture` | 2026-11-04 — test(db): pin argon2 and take K-20 login password from env (SHAA-204) |
| `agent/yuzu/shaa-255-k20` | `ea9da205014b1e5c74308ea6638687f93af2f327` | unmerged | `archive/shaal/agent/yuzu/shaa-255-k20` | 2026-11-03 — test(db): rebuild K-20 fixture k20.sql as deterministic synthetic dat... |
| `agent/flik/a62ee1a1e75a` | `45b5a4e7a010dcb9051910a6d5294fcff205d57f` | sumber penyelamatan, branch dipertahankan | `archive/shaal/agent/flik/a62ee1a1e75a` | 2026-11-04 — WIP(webserver): intake validation and the rejection gate (SHAA-198) |
| `agent/flik/e050bf1448d4` | `3c9c04a0f7666cfd0ac7892089e88d073b3ad64d` | sumber penyelamatan, branch dipertahankan | `archive/shaal/agent/flik/e050bf1448d4` | 2026-11-04 — feat(core): add retry policy with full-jitter backoff and error_class... |
| `agent/viktor/a7f1ba11e6e5` | `70f88d761a1c5df2f7ab9a0cbe98f9a6869d1189` | sumber penyelamatan, branch dipertahankan | `archive/shaal/agent/viktor/a7f1ba11e6e5` | 2026-11-04 — test(frontend): skip link targets the #konten landmark from spec §13.... |


## Catatan

- `agent/flik/0525d1f3f578` tercatat `unmerged` karena tip-nya bukan ancestor `main`, tetapi isinya sudah masuk `main` lewat `3a2e0fa`. Tag arsipnya tetap menunjuk tip asli `bded931`, jadi tidak ada yang hilang dari sisi mana pun.
- Tiga branch sumber penyelamatan tetap ada di remote karena masih dipakai untuk PR SHAA-198, SHAA-199, dan SHAA-202. Tag arsipnya dibuat lebih awal sebagai jaring pengaman bila branch tersebut dihapus setelah PR-nya merge.
