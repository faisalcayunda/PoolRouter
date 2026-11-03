## Ringkasan

Rencana uji beban dan performa F0 untuk SHAA-205 (NFR-01/02, M-08). Harness sudah siap dan sudah diuji mandiri: skrip k6 dengan profil K-22 (11,6 request/s rata-rata), mock Dukcapil dengan latensi yang bisa diatur (0,5 s, 2 s, 5 s) dan penghitung percobaan per pool, serta skrip run yang membersihkan resource berlabel `shaal.issue=SHAA-205`. Eksekusi belum jalan: gateway (SHAA-198) dan worker/controller (SHAA-199) masih `backlog`, dan hash `main` yang diuji belum diisi Shu. Dampak satu `EXISTS` di gerbang T-01 terhadap throughput worker diukur dengan A/B di PostgreSQL setelah SHAA-199 ada.

- Pemilik: Stallion (Platform & SRE). Reviewer laporan: Huan. Reviewer kode harness: Ridley.
- Tanggal: 2026-11-03. Status: draft, persiapan.
- Acuan: ADR-0001 rev 15 §1 (K-22, K-10), §3a.4 (gerbang dispatch), §5a.1 (kuota efektif), §5a.4 (timeout); ADR-0002 rev 5 (adapter Dukcapil); `backend/openapi/downstream-api.yaml` (endpoint sync dan async, `X-API-Key`).

## Gating (tidak boleh dilewati)

| Syarat | Status 2026-11-03 |
|---|---|
| SHAA-192 (kontrak OpenAPI) `done` | Ya |
| SHAA-198 (webserver API downstream) `done` | Belum (`backlog`) |
| SHAA-199 (controller, worker, adapter antrean, webhook) `done` | Belum (`backlog`) |
| Hash `main` yang diuji diisi Shu | Belum |
| Kode gateway ada di `main` | Belum. `main` (898c13e) hanya berisi kerangka, kontrak, dan skema |

Sampai semua syarat di atas terpenuhi, tidak ada run eksekusi. Harness di `perf/` boleh diuji dengan mock sebagai target, seperti selftest di bagian "Bukti kesiapan".

## Pertanyaan performa dan target

- Apakah gateway dengan profil K-22 memenuhi NFR-01/02 dan M-08 pada latensi upstream 0,5 s, 2 s, dan 5 s, sync maupun async?
- Di beban berapa batas pertama muncul (p95 naik tajam, error 5xx/429 muncul, atau kuota tidak lagi terjaga)?
- Konfigurasi konkurensi dan prefetch per pool yang direkomendasikan.

**Target angka belum tertulis di acuan yang saya baca.** NFR-01, NFR-02, dan M-08 ada di PRD v0.4.6 (SHAA-162), yang tidak ada di repo. Sebelum laporan final, angka target (p95, p99, throughput, error budget) harus dikonfirmasi. Pertanyaan ini diteruskan ke Huan (lihat komentar di issue).

## Profil beban

| Parameter | Nilai | Sumber |
|---|---|---|
| Volume | 1.000.000 request/hari ≈ 41.667/jam ≈ 11,6/s rata-rata | ADR-0001 rev 15 §1 (K-22) |
| Porsi sync | Maks 20% per upstream pool sebagai contoh (`SYNC_SHARE=0.2`) | ADR-0001 §1 (K-10) |
| Puncak | Belum ada angka. Default: langkah stres 1×, 2×, 4× dari 11,6/s | Asumsi, perlu konfirmasi (lihat open question) |
| Latensi upstream (mock) | 0,5 s, 2 s, 5 s | Permintaan Huan; latensi Dukcapil asli belum diketahui (T-4) |
| Konkurensi yang dibutuhkan | rate × latensi: ±6 (0,5 s), ±23 (2 s), ±58 (5 s) panggilan paralel | ADR-0001 §1 (rumus), dihitung ulang |
| Kuota | `kuota_efektif = floor(kuota_harian × (1 − margin_kuota))`; K-22 dengan margin 0,01 → 9.900/hari | ADR-0001 §5a.1 |

Catatan: pada 11,6/s selama 10 menit terkirim ±6.960 request. Itu di bawah 9.900, jadi kuota harian asli tidak tercapai dalam satu run. Karena itu pembuktian M-02 memakai kuota yang diskalakan (lihat bagian M-02).

## Skenario

Setiap skenario dijalankan dengan `perf/run-uji-beban.sh` (satu run per kombinasi).

