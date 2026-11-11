# Runbook: DLQ job (retensi, metrik, alert, replay)

Runbook ini menjelaskan antrean dead-letter `<antrean>.dlq` milik setiap upstream pool: berapa lama pesan disimpan, metrik dan alert-nya, dan cara mengembalikan pesan ke antrean utama (ARC-228). Dasar: ADR-0001 rev 15 bagian 5, 5a.9, 6 (W4); ADR-0006 bagian 7 (alert awal `poolrouter_dlq_pesan > 0`).

- **Status:** definisi alert di bawah adalah usulan; ambang dan durasi akhir ditetapkan Platform & SRE (ADR-0006 bagian 7). Repo belum punya berkas aturan Prometheus atau Alertmanager, jadi definisinya hidup di dokumen ini.
- **Diuji:** 11 November 2026 terhadap RabbitMQ 4.4.0 (image yang di-pin di `deploy/compose/compose.yaml`) yang dinaikkan lewat Compose (`rabbitmq`, profile `bundled-infra`). Bagian "Hasil uji" di akhir memuat apa yang dijalankan dan apa yang tidak.

## Apa isi DLQ

Handler worker yang mengembalikan `QueueError::Handler` membuat pesan di-reject tanpa requeue, lalu broker memindahkannya lewat exchange `pr.jobs.dlx` ke `<antrean>.dlq` (routing key = nama antrean). Antrean job bernama `pr.jobs.upstream.<upstream_pool_id>`. Handler tidak membedakan galat sementara dan permanen, jadi galat PostgreSQL sementara juga berakhir di DLQ.

Pesan di DLQ **bukan satu-satunya salinan pekerjaan**. Request-nya tetap `dispatched` di PostgreSQL dan reconciler mengembalikannya ke `queued` setelah ambang `dispatched` (default 120 detik, `POOLROUTER_RECONCILER_DISPATCHED_DETIK`); dispatcher lalu menerbitkan ulang. Karena itu DLQ berfungsi sebagai catatan diagnostik, dan membuang pesan tertua dari DLQ tidak menghilangkan request. Body pesan hanya `request_id` (tanpa NIK).

Pesan yang sampai ke DLQ karena galat **transport** juga mungkin: quorum queue RabbitMQ 4 memberi batas pengiriman ulang bawaan 20 (`delivery-limit`); pesan yang melewatinya di-dead-letter ke DLX yang sama.

## Retensi

Argumen deklarasi DLQ ada di `backend/crates/queue-apalis/src/topologi.rs` (`argumen_dlq`), terpisah dari argumen antrean job:

| Argumen | Nilai | Alasan |
|---|---|---|
| `x-queue-type` | `quorum` | Sama dengan antrean job (ADR-0001 bagian 5). |
| `x-message-ttl` | 604800000 ms (7 hari) | Cukup untuk investigasi lewat akhir pekan; lebih lama hanya menyimpan salinan `request_id` yang sudah dipulihkan reconciler. |
| `x-max-length` | 10000 pesan per DLQ | Body kecil; batas ini mencegah satu pool yang rusak memenuhi disk broker. |
| `x-overflow` | `drop-head` | Pesan tertua dibuang saat penuh. `reject-publish` tidak dipakai: DLQ penuh akan menolak pesan terbaru, padahal untuk diagnosis yang terbaru paling berguna. |

Pesan yang kedaluwarsa atau dibuang tidak masuk antrean lain (DLQ tidak punya DLX).

Nilai ini konstanta (`DLQ_TTL_MS`, `DLQ_PANJANG_MAKS`), bukan konfigurasi. Mengubahnya berarti rilis baru **dan** langkah upgrade di bawah, karena broker menolak deklarasi ulang dengan argumen berbeda.

### Upgrade dari DLQ tanpa retensi (wajib sebelum rilis ARC-228)

Antrean yang sudah ada tidak bisa diubah argumennya lewat deklarasi: broker menjawab `PRECONDITION_FAILED - inequivalent arg 'x-max-length'` (406) dan menutup channel. Dampaknya: sesi consumer worker gagal terus (worker keluar sesudah 90 detik, ARC-226) dan `declare` dispatcher gagal, jadi publish berhenti. Proses versi lama yang masih hidup juga mendeklarasikan ulang DLQ tanpa retensi setiap kali menyambung, jadi urutannya penting. Untuk setiap `<antrean>.dlq` yang sudah ada di broker:

