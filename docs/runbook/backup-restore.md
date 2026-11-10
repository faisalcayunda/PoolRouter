# Runbook: backup dan restore PostgreSQL (pgBackRest)

## Ringkasan

Runbook ini menjelaskan backup, cek rutin, restore PITR, restore setelah kehilangan host, drill, rotasi passphrase, dan peralihan ke jadwal hemat untuk PostgreSQL 17 + pgBackRest di profile `bundled-infra`. Dasar keputusan ada di ADR-0010 (KB private, `products/poolrouter/adr/`). Target R-08: PITR 7 hari, terenkripsi, RTO ≤ 4 jam, RPO ≤ 300 detik [asumsi].

- **Tanggal:** 2026-11-10
- **Status:** draf. Image dan skrip ada di repo ini (`deploy/compose/backup/`). Wiring service `postgres` di Compose masuk lewat PR terpisah, jadi bagian runbook yang bergantung pada wiring itu belum bisa dijalankan dari `main`.
- **Lingkup:** dev dengan data sintetis (staging-smoke dan prod drill masuk gerbang PoC).
- **Keadaan Compose saat ini:** `deploy/compose/compose.yaml` di `main` masih memakai image `postgres:17-alpine` resmi tanpa pgBackRest. Image backup (`localhost/poolrouter-postgres:dev`, `make image-postgres`) dan skrip drill sudah ada, tetapi service `postgres` belum memakainya. `db-bootstrap` dan secret `db_password` sudah ada.

## Prasyarat

- Image `localhost/poolrouter-postgres:dev` dibangun dengan `make image-postgres` (dev, label tiket lewat `ARC_ISSUE=ARC-<n>`). Di staging dan prod image ditarik dari registry dengan digest lewat `POOLROUTER_POSTGRES_IMAGE` di file env (diisi setelah wiring Compose masuk).
- Secret di direktori di luar repo (`POOLROUTER_SECRETS_DIR`, mode `0700`, file `0444`). Untuk backup ditambah `pgbackrest_repo1_cipher_pass`, passphrase repo. Di staging dan prod ditambah `pg_server_crt` dan `pg_server_key` (sertifikat TLS server dari PKI yang disetujui). Isi file tidak pernah ditulis di issue, komentar, atau repo.
- File env di luar repo, dibuat dari `deploy/compose/env/<env>.env.example`. Variabel backup: `PGBACKREST_STANZA`, `PGBACKREST_REPO1_TYPE`, `PGBACKREST_REPO1_RETENTION_FULL` (bawaan 7; ketiganya diteruskan oleh Compose setelah wiring masuk), dan `PR_REPO1_RETENTION_DIFF` (kosong = bawaan pgBackRest; jadwal hemat mengisi 8), `PR_BACKUP_FULL_WEEKDAY` (`daily` atau 1–7, bawaan `daily`).
- Setiap container, volume, dan network yang dibuat untuk drill berlabel `arctiva.issue=ARC-<n>`. Cek ruang kosong lebih dulu dengan `scripts/docker-lane.sh check` (lihat `docs/runbook/docker-lane.md`).

## Layout

| Lokasi | Isi |
|---|---|
| `deploy/compose/backup/Containerfile` | Image PostgreSQL 17 (digest dipin) + pgBackRest 2.60.0 (versi dikunci), user 10001:10001 |
| `deploy/compose/backup/pr-pg-entrypoint.sh` | Baca secret, pasang sertifikat TLS (secret di staging dan prod, self-signed hanya dev), jalankan penjadwal, lalu entrypoint resmi |
| `deploy/compose/backup/test/drive.sh` | Uji penjadwal dengan jam simulasi dan stub pgbackrest, tanpa Docker dan postgres |
| `deploy/compose/backup/pr-backup-scheduler.sh` | Penjadwal: stanza-create dan check saat start, full, diferensial, dan verify |
| `deploy/compose/backup/drill/pr-drill.sh` | Alat drill: penulis sentinel, checksum, restore, serve, dan hitung RPO |
| `deploy/compose/compose.yaml` | Target wiring (PR terpisah, belum ada di `main`): service `postgres` dengan argumen `archive_*` dan `ssl`, volume `pg_backup_repo`, secret `pgbackrest_repo1_cipher_pass` |

Repo backup berada di volume `pg_backup_repo`, terpisah dari volume data `pg_data`. Di staging dan prod repo harus berada di storage terpisah dari host DB (ADR-0010 §4.5); pengaturan itu masuk env file target.

