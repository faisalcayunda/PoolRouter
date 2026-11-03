## Ringkasan

Runbook ini menjelaskan cara menyalakan, memeriksa, dan membersihkan stack PoolRouter untuk lingkungan `dev`, `staging`, dan `prod` memakai Docker Compose v2. Dev berjalan sebagai container Podman di mesin owner lewat provider P3 (`deploy/compose/bin/compose-provider`). Staging dan produksi berjalan di Docker Engine di host target yang belum diputuskan (K-38). Dasar keputusan ada di ADR-0001 Addendum A.

- **Pemilik:** Tetsu (Platform & SRE), reviewer Huan
- **Tanggal:** 2026-11-03
- **Status:** draft, menunggu review Huan. Bagian staging dan prod masih bergantung pada target host.

## Prasyarat

- Podman 5.0 atau lebih baru dengan `podman machine` yang sudah berjalan (di mesin owner: Podman 6.1.2). Podman dan machine disediakan owner; jangan dipasang, di-upgrade, atau diubah dari runbook ini.
- Tidak ada instalasi native. Jangan memasang `docker-compose`, `podman-compose`, atau Docker Engine di mesin owner. Provider Compose berjalan dari image `docker.io/library/docker:27-cli` yang di-pin dengan digest.
- Di staging dan prod: Docker Engine 27 atau lebih baru dan Docker Compose 2.30 atau lebih baru di host target, serta akses pull ke GHCR privat dengan token read-only yang disimpan di credential store engine (`docker login`), bukan di repo.
- Direktori secret di luar repo, mode `0700`, berisi file mode `0444`: `db_password`, `redis_password`, `rabbitmq_password`, `poolrouter_owner`, `poolrouter_db_app`, `poolrouter_kek`. Isi file tidak pernah ditulis di issue, komentar, atau repo.
- File env lingkungan di luar repo. Template ada di `deploy/compose/env/<env>.env.example`. File sungguhan (`deploy/compose/env/<env>.env` atau path lain) tidak di-commit.

## Layout file

| Lokasi | Isi |
|---|---|
| `deploy/compose/compose.yaml` | Definisi dasar: profile `bundled-infra` (PostgreSQL 17, Redis 7, RabbitMQ 4), `migrate`, `app` (backend, frontend), secrets |
| `deploy/compose/compose.dev.yaml` | Build dari source, mock Dukcapil (profile `mock`), seed sintetis |
| `deploy/compose/compose.staging.yaml` | Image berdasarkan digest, `restart`, batas resource, rotasi log, mock (profile `mock`) |
| `deploy/compose/compose.prod.yaml` | Image berdasarkan digest, `restart`, batas resource, rotasi log. Tidak ada mock |
| `deploy/compose/env/*.env.example` | Template variabel tanpa nilai rahasia |
| `deploy/compose/bin/compose-provider` | Provider P3: Docker Compose v2 dari container, terhubung ke socket API Podman |
| `deploy/compose/lint/compose-lint.sh` | Penjaga V1 (fixture di `lint/fixtures/`) |
| `deploy/compose/test/dind.sh` | Harness DinD untuk bukti V3 (container Docker sementara) |
| `Makefile` | Satu-satunya entry point |

## Perintah

Semua perintah dijalankan dari akar repo. Variabel:

- `ENV`: `dev`, `staging`, atau `prod` (default `dev`)
- `CONTAINER_ENGINE`: `podman` (default) atau `docker`
- `ENV_FILE`: path file env. Default `deploy/compose/env/<ENV>.env`. Untuk staging dan prod wajib diisi path di luar repo.
- `COMPOSE_PROFILES`: default `bundled-infra`. Tambahkan `app` dan `mock` bila diperlukan, dipisah spasi. Bila memakai `app`, pastikan image digest sudah tersedia.
- `EXTRA_COMPOSE_FILES`: override tambahan, misalnya label harness uji.

### Menyalakan

```sh
make compose-up ENV=dev ENV_FILE=deploy/compose/env/dev.env CONTAINER_ENGINE=podman
```

