## Ringkasan

Runbook ini menjelaskan cara menyalakan, memeriksa, mengupgrade, dan membersihkan stack PoolRouter untuk lingkungan `dev`, `staging`, dan `prod` memakai Docker Compose v2. Dev berjalan sebagai container Podman di mesin owner lewat provider P3 (`deploy/compose/bin/compose-provider`). Staging F0 adalah lane CI efemeral `staging-smoke` di runner `ubuntu-latest` (Docker Engine), bukan host persisten. `ENV=prod` dan `compose.prod.yaml` dipakai untuk lingkungan PoC BGN, dan produksi F1 berjalan di Kubernetes. Dasar keputusan ada di ADR-0001 §8.3 dan ADR-0009 (KB `products/poolrouter/adr/0009-definisi-f0-beres-dan-platform-staging.md`) serta kontrak env backend v1.

- **Pemilik:** Tetsu (Platform & SRE), reviewer Huan
- **Tanggal:** 2026-11-03
- **Status:** Bagian 1 disetujui Huan @ `6ee9bb7`; staging F0 dan platform produksi mengikuti ADR-0009.

## Prasyarat

- Podman 5.0 atau lebih baru dengan `podman machine` yang sudah berjalan (di mesin owner: Podman 6.1.2). Podman dan machine disediakan owner; jangan dipasang, di-upgrade, atau diubah dari runbook ini.
- Tidak ada instalasi native. Jangan memasang `docker-compose`, `podman-compose`, atau Docker Engine di mesin owner. Provider Compose berjalan dari image `docker.io/library/docker:27-cli` yang di-pin dengan digest.
- Di staging F0 (`staging-smoke`) dan lingkungan PoC BGN (`ENV=prod`): Docker Engine 27 atau lebih baru dan Docker Compose 2.30 atau lebih baru. Lane staging memakai Docker Engine bawaan runner `ubuntu-latest`; tidak ada host persisten. Host PoC BGN ditetapkan di Gerbang PoC (ADR-0009 §2.4 P-1). Akses pull ke GHCR privat memakai token read-only yang disimpan di credential store engine (`docker login`), bukan di repo.
- Direktori secret di luar repo, mode `0700`, berisi file mode `0444`. Nama file = nama secret: `db_password`, `redis_password`, `rabbitmq_password`, `poolrouter_owner`, `pr_api`, `pr_admin_api`, `pr_controller`, `pr_webhook`, `pr_worker`, `poolrouter_kek`, `poolrouter_jwt_signing_key`, `poolrouter_nik_hmac_key`, `poolrouter_idempotency_hmac_key`, dan `poolrouter_seed_password` (dev saja). Isi file tidak pernah ditulis di issue, komentar, atau repo.
- Verifikasi mode sebelum `compose-up` (Linux: `stat -c '%a %n'`; macOS: `stat -f '%Lp %N'`):

  ```sh
  stat -f '%Lp %N' "$POOLROUTER_SECRETS_DIR"          # harus 700
  stat -f '%Lp %N' "$POOLROUTER_SECRETS_DIR"/*         # setiap file harus 444
  ```

  Isi file tidak ditampilkan. Jangan `cat` file secret di terminal yang tercatat.
- File env lingkungan di luar repo. Template ada di `deploy/compose/env/<env>.env.example`. File sungguhan tidak di-commit.

## Layout file