1. Hentikan `backend-worker` dan `backend-controller` versi lama.
2. Replay isi DLQ ke antrean utama (bagian Replay) atau catat `request_id`-nya; request-nya sendiri dipulihkan reconciler. Pastikan DLQ kosong.
3. Hapus DLQ lama: `rmqadmin delete queue --name "$Q.dlq"` (fungsi `rmqadmin` ada di bagian Diagnosis).
4. Naikkan image baru; adapter mendeklarasikan ulang DLQ dengan retensi.

Pesan yang di-dead-letter di antara langkah 3 dan 4 dibuang broker karena DLQ belum ada; request-nya tetap dipulihkan reconciler.

## Metrik

`poolrouter_dlq_pesan` (gauge, tanpa label, ADR-0001 bagian 5a.9): jumlah pesan siap di semua DLQ upstream pool. Dihitung job `dlq_metrik` di controller (`backend/crates/controller/src/dlq.rs`) setiap siklus penjadwal (10 detik) dan hanya oleh leader, lewat passive declare pada tiap `<antrean>.dlq` dari semua baris `upstream_pool`. DLQ yang belum ada dihitung 0. Bila satu DLQ gagal dibaca, gauge tidak diubah dan siklus mencatat galat `dlq_metrik`.

Metrik dibaca dari `/metrics` di port admin controller (`POOLROUTER_ADMIN_ADDR`, default `0.0.0.0:9090`).

Batasan yang diketahui:

- Hanya controller leader yang mengisi gauge. Controller standby tidak mengekspor seri ini. Controller yang kehilangan leadership tetap mengekspor nilai terakhirnya sampai proses restart, jadi `max()` pada alert bisa menahan nilai basi; cek `instance` pada alert dan bandingkan dengan leader saat ini (log `controller menjadi leader`).
- Sebelum ARC-228, `/metrics` tidak mengekspor metrik aplikasi apa pun karena recorder Prometheus tidak dipasang global. ARC-228 memasangnya di `poolrouter_core::runtime`, jadi seluruh metrik `metrics::*` (kuota, token, MFA, data balikan) kini ikut tampil di `/metrics` ketiga proses.

## Alert

Usulan aturan Prometheus (ambang dan durasi: Platform & SRE; pengelompokan dan pesan pulih lewat Alertmanager, ADR-0006 bagian 7):

```yaml
groups:
  - name: poolrouter-dlq
    rules:
      - alert: PoolRouterDlqBerisi
        expr: max(poolrouter_dlq_pesan) > 0
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "DLQ PoolRouter berisi {{ $value }} pesan"
          runbook: "docs/runbook/dlq.md"
```

Arti saat menyala: ada handler yang gagal sebelum commit PostgreSQL (database atau konfigurasi), atau pesan melewati batas pengiriman ulang. Mulai dari log worker (`error`) pada waktu yang sama, lalu dari bagian Diagnosis.

## Diagnosis

Semua perintah dijalankan dari host dengan Compose (`ENGINE=podman` atau `docker`). Container `rabbitmq` menjalankan `rabbitmqadmin` v2 bawaan image; kredensialnya dibaca di dalam container dari secret, tidak lewat argv host.

```sh
ENGINE=podman
RMQ=$($ENGINE ps -q --filter label=com.docker.compose.service=rabbitmq)
rmqadmin() {
  $ENGINE exec "$RMQ" sh -c 'exec rabbitmqadmin -u "$RABBITMQ_DEFAULT_USER" -p "$(cat /run/secrets/rabbitmq_password)" "$@"' sh "$@"
}
Q=pr.jobs.upstream.<upstream_pool_id>

# Panjang DLQ: kolom message_count (kolom kedua dari kanan).
rmqadmin --table-style borderless queues show --name "$Q.dlq"
# Intip 5 pesan tanpa menghapusnya (dikembalikan ke antrean). Body = request_id.
rmqadmin --non-interactive get messages -q "$Q.dlq" -c 5 -a ack_requeue_true
```

Header `x-death` pada pesan memuat antrean asal, alasan (`rejected`), dan jumlah. Cari `request_id` di log dan di tabel `request` (status, `attempt_count`) sebelum memutuskan.

## Replay ke antrean utama

