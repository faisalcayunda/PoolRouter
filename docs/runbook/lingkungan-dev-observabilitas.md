# Runbook: object storage arsip dan monitoring dev (SHAA-334)

Dokumen ini menjelaskan cara menyalakan, memeriksa, dan membersihkan tiga komponen dev F1: object storage arsip, Prometheus, dan Alertmanager. Semua berjalan sebagai container Podman lewat `deploy/compose/compose.dev.yaml`. Tidak ada instalasi native di mesin owner.

## Prasyarat
- File env dev sudah ada (`deploy/compose/env/dev.env`, dari `dev.env.example`). Isi `POOLROUTER_SECRETS_DIR` dengan direktori di luar repo.
- Di direktori secret (dir 0700, file 0444) ada `objstore_root_user`, `objstore_root_password`, dan `objstore_worker_secret`. Nilainya dibuat lokal dan tidak masuk repo. File harus 0444; 0600 membuat container gagal membaca secret.

## Menyalakan
```sh
make compose-config ENV=dev CONTAINER_ENGINE=podman ENV_FILE=deploy/compose/env/dev.env COMPOSE_PROFILES="objstore monitoring egress-mock" EXTRA_MOUNTS=<dir-secret>
make compose-up     ENV=dev CONTAINER_ENGINE=podman ENV_FILE=deploy/compose/env/dev.env COMPOSE_PROFILES="objstore monitoring" EXTRA_MOUNTS=<dir-secret>
```
Urutan: `object-storage` healthy, lalu:
- `objstore-init` membuat bucket `poolrouter-arsip-dev` dengan versioning dan object lock (GOVERNANCE, default retensi `POOLROUTER_ARSIP_RETENSI_HARI`).
- `objstore-iam-init` membuat policy `worker-arsip-nodelete` dan user worker (`OBJSTORE_WORKER_ACCESS_KEY`, secret dari `objstore_worker_secret`). Policy hanya mengizinkan PutObject, GetObject, ListBucket, GetBucketLocation, GetBucketVersioning, GetObjectVersion, dan ListBucketVersions. Tidak ada DeleteObject, DeleteObjectVersion, atau BypassGovernanceRetention.
- `data-balikan-mock` (alamat tetap 10.90.0.10:8080 di network `egress`, internal) sebagai placeholder egress worker.

Kedua init tetap hidup dan healthy setelah selesai (`sleep infinity`), supaya `compose-up` (`--wait`) tidak gagal karena one-shot keluar. Hentikan dengan `compose-down`. `prometheus` dan `alertmanager` healthy. Tidak ada port host yang dibuka oleh layanan-layanan ini.

## Memeriksa
- `make compose-ps ...` menampilkan status. `objstore-init` berstatus `Exited (0)` itu normal.
- Konfigurasi Prometheus dan Alertmanager:
  - `promtool check config /etc/prometheus/prometheus.yml` (di image Prometheus yang di-pin).
  - `amtool check-config /etc/alertmanager/alertmanager.yml` (di image Alertmanager yang di-pin).
- Rules alert: `promtool check rules deploy/compose/monitoring/prometheus/rules/*.rules.yml`.
- Target Prometheus: `backend-webserver`, `backend-controller`, `backend-worker` akan DOWN sampai aplikasi mengekspos `/metrics` di port 9100. Itu diharapkan saat ini.

## Membersihkan
```sh
make compose-down ENV=dev CONTAINER_ENGINE=podman ENV_FILE=deploy/compose/env/dev.env COMPOSE_PROFILES="objstore monitoring" EXTRA_MOUNTS=<dir-secret>
podman volume rm poolrouter-dev_objstore_data poolrouter-dev_prom_data poolrouter-dev_am_data
```
`compose-down` tidak menghapus volume. Hapus volume secara eksplisit hanya jika data dev memang boleh dibuang. Objek di bucket yang masih di dalam masa retensi tidak bisa dihapus sebelum masa retensinya habis.

## Batasan yang diketahui
- Uji kredensial worker tanpa hak Delete belum ada. RustFS 1.0.4 memakai IAM terpisah, dan pembuatan user worker belum diverifikasi. Belum dinyatakan lulus.
- Egress worker ke Data Balikan (ADR-0002 D4) belum ada di compose. Akan memakai placeholder mock dengan IP tetap dan belum dikerjakan di PR ini.
- Kanal dan penerima alert (OQ-F1-05) belum diputuskan. Receiver `dev-null` adalah placeholder.
- Image di-pin dengan tag dan digest index. Digest index diambil dari registry pada 2026-11-04.