| Lokasi | Isi |
|---|---|
| `deploy/compose/compose.yaml` | Definisi dasar: profile `bundled-infra` (PostgreSQL 17, Redis 7, RabbitMQ 4), `migrate`, `app` (backend, frontend), secrets per role |
| `deploy/compose/compose.dev.yaml` | Build dari source, mock Dukcapil (profile `mock`), seed sintetis, `POOLROUTER_DB_SSLMODE=disable` |
| `deploy/compose/compose.staging.yaml` | Image berdasarkan digest, `restart`, batas resource, rotasi log, mock (profile `mock`) |
| `deploy/compose/compose.prod.yaml` | Image berdasarkan digest, `restart`, batas resource, rotasi log, tiga service infra dengan limit yang sama dengan staging. Tidak ada mock |
| `deploy/compose/env/*.env.example` | Template variabel tanpa nilai rahasia |
| `deploy/compose/bin/compose-provider` | Provider P3: Docker Compose v2 dari container, terhubung ke socket API Podman |
| `deploy/compose/lint/compose-lint.sh` | Penjaga V1 (fixture di `lint/fixtures/`, satu pelanggaran per fixture dan label `# expect:`) |
| `deploy/compose/test/dind.sh` | Harness DinD untuk bukti V3 (container Docker sementara) |
| `Makefile` | Satu-satunya entry point. Tidak ada perintah `podman compose` atau `docker compose` langsung di runbook |

## Perintah

Semua perintah dijalankan dari akar repo. Variabel:

- `ENV`: `dev`, `staging`, atau `prod` (default `dev`)
- `CONTAINER_ENGINE`: `podman` (default) atau `docker`
- `ENV_FILE`: path file env. Default `deploy/compose/env/<ENV>.env`. Untuk staging dan prod wajib diisi path di luar repo.
- `COMPOSE_PROFILES`: aturan per lingkungan:
  - `dev`: default `bundled-infra`.
  - `staging` dan `prod`: **wajib diisi eksplisit**. Untuk database bundled isi `bundled-infra`. Untuk endpoint managed isi profile yang benar-benar dipakai (tanpa `bundled-infra`), misalnya `app` setelah Bagian 2. `make` gagal di `check-env` bila nilainya kosong.
  - Tambahkan `migrate` lewat `compose-migrate`, bukan lewat `COMPOSE_PROFILES`.
- `EXTRA_COMPOSE_FILES`: override tambahan, misalnya label harness uji.

### Memeriksa konfigurasi (wajib sebelum up)

```sh
make compose-config ENV=<env> ENV_FILE=<file env yang akan dipakai>
```

Perintah ini merender file Compose dengan **file env yang akan dipakai**, bukan template. Gagal jika ada variabel wajib yang kosong. Jalankan sebelum setiap `compose-up`. `compose-lint` adalah pemeriksaan terpisah yang merender dari template `env/*.env.example`; ia tidak memeriksa nilai di file env Anda.

### Menyalakan

```sh
make compose-up ENV=dev ENV_FILE=deploy/compose/env/dev.env CONTAINER_ENGINE=podman
```

`compose-up` menjalankan `up -d --wait`, jadi perintah ini selesai hanya setelah service yang punya healthcheck berstatus `healthy`, atau gagal dengan exit code 1.

Urutan lengkap dev:

1. `make compose-config` (lihat di atas).
2. `make compose-up` (profile `bundled-infra`). Tunggu sampai ketiga service infra `healthy`.
3. `make compose-migrate` (profile `migrate`, `run --rm`). Jalankan setelah langkah 2 supaya database sudah ada. Migrasi tidak berjalan otomatis saat `compose-up`.
4. Service aplikasi (profile `app`) baru naik setelah Bagian 2 (lihat Batasan).

### Memeriksa

```sh
make compose-ps ENV=dev ENV_FILE=deploy/compose/env/dev.env
make compose-logs ENV=dev ENV_FILE=deploy/compose/env/dev.env
```

Healthcheck bawaan:

- `postgres`: `pg_isready` terhadap `POSTGRES_USER` dan `POSTGRES_DB`
- `redis`: `redis-cli ping` dengan `REDISCLI_AUTH` dari secret (tidak lewat argumen)
- `rabbitmq`: `rabbitmq-diagnostics -q ping`
- `backend-webserver`: `webserver healthcheck` (probe lokal lewat subcommand binary, ADR-0001 §11.2). Subcommand ini ada di lingkup SHAA-175 (Flik). Sampai ada di image, service ini tidak akan `healthy`. Probe TCP tidak dipakai: image runtime tidak punya shell atau `nc`, dan probe TCP akan memasukkan alat yang harus dicabut lagi. `backend-controller` dan `backend-worker` belum punya healthcheck; ditambahkan di Bagian 2.
- `frontend`: `wget` ke port 8080 di dalam container. Verifikasi pada Bagian 2 bahwa image nginx-unprivileged memiliki `wget`.

