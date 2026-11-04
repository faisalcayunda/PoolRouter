## Ringkasan

Runbook ini menjelaskan backup, cek rutin, restore PITR, restore setelah kehilangan host, drill, rotasi passphrase, dan peralihan ke jadwal hemat untuk PostgreSQL 17 + pgBackRest di profile `bundled-infra`. Dasar keputusan ada di ADR-0005 (`docs/adr/0005-hosting-postgresql-poc-dan-backup-pitr.md`, rev 4). Target R-08: PITR 7 hari, terenkripsi, RTO ≤ 4 jam, RPO ≤ 300 detik [asumsi].

- **Pemilik:** Sid (Platform & SRE). Reviewer runbook: Huan.
- **Tanggal:** 2026-11-04
- **Status:** draf untuk review Huan. Bukti drill lokal ada di SHAA-381.
- **Lingkup:** dev dengan data sintetis. Staging dan prod belum diuji (lihat bagian Batasan).

## Prasyarat

- Image `localhost/poolrouter-postgres:dev` dibangun dengan `make image-postgres` (dev) atau ditarik dari registry dengan digest (staging dan prod).
- Secret di direktori di luar repo (`POOLROUTER_SECRETS_DIR`, mode `0700`, file `0444`). Untuk backup ditambah `pgbackrest_repo1_cipher_pass`, passphrase repo. Isi file tidak pernah ditulis di issue, komentar, atau repo.
- File env di luar repo, dibuat dari `deploy/compose/env/<env>.env.example`. Variabel backup: `PGBACKREST_STANZA`, `PGBACKREST_REPO1_TYPE`, `PGBACKREST_REPO1_RETENTION_FULL` (bawaan 7), dan `PR_REPO1_RETENTION_DIFF` (kosong = bawaan pgBackRest; jadwal hemat mengisi 8), `PR_BACKUP_FULL_WEEKDAY` (`daily` atau 1–7, bawaan `daily`).
- Di mesin lane (sandbox instabox): Docker lane SHAA-373 (`docs/runbook/docker-lane.md`), dengan cek ruang kosong dan label `shaal.issue=SHAA-381` pada setiap resource.

## Layout

| Lokasi | Isi |
|---|---|
| `deploy/compose/backup/Containerfile` | Image PostgreSQL 17 (digest dipin) + pgBackRest 2.60.0 (versi dikunci), user 10001:10001 |
| `deploy/compose/backup/pr-pg-entrypoint.sh` | Baca secret, buat sertifikat TLS dev bila belum ada, jalankan penjadwal, lalu entrypoint resmi |
| `deploy/compose/backup/pr-backup-scheduler.sh` | Penjadwal: stanza-create dan check saat start, full, diferensial, dan verify |
| `deploy/compose/backup/drill/pr-drill.sh` | Alat drill: penulis sentinel, checksum, restore, serve, dan hitung RPO |
| `deploy/compose/compose.yaml` | Service `postgres` (argumen `archive_*`, `ssl`), volume `pg_backup_repo`, secret cipher |

Repo backup berada di volume `pg_backup_repo`, terpisah dari volume data `pg_data`. Di staging dan prod repo harus berada di storage terpisah dari host DB (ADR-0005 §4.5); pengaturan itu masuk env file target.

## Jadwal backup (UTC, WIB = UTC+7)

| Jam | Tindakan | Perintah yang dijalankan penjadwal |
|---|---|---|
| 18.00 UTC (01.00 WIB) | Full harian (jadwal SHAA-206) | `pgbackrest --stanza=poolrouter --type=full backup` |
| 00, 06, 12 UTC | Diferensial tiap 6 jam | `pgbackrest --stanza=poolrouter --type=diff backup` |
| 19.00 UTC (02.00 WIB) | Verifikasi harian | `pgbackrest --stanza=poolrouter verify` |
| Terus-menerus | Arsip WAL | `archive_command = pgbackrest archive-push %p`, `archive_timeout = 60`, `archive-async=y` |

Penjadwal dan keluarannya ada di log container `postgres` (`docker logs <kontainer>`, baris berawalan `pr-backup`). Baris yang berisi `GAGAL` harus ditindaklanjuti. Penjadwal menulis baris hanya saat ada kegagalan atau saat backup mulai.

## Cek rutin

Semua perintah pgBackRest dijalankan di dalam container dengan passphrase dari secret. Passphrase tidak ditulis di perintah atau di log:

```sh
docker exec <kontainer> sh -c 'export PGBACKREST_REPO1_CIPHER_PASS="$(cat /run/secrets/pgbackrest_repo1_cipher_pass)"; pgbackrest --stanza=poolrouter check; echo "check-rc=$?"'
docker exec <kontainer> sh -c 'export PGBACKREST_REPO1_CIPHER_PASS="$(cat /run/secrets/pgbackrest_repo1_cipher_pass)"; pgbackrest --stanza=poolrouter info; echo "info-rc=$?"'
docker exec <kontainer> psql -X -At -U <POSTGRES_USER> -d <POSTGRES_DB> -c "select archived_count, failed_count, last_archived_time from pg_stat_archiver;"
```