| ID | Mode | Latensi mock | Rate | Durasi | Ulangan |
|---|---|---|---|---|---|
| S1-x | sync | 0,5 s / 2 s / 5 s | 11,6/s | 10 menit | 3 |
| S2-x | async (enqueue, sampel 10% sampai terminal) | 0,5 s / 2 s / 5 s | 11,6/s | 10 menit | 3 |
| S3-2s | mix (sync 20%, async 80%) | 2 s | 11,6/s | 10 menit | 3 |
| S4-x | stres sync dan async, langkah 1×, 2×, 4× | 2 s (dan 5 s bila perlu) | 11,6 / 23,2 / 46,4/s | 5 menit per langkah | 1 |
| W | warm-up, dibuang dari hasil | 2 s | 11,6/s | 2 menit | 1 |

Jumlah run: 21 run utama (S1 9, S2 9, S3 3), ditambah run stres dan warm-up. Estimasi ±5–6 jam di mesin lokal. Setiap run ditutup dengan exit code dan `m02_over_kuota_total`.

Setiap skenario mengukur:
- Sync: `http_req_duration` p50, p95, p99, max, dan throughput efektif.
- Async: waktu enqueue (p50, p95, p99) dan waktu end-to-end (enqueue sampai `done`/`failed`/`expired`/`cancelled`) pada sampel 10%.
- Error: 5xx, 429 (dipisah dari 5xx), dan timeout.
- M-02: `m02_over_kuota_total` dari mock.
- Diagnostik: `poolrouter_slot_kedaluwarsa_total` dan metrik konkurensi dari gateway, bila sudah ada (katalog metrik ADR-0001 §5a.9).

## Metode dan kebenaran angka

- Baseline minimal tiga ulangan per skenario. Laporkan rata-rata, min, max, dan koefisien variasi (CV) untuk p95. Jika CV p95 di atas 10%, ulangan keempat dan kelima dijalankan dan dilaporkan terpisah. Angka jangan dipilih dari ulangan terbaik.
- Warm-up (skenario W) dijalankan sebelum ulangan pertama dan dibuang.
- Run tidak boleh melintasi batas reset hari kuota (default 00:00 Asia/Jakarta, ADR-0001 §5a.3 butir 1b). Jadi run dimulai minimal 30 menit sebelum batas dan tidak melewatinya, atau reset counter dan `kuota_efektif` dicatat di laporan.
- Semua angka dari mesin yang sama (Podman VM 2 CPU, 3,7 GiB, lihat keterbatasan).

## Mock Dukcapil dan pembuktian M-02

- Lokasi: `perf/mock-dukcapil/` (Python stdlib, image dipin ke digest). Latensi diatur lewat env `MOCK_LATENCY_MS`; setiap panggilan yang sampai ke mock dihitung sebagai percobaan upstream.
- Penghitung per pool: base URL setiap pool dibuat sebagai `http://perf-mock:8080/p/<pool>`, dan mock menghitung per `<pool>`. Ini asumsi desain; konfirmasi saat SHAA-198/199 membangun adapter, karena base URL adalah konfigurasi provider (ADR-0002, bagian Base URL).
- M-02 = jumlah percobaan dengan nomor di atas `kuota_efektif` untuk pool itu. Mock tetap menjawab panggilan seperti itu, jadi pelanggaran terlihat dalam angka, bukan disembunyikan.
- Pembuktian: kuota pool diset ke nilai yang lebih kecil dari beban (misalnya 1.000 untuk run 10 menit dengan ±6.960 demand). Gateway harus berhenti di ±1.000. `m02_over_kuota_total` harus 0.
- Catatan kebijakan: ADR-0001 §5a.1 butir 7 mengizinkan `kuota_efektif + slot_kedaluwarsa`. Acceptance criteria SHAA-205 lebih ketat (≤ `kuota_efektif`). Laporan memakai acceptance criteria. Bila ada pelanggaran, pembandingnya `poolrouter_slot_kedaluwarsa_total` dicatat sebagai diagnosis.
- Verifikasi harness: negative control (kuota 50, demand 240) menghasilkan `m02_over_kuota_total = 190` dan exit 1. Positive control (kuota 100.000, latensi 2 s) menghasilkan 0 dan exit 0. Keduanya sudah dijalankan.

## Dampak gerbang T-01 (`EXISTS`) pada throughput worker

Gerbang T-01 menambah satu predikat `EXISTS` ke transaksi compare-and-set `dispatched → in_flight` (ADR-0001 §3a.4). Rencana ukurnya:

1. Varian A: CAS tanpa predikat. Varian B: CAS dengan `AND EXISTS (...)` yang membaca enam entitas (client, kredensial, endpoint, downstream pool, upstream pool, provider) dalam transaksi yang sama, bukan dari cache.
2. Alat: `pgbench` dengan skrip kustom, di PostgreSQL 17 container (image dipin), dataset sintetis dengan ±1 juta baris `request` dan tabel konfigurasi berukuran realistis, skema dari `backend/migrations/`.
3. Ukuran: tps dan p95 per transaksi pada konkurensi 8 dan 16; `EXPLAIN (ANALYZE, BUFFERS)` untuk memastikan `EXISTS` memakai index primary key (bukan seq scan).
4. Dampak ke worker = penurunan tps varian B dibanding A, dibandingkan dengan kebutuhan worker pada 11,6/s (±6 tulis per request, ADR-0001 §1).
5. Lalu diverifikasi end-to-end di run S2 dengan worker yang sebenarnya.