### Upgrade (staging dan prod)

Deploy selalu memakai digest, bukan tag.

1. **Catat digest yang sedang berjalan sebelum upgrade.** Simpan salinan file env dengan tanggal di direktori `0700`, di luar repo:

   ```sh
   install -d -m 0700 "$HOME/poolrouter-env-history"
   cp "$ENV_FILE" "$HOME/poolrouter-env-history/<env>-$(date +%Y%m%d%H%M%S).env"
   ```

   Ini sumber nilai rollback. Tanpa salinan ini, langkah rollback tidak punya nilai "sebelumnya" untuk dikembalikan.
2. Ubah baris `POOLROUTER_BACKEND_IMAGE`, `POOLROUTER_FRONTEND_IMAGE`, dan image lain di file env ke `ghcr.io/faisalcayunda/poolrouter-<nama>@sha256:<digest>`. Digest berasal dari hasil CI untuk commit yang sudah di-review (tag `sha-<12 hex>` atau `vX.Y.Z`).
3. `make compose-config ENV=<env> ENV_FILE=<file>`. Wajib lulus. Ini memeriksa file env yang akan dipakai.
4. `make compose-up ENV=<env> ENV_FILE=<file>`.
5. `make compose-migrate ENV=<env> ENV_FILE=<file>`, jika ada migrasi baru. Migrasi harus dijalankan setelah infra `healthy`.
6. **Rollback:** kembalikan baris digest ke nilai dari salinan langkah 1, lalu jalankan `compose-up` lagi. Digest lama tetap ada di registry karena tag `sha-*` disimpan sesuai retensi ADR-0001 Addendum §7.
   - **Rollback image tidak membatalkan migrasi skema.** Migrasi harus kompatibel mundur (kolom atau tabel baru tidak dibaca image lama), atau punya langkah down sendiri yang direview sebelum deploy. Tanpa salah satunya, jangan rollback image setelah migrasi berjalan; eskalasi ke Huan.

### Migrasi skema

```sh
make compose-migrate ENV=<env> ENV_FILE=<file>
```

Target ini menjalankan `run --rm migrate`, memakai `CONTAINER_ENGINE` yang sama dengan target lain. Hanya service `migrate` yang menerima secret `poolrouter_owner` (ADR-0001 §7.2, dijaga `compose-lint`). Urutan terhadap `bundled-infra`: infra harus `healthy` dulu (`compose-up`), baru migrasi.

### Bootstrap admin dan pengguna (backend-cli)

Image backend memuat `backend-cli` di `/usr/local/bin`, sama dengan `webserver`, `controller`, dan `worker` (ADR-0003 §3.6c, ARC-236). Cara utama di Compose adalah service `backend-cli` (profile `cli`, ARC-237): perintah sekali jalan lewat `compose run --rm`, bukan `up`. Service ini hanya menerima `POOLROUTER_DB_*` dan secret role `pr_admin_api`; di dev ia juga satu-satunya yang menerima `POOLROUTER_SEED_PASSWORD_FILE`. Jalankan setelah `compose-up` dan `compose-migrate`; platform admin pertama hanya sah saat tabel pengguna masih kosong.

Dev: `make compose-seed ENV=dev ENV_FILE=<file>` menjalankan `seed pengguna` lalu `seed dukcapil`.