## Jadwal backup (UTC, WIB = UTC+7)

| Jam | Tindakan | Perintah yang dijalankan penjadwal |
|---|---|---|
| 18.00 UTC (01.00 WIB) | Full harian | `pgbackrest --stanza=poolrouter --type=full backup` |
| 00, 06, 12 UTC | Diferensial tiap 6 jam | `pgbackrest --stanza=poolrouter --type=diff backup` |
| 19.00 UTC (02.00 WIB) | Verifikasi harian | `pgbackrest --stanza=poolrouter verify` |
| Terus-menerus | Arsip WAL | Target konfigurasi Compose (wiring): `archive_command = pgbackrest archive-push %p`, `archive_timeout = 60`, `archive-async=y`. Dijalankan Postgres, bukan penjadwal |

Slot dihitung per jam UTC, tidak per menit. Jika loop tertunda, misalnya full berjalan lebih dari satu jam, slot jam berikutnya tetap dijalankan. Setiap jam yang terlewat dicatat sebagai `slot <jam> UTC dilewati`, dan verify yang terlewat dikejar. Start container di tengah jam tidak menjalankan slot jam itu, dan dicatat sebagai `tidak dijalankan ulang saat start`.

Keluaran penjadwal ada di log container `postgres` (`docker logs <kontainer>`, baris berawalan `pr-backup`). Setiap backup yang mulai dicatat, begitu juga hasilnya (sukses atau `GAGAL`), setiap verify, dan setiap slot yang dilewati. Baris `GAGAL` atau `dilewati` harus ditindaklanjuti.

**Pengawasan:** penjadwal berjalan sebagai proses latar belakang tanpa supervisi. Jika proses itu mati, backup berhenti tanpa gejala di log postgres. Deteksinya bergantung pada metrik umur backup terakhir dari exporter. Exporter dan alert rule belum ada di repo ini; keduanya harus terpasang sebelum bagian Triase alert bisa dipicu otomatis.

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
   ENGINE=docker DRILL_LABEL=arctiva.issue=ARC-<n> REPO_VOLUME=<volume repo> SECRETS_DIR=<dir secret> \
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

Skenario (b). Volume data hilang atau rusak, repo masih ada. Repo memuat arsip WAL sampai titik terakhir yang sudah diarsip, jadi RPO dibatasi `archive_timeout` (60 detik) ditambah WAL yang belum diarsip saat kegagalan.

Restore dari repo yang sama mempertahankan system identifier cluster, jadi stanza tetap dan tidak dibuat ulang. Pemulihan sampai akhir arsip membuka timeline baru (timeline naik satu). Arsip WAL timeline baru ditulis ke repo yang sama. Jalur ini untuk layanan (bukan drill): arsip dan penjadwal aktif.

Keputusan pemulihan dicatat di issue dengan T0 (waktu keputusan) sebelum langkah pertama.

1. Hentikan stack. Volume tidak dihapus oleh perintah ini:
   ```sh
   make compose-down ENV=<env> COMPOSE_PROFILES=bundled-infra ENV_FILE=<env file>
   ```
2. Pastikan volume data lama tidak akan dipakai lagi. Volume yang rusak dihapus hanya setelah keputusan tertulis di issue. Buat ulang dengan label Compose agar dikenali sebagai volume service `pg_data`:
   ```sh
   docker volume rm <project>_pg_data
   docker volume create --label com.docker.compose.project=<project> --label com.docker.compose.volume=pg_data <project>_pg_data
   ```
3. Restore ke volume itu, sampai akhir arsip (tanpa `T_target`). Label `DRILL_LABEL` diisi penanda insiden, mis. `incident=<id>`:
   ```sh
   ENGINE=docker DRILL_LABEL=incident=<id> REPO_VOLUME=<project>_pg_backup_repo SECRETS_DIR=<dir secret> \
   POSTGRES_USER=<user> POSTGRES_DB=<db> PGBACKREST_STANZA=poolrouter \
   sh deploy/compose/backup/drill/pr-drill.sh restore <project>_pg_data -
   ```
4. Naikkan stack dengan perintah biasa. Penjadwal dan archive_command aktif lagi. Recovery selesai di akhir arsip dan server dipromosikan sendiri:
   ```sh
   make compose-up ENV=<env> COMPOSE_PROFILES=bundled-infra ENV_FILE=<env file>
   ```
   Tunggu `healthy` di `docker compose ps`.