Yang diharapkan: `check-rc=0`, `info` menampilkan `status: ok`, `failed_count` tidak naik, dan selisih `now() - last_archived_time` ≤ 120 detik saat idle (archive_timeout 60 detik).

Pembacaan repo tanpa passphrase harus gagal. Ini bukti bahwa repo terenkripsi:

```sh
docker exec <kontainer> sh -c 'pgbackrest --stanza=poolrouter info'   # harus gagal: requires option: repo1-cipher-pass
```

## Restore PITR ke timestamp

Skenario (a). Restore ke volume baru, tanpa menyentuh volume data yang berjalan.

1. Tentukan `T_target` (format `YYYY-MM-DD HH:MM:SS+00`, waktu server, UTC). Pastikan `T_target` berada dalam jendela pemulihan yang tersedia (`info`: `wal archive min/max`).
2. Restore ke volume baru. Nama volume unik per run:
   ```sh
   ENGINE=docker DRILL_LABEL=shaal.issue=SHAA-381 REPO_VOLUME=<volume repo> SECRETS_DIR=<dir secret> \
   POSTGRES_USER=<user> POSTGRES_DB=<db> PGBACKREST_STANZA=poolrouter \
   sh deploy/compose/backup/drill/pr-drill.sh restore <volume-data-baru> "<T_target>"
   ```
3. Nyalakan server dari volume hasil restore. Server ini tanpa penjadwal dan tanpa arsip, sehingga tidak menulis ke repo:
   ```sh
   sh deploy/compose/backup/drill/pr-drill.sh serve <volume-data-baru> <kontainer-restore>
   ```
4. Verifikasi: `pg_is_in_recovery()` harus `f`, dan log harus memuat `recovery stopping before commit ... time <T_target>` atau `archive recovery complete`.
5. Cocokkan data yang sah: bandingkan `checksum` pada primary dan pada hasil restore untuk `ts <= T_target`. Hasilnya harus identik.
   ```sh
   sh deploy/compose/backup/drill/pr-drill.sh checksum <kontainer-restore> "<T_target>"
   ```
6. Setelah selesai, hapus kontainer restore dan volume restore dengan filter label (lihat bagian Pembersihan). Volume hasil restore dipakai sebagai database sementara, bukan sebagai pengganti volume data yang berjalan tanpa keputusan terpisah.

## Restore setelah kehilangan host

Skenario (b). Volume data hilang, repo masih ada. Yang dipulihkan: sampai arsip WAL terakhir.

1. Catat `T0` (waktu keputusan pulih). RTO diukur dari `T0` sampai server hasil restore menjawab kueri (`T1`).
2. Restore ke volume data kosong baru (tanpa `T_target`):
   ```sh
   sh deploy/compose/backup/drill/pr-drill.sh restore <volume-data-baru> -
   ```
3. Nyalakan server, lalu ukur RPO. Ambil `max_ts` dari checksum. RPO = waktu commit terakhir yang di-ACK sebelum kegagalan dikurangi `max_ts`:
   ```sh
   sh deploy/compose/backup/drill/pr-drill.sh serve <volume-data-baru> <kontainer-restore>
   sh deploy/compose/backup/drill/pr-drill.sh checksum <kontainer-restore>
   sh deploy/compose/backup/drill/pr-drill.sh rpo <log-sentinel> "<max_ts>"
   ```
4. RPO harus ≤ 300 detik dan RTO ≤ 4 jam. Catat keduanya di issue.
5. Setelah database hasil restore dipakai sebagai primary, jalankan `stanza-create` dan backup penuh baru. Stanza yang sama dengan system identifier berbeda tidak boleh dipakai untuk database lain. Jika ini terjadi, pgBackRest menolak dan ini sinyal, bukan kesalahan konfigurasi yang harus dipaksa.

## Drill

Drill dijalankan di lane Docker dengan data sintetis saja. Urutan (SHAA-206 §7):

1. `sentinel-init` pada primary, lalu full backup manual.
2. `sentinel-write <kontainer> <detik> <log>` dijalankan di latar belakang untuk skenario (a), lalu pilih baris sekitar tengah log sebagai `T_target` (+0,5 detik) dan jalankan restore PITR (bagian di atas).
3. Skenario (b): jalankan `sentinel-write` dengan jumlah detik yang cukup. Setelah sekitar 60 baris ACK, jalankan `docker kill -s KILL <kontainer>`, lalu hapus kontainer dan volume data dengan `docker rm -f -v` dan `docker volume rm`. Repo tidak disentuh.
4. Pulihkan dan ukur seperti bagian host loss.