Perintah lain memakai bentuk yang sama dengan baris `compose-seed` di Makefile. Makefile belum punya target generik untuk perintah selain seed, jadi ambil awalan `compose -p ... --env-file ...` dari keluaran `make -n compose-seed ENV=<env> ENV_FILE=<file>`, lalu ganti perintah akhirnya:

```sh
<awalan compose> --profile cli run --rm backend-cli --help
<awalan compose> --profile cli run --rm backend-cli bootstrap \
  --username <admin> --alasan "<alasan 10-500 karakter, rujukan BA>"
```

Password diminta dari TTY (dua kali, tanpa echo); tanpa TTY, tambahkan `-T` dan kirim satu baris lewat stdin. Password tidak pernah lewat argumen.

`exec` di container backend tetap mungkin karena binarinya ada di image, mis. untuk investigasi di container yang sedang berjalan. Untuk bootstrap dan seed tetap pakai service `backend-cli`: konfigurasinya paling sempit dan satu-satunya yang menerima password seed. Akses `exec` ke container dibatasi untuk petugas infrastruktur.

Bukti image memuat CLI: `make image test-image-cli IMAGE=localhost/poolrouter-backend:dev` (membangun image, lalu menjalankan `backend-cli --help` di dalamnya). Workflow `backend-image` menjalankan pemeriksaan yang sama pada image hasil build Buildah sebelum push. `make test` di `backend/` memeriksa Containerfile secara statis.

### Membersihkan

Dua kasus berbeda. Jangan campur.

**(a) Resource run agent (berlabel `arctiva.issue`).** Dihapus setelah setiap run agent, lalu dicek kosong dengan filter yang sama:

```sh
podman rm -f -v --filter label=arctiva.issue=<ARC-n>
podman volume prune -f --filter label=arctiva.issue=<ARC-n>
podman network prune -f --filter label=arctiva.issue=<ARC-n>

podman ps -a --filter label=arctiva.issue=<ARC-n>
podman volume ls --filter label=arctiva.issue=<ARC-n>
podman network ls --filter label=arctiva.issue=<ARC-n>
```

Ketiga perintah terakhir harus tidak menampilkan apa pun.

**(b) Volume stack dev owner (tidak berlabel `arctiva.issue`).** Ini data milik owner. Hanya dihapus atas keputusan owner, dengan perintah eksplisit per nama volume:

```sh
make compose-down ENV=dev ENV_FILE=deploy/compose/env/dev.env
# Hanya atas keputusan owner, satu per satu:
podman volume rm poolrouter-dev_pg_data
podman volume rm poolrouter-dev_redis_data
podman volume rm poolrouter-dev_rabbitmq_data
```

`compose-down` menghentikan dan menghapus container dan network, tetapi volume tetap ada. Jangan menjalankan `podman system prune`, `podman system reset`, atau `podman machine stop|rm|reset`.

### Compose lint (V1)

```sh
make compose-lint
```

Perintah ini menjalankan self-test (setiap fixture harus gagal dengan labelnya), merender `compose config` untuk dev, staging, dan prod dari template, lalu memeriksa:

- pola terlarang (ADR-0001 Addendum §5): `container_name`, `userns_mode`, opsi volume `U` di posisi mana pun, `privileged`, `cap_add` di service selain `postgres` dan `redis`, `network_mode: host`, `include`, `extends`, `develop`, `watch`, deploy mode Swarm, driver logging selain `json-file`, dan lain-lain
- image wajib berdigest di semua mode. `localhost/` hanya di dev (build lokal)
- secret `poolrouter_owner` hanya di service `migrate`
- `build:` tidak boleh di staging dan prod
- `mock-dukcapil` dan seed sintetis tidak boleh ada di render prod
- Containerfile: tanpa `HEALTHCHECK`, `SHELL`, `ONBUILD`, directive `# syntax`, flag BuildKit (`--link`, `--parents`, `--exclude`), `FROM` tanpa digest, dan user akhir harus `10001:10001`. `.dockerignore` wajib ada di sebelah setiap Containerfile