5. Verifikasi: `pg_is_in_recovery()` harus `f`, `pgbackrest check` harus `check-rc=0` (arsip di timeline baru berjalan), dan kueri aplikasi lolos.
6. Segera setelah `healthy`, buat backup full baru di timeline baru: `pgbackrest --stanza=poolrouter --type=full backup`. Lalu cek `info`.
7. RTO = T1 (healthy dan kueri lolos) dikurangi T0. RPO = commit terakhir yang di-ACK atau diketahui sebelum kegagalan dikurangi titik pulih terakhir. Catat keduanya di issue.

Jika repo tidak berisi WAL sampai titik yang diinginkan, pemulihan hanya bisa sampai titik yang tersedia. Titik itu dicatat di issue, bukan ditebak.

## Drill

Drill dijalankan dengan data sintetis saja. Urutan:

1. `sentinel-init` pada primary, lalu full backup manual.
2. `sentinel-write <kontainer> <detik> <log>` dijalankan di latar belakang untuk skenario (a), lalu pilih baris sekitar tengah log sebagai `T_target` (+0,5 detik) dan jalankan restore PITR (bagian di atas).
3. Skenario (b): jalankan `sentinel-write` dengan jumlah detik yang cukup. Setelah sekitar 60 baris ACK, jalankan `docker kill -s KILL <kontainer>`, lalu hapus kontainer dan volume data dengan `docker rm -f -v` dan `docker volume rm`. Repo tidak disentuh.
4. Pulihkan dan ukur seperti bagian host loss.

Hasil drill lokal terdahulu (2026-11-04, dari lane arsip sebelum repo ini dialihkan; belum diulang pada image di repo ini):

| Skenario | Hasil |
|---|---|
| (a) PITR ke `T_target` | 150 baris ≤ `T_target`, md5 sama dengan primary, 0 baris setelah `T_target` |
| (b) kehilangan host | RPO 59,521 detik (≤ 300); RTO sekitar 5,7 detik (T0 ke T1) untuk database kecil |

Catatan RPO: batas kehilangan dibatasi `archive_timeout = 60` detik, plus WAL yang belum diarsip saat kegagalan. Angka 59,5 detik konsisten dengan batas itu.

## Rotasi passphrase

pgBackRest tidak mengganti passphrase repo yang sudah ada. Rotasi berarti repo baru, dengan jendela PITR yang mulai dari nol. Selama repo lama masih dalam retensi, repo lama adalah satu-satunya titik pulih penuh untuk waktu sebelum rotasi. Repo lama dan repo baru hidup berdampingan di volume `pg_backup_repo`: repo lama di akar volume, repo baru di subdirektori `r2`.

1. Tulis keputusan di issue: tanggal rotasi, alasan, dan tanggal akhir masa tumpang tindih (retensi repo lama).
2. Simpan passphrase lama sebagai secret terpisah, dengan nama `pgbackrest_repo1_cipher_pass_old` (mode 444). Jangan menyalin isinya ke issue atau chat:
   ```sh
   cp <dir secret>/pgbackrest_repo1_cipher_pass <dir secret>/pgbackrest_repo1_cipher_pass_old
   chmod 444 <dir secret>/pgbackrest_repo1_cipher_pass_old
   ```
3. Ganti isi `pgbackrest_repo1_cipher_pass` dengan passphrase baru (nama file tetap, isi baru). Simpan passphrase baru hanya di manajer secret.
4. Di env file, arahkan repo baru ke subdirektori: `PGBACKREST_REPO1_PATH=/var/lib/pgbackrest/r2`.
5. Naikkan stack. Penjadwal melakukan `stanza-create` pada repo baru. Lalu buat full pertama dan `check`:
   ```sh
   make compose-up ENV=<env> COMPOSE_PROFILES=bundled-infra ENV_FILE=<env file>
   ```
6. Selama tumpang tindih, restore dari repo lama dengan passphrase lama. Jalur ini sama dengan restore biasa, dengan variabel `REPO_PATH` dan `CIPHER_SECRET` di-set ke repo lama:
   ```sh
   REPO_PATH=/var/lib/pgbackrest CIPHER_SECRET=pgbackrest_repo1_cipher_pass_old ENGINE=docker DRILL_LABEL=<label> \
   REPO_VOLUME=<project>_pg_backup_repo SECRETS_DIR=<dir secret> POSTGRES_USER=<user> POSTGRES_DB=<db> \
   sh deploy/compose/backup/drill/pr-drill.sh restore <volume-restore> "<T_target>"
   ```