Hasil drill lokal (SHAA-381, 2026-11-04):

| Skenario | Hasil |
|---|---|
| (a) PITR ke `T_target` | 150 baris ≤ `T_target`, md5 sama dengan primary, 0 baris setelah `T_target` |
| (b) kehilangan host | RPO 59,521 detik (≤ 300); RTO sekitar 5,7 detik (T0 ke T1) untuk database kecil |

Catatan RPO: batas kehilangan dibatasi `archive_timeout = 60` detik, plus WAL yang belum diarsip saat kegagalan. Angka 59,5 detik konsisten dengan batas itu.

## Rotasi passphrase

pgBackRest tidak mengganti passphrase repo yang sudah ada. Rotasi berarti repo baru:

1. Buat secret baru `pgbackrest_repo1_cipher_pass` di direktori secret (nama file sama, isi baru) pada repo baru, dengan path repo baru (`PGBACKREST_REPO1_PATH`) atau stanza baru.
2. `stanza-create` dan full backup pada repo baru, lalu `check`.
3. Repo lama dipertahankan sampai retensinya habis (7 hari ditambah satu interval full, ADR-0005 §5.1). Passphrase lama disimpan di tempat yang sama dengan secret lain sampai repo lama dihapus.
4. Catat tanggal rotasi, hash commit konfigurasi, dan rentang retensi repo lama di issue.

## Peralihan ke jadwal hemat

Jadwal hemat (ADR-0005 §5.2): full mingguan, diferensial tiap 6 jam, `repo1-retention-diff=8`. Peralihan menaikkan retensi efektif dari ±8 hari menjadi ±14 hari.

Sebelum peralihan, tulis komentar di issue dengan isi: tanggal berlaku, pemicu (ukuran full terkompresi lebih dari 50 GB, atau anggaran repo), dan retensi efektif baru (±14 hari). Komentar me-mention Shu dan Klaus (ADR §5.2).

Setelah komentar itu:

1. Di env file: `PR_BACKUP_FULL_WEEKDAY=<hari ISO, mis. 7 untuk Minggu>` dan `PR_REPO1_RETENTION_DIFF=8`.
2. `make compose-up` (penjadwal dan variabel baru berlaku saat container dibuat ulang).
3. Verifikasi: `info` menampilkan pola full mingguan setelah full pertama pada hari yang ditentukan.

## Batasan

- **Belum diuji:** repo `s3` (OBS). Konfigurasi S3 belum ada di compose, dan secret kredensial S3 belum didefinisikan. Uji dilakukan terhadap object storage S3-compatible dari SHAA-334 bila sudah ada di `main`.
- **Belum diuji:** penjadwal berjalan sesuai jam (18.00, 00/06/12, 19.00 UTC). Perintahnya diuji manual dengan urutan yang sama, tetapi jadwal nyata belum diamati.
- **Belum diukur:** umur full tertua setelah lebih dari 8 hari (butir non-blocking, SHAA-206), dan volume repo per hari dengan `archive_timeout = 60` setelah 24 jam. Keduanya butuh waktu berjalan.
- **TLS dev:** sertifikat server self-signed dibuat `pr-pg-entrypoint.sh` jika belum ada. Staging dan prod wajib memasang sertifikat sungguhan lewat volume.
- **Staging dan prod:** belum diuji. Mengikuti runbook deploy dan keputusan K-38.
- Passphrase repo dan variabel S3 bisa dibaca dari `/proc/1/environ` oleh proses dengan uid yang sama di dalam container. Pola ini sama dengan RabbitMQ (KD-1). Container tidak diberi akses lain ke secret.

## Pembersihan

Pembersihan akhir run hanya menghapus resource berlabel `shaal.issue=SHAA-381` (playbook §6, `docs/runbook/docker-lane.md`):

```sh
scripts/docker-lane.sh cleanup SHAA-381
```

Image `localhost/poolrouter-postgres:dev` tidak dihapus. Dilarang `docker system prune`, `docker volume prune`, dan prune atau `rm` tanpa filter label.

Sebelum build berat, cek ruang kosong dengan `scripts/docker-lane.sh check` (ambang 3 GB untuk Docker root, 1 GB untuk `/data`). Di bawah ambang, run berhenti dan dilaporkan sebagai blocker.

## Rujukan

- ADR-0005 (`docs/adr/0005-hosting-postgresql-poc-dan-backup-pitr.md`), §4.5, §5.1, §5.2.
- SHAA-206 §5–§7 (desain backup dan skenario drill), contract v0.7.11 §8.4 B-01–B-04.
- Runbook deploy (`docs/runbook/deploy.md`), dan ADR-0001 Addendum A.
- Dokumentasi pgBackRest: https://pgbackrest.org/user-guide.html