Exit code 0 berarti lulus. Hasil render disimpan di `deploy/compose/lint/.render/` (diabaikan git).

### Harness DinD (V3)

```sh
make compose-test-dind ARC_ISSUE=ARC-<n> ENV_FILE=<file env di luar repo>
```

Harness menjalankan `docker:27-dind` dengan `--privileged` sebagai container sementara berlabel `arctiva.issue`, lalu menaikkan stack `bundled-infra` lewat klien Compose di container CLI. Smoke test sama dengan V2: `psql`, `redis-cli`, dan `rabbitmq-diagnostics`. Lulus jika semua service healthy dan ketiga smoke test exit 0. Di akhir run, harness menghapus stack di dalam DinD dan memeriksa tiga perintah pembersihan (a) di atas.

Jika DinD tidak bisa berjalan di Podman rootless, catat kegagalannya di komentar issue. Bukti V4 di GitHub Actions menggantikan V3 (fallback ADR-0001 Addendum §8).

## Secret dan file env

- Secret dibaca dari file lewat `secrets: file:` di `POOLROUTER_SECRETS_DIR`. Di container tampil sebagai `/run/secrets/<nama>`.
- Hak akses: direktori `0700` milik user deploy, file `0444`. Pola ini dipilih agar UID 10001 bisa membaca file di Podman rootless maupun Docker rootful tanpa `chown`.
- Provider P3 hanya bisa membaca file env dan secret yang berada di path yang dipasang ke container provider (lewat `POOLROUTER_EXTRA_MOUNTS`, dipasang read-only). Makefile mengatur ini otomatis dari `ENV_FILE` dan `POOLROUTER_SECRETS_DIR`.
- Cara secret masuk ke entrypoint (KD-1, pola tetap untuk image pihak ketiga yang tidak mendukung `*_FILE`):
  - Redis: entrypoint menulis config `requirepass` ke `/tmp/redis.conf` (tmpfs), lalu `redis-server` membacanya. Password tidak ada di argv.
  - RabbitMQ 4: password dibaca dari secret lalu diekspor sebagai `RABBITMQ_DEFAULT_PASS` di shell entrypoint. Nilai ini terlihat di `/proc/<pid>/environ` oleh user yang sudah punya akses root host, tetapi tidak di `podman inspect`, `compose config`, argv, atau image.
  - Postgres: `POSTGRES_PASSWORD_FILE` (dukungan resmi).