7. Selesai tumpang tindih: hapus berkas repo lama (`archive/` dan `backup/` di akar volume) dan secret `_old`, hanya dengan keputusan tertulis di issue. Jangan hapus berkas repo secara manual sebelum keputusan itu.

## Peralihan ke jadwal hemat

Jadwal hemat (ADR-0010 §5.2): full mingguan, diferensial tiap 6 jam, `repo1-retention-diff=8`. Peralihan menaikkan retensi efektif dari ±8 hari menjadi ±14 hari.

Sebelum peralihan, tulis komentar di issue dengan isi: tanggal berlaku, pemicu (ukuran full terkompresi lebih dari 50 GB, atau batas anggaran repo yang disetujui), dan retensi efektif baru (±14 hari). Komentar menyebut pemilik platform dan Operator (ADR-0010 §5.2).

Setelah komentar itu:

1. Di env file: `PR_BACKUP_FULL_WEEKDAY=<hari ISO, mis. 7 untuk Minggu>` dan `PR_REPO1_RETENTION_DIFF=8`.
2. `make compose-up` (penjadwal dan variabel baru berlaku saat container dibuat ulang).
3. Verifikasi: `info` menampilkan pola full mingguan setelah full pertama pada hari yang ditentukan.

## Triase alert

Alert rule (belum ada di repo ini) akan menaut ke anchor di bawah. Setiap bagian punya cek pertama, penyebab umum, dan tindakan. Anchor ditulis eksplisit agar stabil saat judul berubah.

<a id="alert-arsip-wal-tertunda"></a>
### Arsip WAL gagal atau tertunda (> 5 menit)