**Keterbatasan:** predikat CAS yang persis ada di kode worker (SHAA-199). Sampai itu ada, micro-benchmark memakai rekonstruksi dari T-01 dan angkanya belum bisa diklaim sebagai dampak produksi.

## Lingkungan dan keterbatasan

- Semua dependensi runtime (gateway, PostgreSQL, Redis, RabbitMQ, mock, k6) jalan sebagai container Podman di VM lokal yang sama (2 vCPU, 3,7 GiB). Angka absolut tidak bisa dipakai sebagai kapasitas produksi, dan konkurensi antar-container saling mengambil CPU. Laporan hanya klaim relatif (A/B, batas di mesin ini).
- Bila lingkungan ini dinilai tidak representatif untuk NFR-01/02, itu eskalasi ke Huan (`shaal-playbook` bagian 2 "Eskalasi").
- Data: NIK sintetis yang dibangkitkan di k6 (berawalan 3201, dibuat acak). Tidak ada data penduduk nyata. Body respons mock adalah placeholder dan harus diganti envelope sintetis dari kontrak Dukcapil (ADR-0002) sebelum eksekusi.
- Kredensial: `POOLROUTER_API_KEY` dan `ENDPOINT_ID` dibaca dari environment, tidak ditulis di repo atau log.

## Keamanan lingkungan (shaal-eng-standards §8)

- Setiap container, network, dan image yang dibangun diberi label `shaal.issue=SHAA-205`. Pembersihan di akhir run memakai tiga perintah berfilter label (`shaal-playbook` bagian 6).
- Resource milik issue lain tidak disentuh. Di mesin saat ini ada `shaal-hindsight` (`shaal.service=hindsight`) dan container SHAA-195; keduanya tidak dihentikan.
- Tidak ada instalasi native. `k6` dan `python` dijalankan sebagai container dengan image dipin.

## Cara mengulang (langkah tertulis)

Prasyarat: syarat gating terpenuhi, gateway berjalan di network `<GATEWAY_NET>`, endpoint sudah dikonfigurasi dengan base URL `http://perf-mock:8080/p/<pool>`.

```sh
# dari akar repo PoolRouter
export POOLROUTER_API_KEY=...          # dari environment, bukan file
RUN_NAME=sync-lat2s-r1 MODE=sync LAT_MS=2000 RATE=11.6 DURATION=10m \
KUOTA_EFEKTIF='{"<pool>":9900}' \
BASE_URL=http://poolrouter-gateway:8080/ ENDPOINT_ID=<uuid-sync> \
GATEWAY_NET=<network-gateway> \
  sh perf/run-uji-beban.sh
# hasil: <workdir>/logs/perf-shaa205/sync-lat2s-r1/{k6-summary.json,mock-stats.json,k6-stdout.log}
```

Untuk pembuktian M-02, `KUOTA_EFEKTIF` diset ke nilai terskala (mis. `1000`), lalu `m02_over_kuota_total` harus 0.

Bukti kesiapan (sudah dijalankan 2026-11-03):
- Negative control: `RUN_NAME=selftest-kuota50 ... KUOTA_EFEKTIF='{"pool-uji":50}' BASE_URL=http://perf-mock:8080/p/pool-uji` → 240 percobaan, 190 di atas kuota, exit 1.
- Positive control: `RUN_NAME=selftest-kuota-big-lat2s ... LAT_MS=2000 KUOTA_EFEKTIF='{"pool-uji":100000}'` → 240 percobaan, 0 di atas kuota, latensi sync rata-rata 2 s, exit 0.
- Setelah kedua run, tidak ada container atau network berlabel SHAA-205 yang tersisa.

## Open questions

1. **Target NFR-01/02 dan M-08** (p95, p99, throughput, error budget). Ke Huan. Sampai dijawab, laporan hanya memuat angka mentah dan tidak menyatakan lulus atau gagal.
2. **Faktor puncak** di atas 11,6/s, dan porsi sync per pool. Default sementara 1×/2×/4× dan 20%. Ke Huan.
3. **Format envelope respons Dukcapil** untuk mock (ADR-0002, kontrak "Verify By Element"). Ke Huan; bisa diselesaikan dengan Klaus bila perlu.
4. **Pemetaan pool ke base URL** (`/p/<pool>`) dan konfirmasi bahwa satu panggilan ke mock = satu percobaan `attempt_count`. Ke pelaksana SHAA-198/199 saat kode ada.