- Kedua entrypoint menolak start (exit non-zero, pesan menyebut nama secret) jika isi secret kosong.
- **`*_DEFAULT_PASS` dan `POSTGRES_PASSWORD_FILE` hanya berlaku saat volume pertama kali diinisialisasi.** Mengubah secret tanpa langkah tambahan tidak mengubah password di database atau RabbitMQ yang sudah ada.
- **Rotasi password:** ubah isi file secret, lalu ubah password di sisi service (`ALTER ROLE` atau `rabbitmqctl change_password`) lewat sesi interaktif yang tidak merekam riwayat shell. Password tidak boleh ditaruh di argv (`psql -c "... '<password>'"`), di file migrasi, atau di komentar. Rotasi yang mengikuti pola ini dicatat di issue tanpa nilainya.
- Batasan format: nilai `redis_password` ditulis dalam tanda kutip ganda di config Redis. Gunakan karakter hex atau alfanumerik. Karakter `"` dan `\` tidak didukung.
- Dilarang: menulis nilai secret di repo, issue, komentar, `environment:`, build arg, atau argumen perintah.

## Gerbang deploy

- **Lingkungan PoC BGN (`ENV=prod`):** `make compose-up` hanya dijalankan setelah ada issue yang menyebut host PoC BGN dan approval Huan, serta host itu disetujui. Tanpa keduanya, jangan jalankan perintah apa pun di host PoC. Lane `staging-smoke` tidak lewat gerbang ini: ia berjalan di runner CI dan stack-nya dihapus di akhir job. Deploy produksi F1 tetap G3 (CEO).
- Dev di mesin owner tidak butuh gerbang ini.

## Lingkungan: staging F0, PoC BGN, dan produksi F1

Staging F0 adalah lane CI efemeral `staging-smoke` (ADR-0009 §2.2). Runner `ubuntu-latest` menyediakan Docker Engine dan Compose yang memenuhi prasyarat di atas; image ditarik dari GHCR berdasarkan digest, profile `mock` aktif, dan tidak ada `build:`. Secret dibuat acak per run, stack dihapus di akhir job, tidak ada port publik, dan tidak ada data asli. Nilai yang dicetak lane ini (latensi, throughput) berlabel relatif, bukan angka absolut.

`prod` di repo ini berarti lingkungan PoC BGN, bukan produksi (ADR-0009 §2.3). `ENV=prod` dan `compose.prod.yaml` menyalakan Compose di Docker Engine host PoC dengan image berdigest dan tanpa mock. Nama `prod` dipertahankan supaya Makefile, `compose-lint`, fixture, dan workflow tidak berubah.

Produksi F1 berjalan di Kubernetes (PRD §10, ADR-0001 §8.3). Manifest, lane staging F1, dan ADR infrastruktur produksi masuk F1, sebelum G3.

Bagian ini belum final. Keputusan berikut menunggu owner dan ADR infrastruktur lingkungan PoC BGN:

- Host PoC BGN dan arsitektur CPU (asumsi `linux/amd64`, Q-A2).
- Bundled atau managed PostgreSQL, Redis, dan RabbitMQ (`POOLROUTER_DB_HOST`, `POOLROUTER_REDIS_HOST`, `POOLROUTER_RABBITMQ_HOST` kosong di template prod). Nilai `POOLROUTER_DB_SSLMODE` ditetapkan bersama ADR hosting PostgreSQL (ARC-211). Template memakai `require` sebagai placeholder.
- Reverse proxy atau load balancer di depan port 8080, serta TLS.
- Registry mirror BGN untuk lingkungan PoC BGN (digest disalin dengan `skopeo copy --preserve-digests`).
- Restart setelah reboot host dan rotasi log di Docker rootful (sudah diatur di compose; verifikasi di host PoC).

## Batasan saat ini

- Service backend, frontend, dan mock Dukcapil sudah didefinisikan, tetapi image-nya dibangun di Bagian 2 (SHAA-207 Containerfile). Sampai itu, profile `app` dan `mock` tidak bisa dinyalakan.
- Subcommand `webserver healthcheck` dan konvensi `*_FILE` ada di lingkup backend SHAA-175 (Flik). Sampai ada, healthcheck backend tidak akan `healthy`.
- Service `backend-cli` (ADR-0003 §3.6c, role `ADMIN_API`) ada di profile `cli` dan dijalankan sekali lewat `compose run --rm`, bukan lewat `up`. Di dev, `make compose-seed ENV=dev` menjalankan `seed pengguna` lalu `seed dukcapil` (ARC-237), sesudah `compose-migrate`. Image backend harus dibangun ulang agar memuat `backend-cli`.
- Nama variabel `POOLROUTER_DB_*`, `POOLROUTER_REDIS_*`, dan `POOLROUTER_RABBITMQ_*` mengikuti kontrak env backend v1 (final, Shu). Perubahannya hanya lewat revisi kontrak.
- Job CI V4 (`compose-portability`) diserahkan ke Tessai.

## Rujukan

- ADR-0001 Addendum A (`docs/adr/0001-addendum-a-deploy-engine-agnostic.md` setelah masuk main; sampai saat itu lampiran di ARC-190)
- Kontrak env backend PoolRouter v1 (lampiran SHAA-207; final, Shu)
- SHAA-207 (sub-issue K2), ARC-190 (induk K-38)
- `arctiva-playbook` bagian 6, `shaal-eng-standards` bagian 8