- **Cek pertama:** `pg_stat_archiver` (`failed_count` naik?, `last_failed_wal`, `last_archived_time`) dan log kontainer: `docker logs <kontainer> 2>&1 | grep "archive command failed" | tail -3`.
- **Penyebab umum:** repo tidak bisa ditulis (volume penuh, lihat [repo terisi](#alert-repo-terisi)), secret passphrase hilang atau salah, kredensial S3 kedaluwarsa (bila repo s3), atau lock pgBackRest tertinggal.
- **Tindakan:** perbaiki penyebab. Postgres mencoba ulang archive sendiri. Jangan hapus berkas di `pg_wal` secara manual. Jika arsip mati lebih dari satu jam, catat di issue dan periksa RPO yang sedang berjalan.

<a id="alert-full-terlalu-tua"></a>
### Full terakhir yang sukses terlalu tua

- **Cek pertama:** `pgbackrest info` (timestamp full terakhir yang sukses) dan `docker logs <kontainer> 2>&1 | grep "pr-backup"`.
- **Ambang:** 26 jam pada jadwal harian. 8 hari pada jadwal hemat (ADR-0010 §5.1 butir c).
- **Penyebab umum:** penjadwal berhenti (kontainer atau proses penjadwal mati, tanpa supervisi, lihat bagian Jadwal), full gagal (lihat log `GAGAL`), atau repo penuh.
- **Tindakan:** jalankan full manual sekali (`pgbackrest --stanza=poolrouter --type=full backup`) lalu cek `info`. Jika gagal berulang, eskalasi ke pemilik platform.

<a id="alert-verify-gagal"></a>
### Verifikasi repo gagal

- **Cek pertama:** keluaran `pgbackrest verify` (file atau set backup yang disebut) dan `pgbackrest check`.
- **Penyebab umum:** berkas repo rusak atau hilang, WAL yang diperlukan tidak ada, atau disk bermasalah.
- **Tindakan:** jangan hapus berkas repo secara manual. Catat set backup yang terdampak di issue, buat full baru, lalu biarkan set rusak kedaluwarsa lewat retensi. Karena ini menyentuh data, eskalasi ke pemilik platform sebelum tindakan lain.

<a id="alert-repo-terisi"></a>
### Repo terisi > 80%

- **Cek pertama:** ukuran repo (`du -sh` pada volume repo, lewat kontainer dengan mount read-only) dan ruang volume.
- **Penyebab umum:** volume terlalu kecil untuk S2 (ukuran S2, ADR-0010 §3.2), retensi lebih panjang dari perkiraan, atau trafik lebih besar.
- **Tindakan:** jangan hapus berkas repo secara manual. Jalankan `pgbackrest expire --stanza=poolrouter`, yang hanya menghapus di luar retensi. Evaluasi peralihan ke jadwal hemat (bagian Peralihan). Catat keputusan di issue sebelum jadwal berubah. Menambah kapasitas repo adalah keputusan anggaran, diteruskan ke Operator.

<a id="alert-titik-pulih-tertua"></a>
### Titik pulih tertua < 7 hari

- **Cek pertama:** `pgbackrest info` (full tertua dan `wal archive min`).
- **Penyebab umum:** stanza baru dan belum berjalan 7 hari (wajar, catat sebagai ekspektasi, bukan insiden), expire terlalu agresif (retensi di env turun), atau full gagal berulang.
- **Tindakan:** jangan menurunkan retensi untuk menutup alert. Pastikan full berjalan (lihat [full terlalu tua](#alert-full-terlalu-tua)). Jika stanza baru, catat tanggal mulai di issue.

## Pembaruan versi pgBackRest

Versi dikunci di `deploy/compose/backup/Containerfile` (`pgbackrest=2.60.0-1.pgdg12+1`). Repositori PGDG dapat menggeser versi itu, dan build akan rusak. Langkah bump:

1. Di image dasar, cek versi kandidat: `apt-cache policy pgbackrest` (setelah `apt-get update`).
2. Ubah baris `pgbackrest=<versi>` dan, bila base berubah, digest `FROM` di Containerfile. Digest base dipin dari image yang sudah ditarik.
3. `make image-postgres CONTAINER_ENGINE=docker`, lalu `make compose-lint CONTAINER_ENGINE=docker`.
4. Uji minimal: `check`, backup full, `verify`, dan satu skenario drill (a). Hasilnya dicatat di issue.
5. Catat versi lama dan baru, hash commit, dan tanggal di issue.

## Batasan

- **Belum diuji:** repo `s3` (OBS). Konfigurasi S3 belum ada di compose, dan secret kredensial S3 belum didefinisikan. Uji dilakukan terhadap object storage S3-compatible bila sudah tersedia.
- **Belum diuji:** penjadwal berjalan sesuai jam (18.00, 00/06/12, 19.00 UTC). Perintahnya diuji manual dengan urutan yang sama, tetapi jadwal nyata belum diamati.
- **Belum diukur:** umur full tertua setelah lebih dari 8 hari, dan volume repo per hari dengan `archive_timeout = 60` setelah 24 jam. Keduanya butuh waktu berjalan.
- **TLS:** di dev, sertifikat self-signed dibuat `pr-pg-entrypoint.sh` bila belum ada. Di staging dan prod, secret `pg_server_crt` dan `pg_server_key` wajib ada. Tanpa keduanya, start gagal, tidak ada self-signed diam-diam.
- **Kontainer postgres tidak read_only dan tanpa tmpfs:** pengecualian yang dicatat, karena PGDATA, WAL, spool, dan log pgBackRest ditulis ke volume dan path yang ada. Kontrol lain tetap berlaku (cap_drop ALL, no-new-privileges, user 10001).
- **Staging dan prod:** belum diuji. Mengikuti `docs/runbook/deploy.md`.
- Sentinel drill: interval nyata lebih dari 1 detik (`sleep 1` ditambah waktu `exec`). RPO tetap memakai `clock_timestamp()` server, jadi angkanya tidak terpengaruh.
- Passphrase repo dan variabel S3 bisa dibaca dari `/proc/1/environ` oleh proses dengan uid yang sama di dalam container. Pola ini sama dengan RabbitMQ (KD-1, `docs/runbook/deploy.md`). Container tidak diberi akses lain ke secret.

## Pembersihan

Pembersihan akhir run hanya menghapus resource berlabel tiketnya sendiri (`arctiva.issue=ARC-<n>`):

```sh
scripts/docker-lane.sh cleanup ARC-<n>
```

Image `localhost/poolrouter-postgres:dev` tidak dihapus. Dilarang `docker system prune`, `docker volume prune`, dan prune atau `rm` tanpa filter label.

## Rujukan

- ADR-0010 (KB private, `products/poolrouter/adr/`), §4.5, §5.1, §5.2.
- `docs/runbook/deploy.md` (runbook deploy) dan `docs/runbook/docker-lane.md`.
- Dokumentasi pgBackRest: https://pgbackrest.org/user-guide.html