`compose-up` menjalankan `up -d --wait`, jadi perintah ini selesai hanya setelah service yang punya healthcheck berstatus `healthy`, atau gagal dengan exit code 1.

### Memeriksa

```sh
make compose-ps ENV=dev ENV_FILE=deploy/compose/env/dev.env
make compose-logs ENV=dev ENV_FILE=deploy/compose/env/dev.env
make compose-config ENV=dev ENV_FILE=deploy/compose/env/dev.env   # validasi render saja
```

Healthcheck bawaan:

- `postgres`: `pg_isready` terhadap `POSTGRES_USER` dan `POSTGRES_DB`
- `redis`: `redis-cli ping` dengan password dari secret
- `rabbitmq`: `rabbitmq-diagnostics -q ping`
- `backend-webserver`: `poolrouter healthcheck` (probe lokal; sebelum subcommand itu ada di image, service ini belum bisa `healthy`)
- `frontend`: `wget` ke port 8080 di dalam container

### Upgrade dan rollback (staging dan prod)

Deploy selalu memakai digest, bukan tag:

1. Ubah baris `POOLROUTER_BACKEND_IMAGE`, `POOLROUTER_FRONTEND_IMAGE`, dan image lain di file env ke `ghcr.io/faisalcayunda/poolrouter-<nama>@sha256:<digest>`. Digest berasal dari hasil CI untuk commit yang sudah di-review (tag `sha-<12 hex>` atau `vX.Y.Z`).
2. Jalankan `make compose-lint` terhadap file env yang akan dipakai, atau cukup `make compose-config ENV=<env> ENV_FILE=<file>`.
3. `make compose-up ENV=<env> ENV_FILE=<file>`.
4. Rollback: kembalikan baris digest ke nilai sebelumnya lalu jalankan `compose-up` lagi. Digest lama tetap ada di registry karena tag `sha-*` disimpan sesuai retensi ADR-0001 Addendum §7.

Migrasi skema berjalan terpisah:

```sh
podman compose ... --profile migrate run --rm migrate
```

Lewat Makefile, tambahkan `COMPOSE_PROFILES=migrate` ke `compose-up` yang hanya menjalankan service migrate. Pastikan `bundled-infra` juga aktif supaya database tersedia.

### Membersihkan

```sh
make compose-down ENV=dev ENV_FILE=deploy/compose/env/dev.env
```

`compose-down` menghentikan dan menghapus container, network, dan project, tetapi volume tetap ada. Untuk menghapus volume data dev:

```sh
podman volume ls --filter label=shaal.issue=<SHAA-n>
```

Volume dev di mesin owner tidak berlabel `shaal.issue` karena stack ini dipakai owner. Hapus volume data hanya lewat keputusan owner. Jangan menjalankan `podman system prune`, `podman system reset`, atau `podman machine stop|rm|reset`.

Setelah run agent, bersihkan resource berlabel issue dengan tiga perintah berikut, berurutan:

```sh
podman rm -f -v --filter label=shaal.issue=<SHAA-n>
podman volume prune -f --filter label=shaal.issue=<SHAA-n>
podman network prune -f --filter label=shaal.issue=<SHAA-n>
```

### Compose lint (V1)

```sh
make compose-lint
```

Perintah ini menjalankan self-test (setiap fixture pelanggaran harus gagal), merender `compose config` untuk dev, staging, dan prod, lalu memeriksa:

- pola terlarang (ADR-0001 Addendum §5): `container_name`, `userns_mode`, `:U`, `privileged`, `cap_add`, `network_mode: host`, `include`, `extends`, `develop`, `watch`, deploy mode Swarm, driver logging selain `json-file`, dan lain-lain
- image di staging dan prod harus berdigest, dan tidak boleh ada `build:`
- `mock-dukcapil` dan seed sintetis tidak boleh ada di render prod
- Containerfile: tanpa `HEALTHCHECK`, `SHELL`, `ONBUILD`, directive `# syntax`, flag BuildKit, `FROM` tanpa digest, dan user akhir harus `10001:10001`. `.dockerignore` wajib ada di sebelah setiap Containerfile

Exit code 0 berarti lulus. Hasil render disimpan di `deploy/compose/lint/.render/` (diabaikan git).