Replay aman diulang: pesan duplikat untuk request yang sudah tidak `dispatched` di-ack worker tanpa panggilan upstream dan tanpa baris log (handler langkah 1, ADR-0001 bagian 6 W5). Replay tidak menaikkan kuota untuk request yang sudah selesai. Replay hanya berguna setelah **penyebab** diperbaiki; kalau tidak, pesan kembali ke DLQ. Pesan yang body-nya bukan `request_id` valid tidak bisa diproses; buang dengan `rmqadmin purge queue --name "$Q.dlq"` setelah dicatat.

Mekanisme: shovel dinamis `rabbitmq_shovel` dari `<antrean>.dlq` ke exchange `pr.jobs` dengan routing key = nama antrean. `src-delete-after: queue-length` membuatnya memindahkan hanya pesan yang ada saat dibuat, lalu menghapus dirinya. Shovel permanen **tidak boleh** dipakai: pesan yang gagal lagi akan berputar antara antrean utama dan DLQ. Properti pesan (persistent, header) ikut terbawa, dan shovel meng-ack sumber hanya setelah publisher confirm tujuan (`on-confirm`).

Prasyarat sekali per broker (perubahan plugin di broker adalah tindakan operasional; plugin tidak aktif secara bawaan di image ini):

```sh
$ENGINE exec "$RMQ" rabbitmq-plugins enable rabbitmq_shovel rabbitmq_shovel_management
```

Replay satu pool:

```sh
rmqadmin parameters set --component shovel --name "replay-$Q" --value "{
  \"src-protocol\":\"amqp091\",\"src-uri\":\"amqp://\",\"src-queue\":\"$Q.dlq\",
  \"src-predeclared\":true,\"src-delete-after\":\"queue-length\",
  \"dest-protocol\":\"amqp091\",\"dest-uri\":\"amqp://\",
  \"dest-exchange\":\"pr.jobs\",\"dest-exchange-key\":\"$Q\",\"dest-predeclared\":true,
  \"ack-mode\":\"on-confirm\"}"
```

`src-predeclared` dan `dest-predeclared` wajib: tanpa keduanya shovel mendeklarasikan antrean dengan argumen bawaan dan broker menjawab 406. Beberapa detik kemudian:

```sh
rmqadmin --table-style borderless queues show --name "$Q.dlq"    # message_count 0
rmqadmin --table-style borderless queues show --name "$Q"        # pesan di antrean utama
rmqadmin --table-style borderless parameters list --component shovel   # kosong: shovel sudah menghapus dirinya
```

Bila `parameters list` masih menampilkan shovel dan DLQ tidak turun, lihat `rmqadmin shovels list_all` (kolom `state`) dan log container `rabbitmq`; hapus dengan `rmqadmin parameters clear --component shovel --name "replay-$Q"`.

Tidak ada subperintah `backend-cli` untuk DLQ; `backend-cli` hanya punya `seed` dan bootstrap pengguna.

## Hasil uji (11 November 2026)

Dijalankan terhadap `rabbitmq:4-management-alpine` (digest di `compose.yaml`, RabbitMQ 4.4.0) lewat `compose up` profile `bundled-infra` service `rabbitmq` berlabel `arctiva.issue=ARC-228`:

- Plugin shovel diaktifkan online dengan `rabbitmq-plugins enable` pada container berjalan (user `100:101`).
- Topologi dideklarasikan dengan argumen yang sama dengan adapter (quorum, `x-message-ttl`, `x-max-length`, `x-overflow: drop-head`); broker menerima. Dengan `x-max-length: 2` dan tiga pesan yang di-dead-letter, DLQ berisi dua pesan terbaru (drop-head bekerja pada jalur dead-letter).
- Deklarasi ulang DLQ tanpa argumen retensi ditolak `inequivalent arg 'x-max-length'` (dasar langkah upgrade).
- Replay dengan perintah di atas: dua pesan dari DLQ pindah ke antrean utama, DLQ 0, shovel hilang sendiri, properti `delivery_mode: 2` dan header `x-death` terbawa.

Tidak dijalankan: `x-message-ttl` 7 hari sampai kedaluwarsa (tidak praktis; broker hanya diperiksa menerima argumennya); replay lewat worker PoolRouter sungguhan (stack aplikasi tidak dinaikkan; perilaku duplikat dibuktikan uji handler di crate worker); suite `uji-amqp` (`dlq_dideklarasikan_dengan_retensi_dan_antrean_job_tidak`, `jumlah_dlq_membaca_panjang_dan_antrean_tak_ada_nol`) tertulis di `backend/crates/queue-apalis/tests/kontrak_amqp.rs` dan dijalankan oleh `make -C backend uji-amqp`.