### Harness DinD (V3)

```sh
make compose-test-dind SHAA_ISSUE=SHAA-<n> ENV_FILE=<file env di luar repo>
```

Harness menjalankan `docker:27-dind` dengan `--privileged` sebagai container sementara berlabel `shaal.issue`, lalu menaikkan stack `bundled-infra` lewat klien Compose di container CLI. Di akhir run, harness menghapus stack di dalam DinD dan tiga perintah pembersihan di atas. Ini satu-satunya tempat `privileged` dalam repo, dan hanya untuk bukti.

Jika DinD tidak bisa berjalan di Podman rootless, catat kegagalannya di komentar issue. Bukti V4 di GitHub Actions menggantikan V3 (fallback ADR-0001 Addendum §8).

## Secret dan file env

- Secret dibaca dari file lewat `secrets: file:` di `POOLROUTER_SECRETS_DIR`. Di container tampil sebagai `/run/secrets/<nama>`.
- Hak akses: direktori `0700` milik user deploy, file `0444`. Pola ini dipilih agar UID 10001 bisa membaca file di Podman rootless maupun Docker rootful tanpa `chown`.
- Provider P3 hanya bisa membaca file env dan secret yang berada di path yang dipasang ke container provider (lewat `POOLROUTER_EXTRA_MOUNTS`, dipasang read-only). Makefile mengatur ini otomatis dari `ENV_FILE` dan `POOLROUTER_SECRETS_DIR`.
- Batasan yang diketahui: password RabbitMQ diekspor di shell entrypoint dan Redis dibaca lewat `$(cat ...)` di perintah. Keduanya terlihat oleh user yang punya akses ke container yang sama, tetapi tidak di `podman inspect`. Konfigurasi file untuk RabbitMQ 4 menggantikan cara ini setelah ada keputusan platform.
- Dilarang: menulis nilai secret di repo, issue, komentar, `environment:`, atau build arg.

## Staging dan prod: bergantung target host

Bagian ini belum final. Keputusan berikut menunggu owner dan ADR infrastruktur terpisah (ADR-0001 Addendum §9):

- Host target dan arsitektur CPU (asumsi `linux/amd64`, Q-A2).
- Bundled atau managed PostgreSQL, Redis, dan RabbitMQ (`POOLROUTER_DB_HOST`, `POOLROUTER_REDIS_HOST`, `POOLROUTER_RABBITMQ_HOST` kosong di template prod).
- Reverse proxy atau load balancer di depan port 8080, serta TLS.
- Registry mirror BGN untuk produksi (digest disalin dengan `skopeo copy --preserve-digests`).
- Restart setelah reboot host dan rotasi log di Docker rootful (sudah diatur di compose; verifikasi di host target).

Sampai keputusan itu, `make compose-up ENV=staging` dan `ENV=prod` hanya boleh dijalankan di host target yang sudah disetujui owner.

## Batasan saat ini

- Service backend, frontend, dan mock Dukcapil sudah didefinisikan, tetapi image-nya dibangun di Bagian 2 (SHAA-207 Containerfile, Sid dan Tessai). Sampai itu, profile `app` dan `mock` tidak bisa dinyalakan.
- Subcommand `poolrouter healthcheck` dan konvensi `*_FILE` ada di lingkup backend SHAA-175 (Flik). Sampai ada, healthcheck backend tidak akan `healthy`.
- Nama variabel `POOLROUTER_DB_*`, `POOLROUTER_REDIS_*`, dan `POOLROUTER_RABBITMQ_*` adalah usulan dari sisi deploy. Backend harus mengikutinya; perbedaan dicatat di issue backend.
- Job CI V4 (`compose-portability`) diserahkan ke Tessai.

## Rujukan

- ADR-0001 Addendum A (`docs/adr/0001-addendum-a-deploy-engine-agnostic.md` setelah masuk main; sampai saat itu lampiran di SHAA-190)
- SHAA-207 (sub-issue K2), SHAA-190 (induk K-38)
- `shaal-playbook` bagian 6, `shaal-eng-standards` bagian 8
