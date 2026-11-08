## Ringkasan

Rencana uji beban dan performa F0 untuk SHAA-205 (NFR-01/02, M-08). Harness sudah siap dan sudah diuji mandiri: skrip k6 dengan profil K-22 (11,6 request/s rata-rata), mock Dukcapil dengan latensi yang bisa diatur (0,5 s, 2 s, 5 s) dan penghitung percobaan per pool, serta skrip run yang membersihkan resource berlabel `shaal.issue=SHAA-205`. Eksekusi belum jalan: gateway (SHAA-198) dan worker/controller (SHAA-199) masih `backlog`, dan hash `main` yang diuji belum diisi Shu. Dampak satu `EXISTS` di gerbang T-01 terhadap throughput worker diukur dengan A/B di PostgreSQL setelah SHAA-199 ada.

Versi ini menambahkan target dari PRD v0.4.7 §4a dan §9 (bagian Target), memperbaiki angka kuota K-22 (kuota efektif 990.000, bukan 9.900), dan mengubah durasi run utama ke minimal 30 menit sesuai M-08. Bukti M-08 hanya sah dari lingkungan PoC BGN; hasil dari Podman lokal berstatus smoke test dan relatif.

- Pemilik: Stallion (Platform & SRE). Reviewer laporan: Huan. Reviewer kode harness: Ridley.
- Tanggal: 2026-11-03. Status: draft, persiapan (menunggu SHAA-198/199 `done` dan hash `main`).
- Acuan (versi lain tidak dipakai):
  - PRD v0.4.7 (lampiran SHAA-162, komentar `01a19fb0-e5b3…`, berkas `PRD-PRJ-2026-01-PoolRouter-v0.4.7.md`): §4a M-02 dan M-08; §9 NFR-01 dan NFR-02 (ditandai "usulan angka, perlu disetujui").
  - ADR-0001 rev 15 (`docs/adr/0001-arsitektur-poolrouter.md`): §1 (K-22 11,6/s rata-rata; K-10 porsi sync "mis. 20%"), §3a.4 (gerbang T-01), §4 (overhead sync p95 ≤ 50 ms, NFR-01), §4.1 (kode HTTP sync), §5a.1 (kuota efektif), §5a.4 (timeout).
  - ADR-0002 rev 5 D3 (bentuk respons Dukcapil "Verify By Element").
  - Data contract v0.7.10 (lampiran SHAA-218, komentar `01a1a000-8767…`; tidak ada di repo): kode HTTP dan `failure_reason` sync, `retry_habis`.
  - `backend/openapi/downstream-api.yaml` (endpoint sync dan async, `X-API-Key`).

## Gating (tidak boleh dilewati)

| Syarat | Status 2026-11-03 |
|---|---|
| SHAA-192 (kontrak OpenAPI) `done` | Ya |
| SHAA-198 (webserver API downstream) `done` | Belum (`backlog`) |
| SHAA-199 (controller, worker, adapter antrean, webhook) `done` | Belum (`backlog`) |
| Hash `main` yang diuji diisi Shu | Belum |
| Kode gateway ada di `main` | Belum. `main` hanya berisi kerangka, kontrak, dan skema |
| Pemisahan 502, 504, dan 429 per kode di harness (`http_502_total`, `http_504_total`, `http_5xx_lain_total`, `http_429_total` dengan tag `alasan`) | Ada di kode (`perf/k6/uji-beban.js`). **Belum diuji** dengan respons 502, 504, atau 429 nyata. Selftest wajib: mock menginjeksi kode itu pada fraksi tertentu, dan jumlah counter k6 harus sama dengan jumlah yang dikirim mock. Mock belum punya injeksi status. Ini prasyarat sebelum ulangan pertama, dan Ridley memeriksanya di review berikutnya |
| Latensi aktual mock tercatat pada run kontrol K0 untuk tiap nilai latensi (`latency_actual_ms` di `mock-stats.json`) | Mekanisme ada dan diuji (smoke 2 s, 60 detik: mean 2.000,448 ms). **Belum** untuk 0,5 s dan 5 s. K0 5 menit per nilai wajib selesai sebelum S-run pada nilai itu, karena overhead dihitung dari mean K0 (lihat "Metode dan kebenaran angka") |

Sampai semua syarat di atas terpenuhi, tidak ada run eksekusi. Harness di `perf/` boleh diuji dengan mock sebagai target, seperti selftest di bagian "Bukti kesiapan".

## Pertanyaan performa dan target

- Apakah gateway dengan profil K-22 memenuhi NFR-01/02 dan M-08 pada latensi upstream 0,5 s, 2 s, dan 5 s, sync maupun async?
- Di beban berapa batas pertama muncul (p95 naik tajam, error 5xx atau 429 muncul, atau kuota tidak lagi terjaga)?
- Konfigurasi konkurensi dan prefetch per pool yang direkomendasikan.

## Target

Angka dari PRD dan ADR dikutip apa adanya. Angka yang belum ada di acuan ditandai **usulan** dan perlu disetujui Huan di rencana ini.

| Metrik | Target | Status | Sumber |
|---|---|---|---|
| NFR-01: overhead sync, p95 | ≤ 50 ms | Dari acuan | PRD §4a M-08 dan §9 NFR-01; ADR-0001 rev 15 §4 |
| NFR-02: enqueue async, p95 | ≤ 100 ms | Dari acuan | PRD §4a M-08 dan §9 NFR-02 |
| Definisi overhead | latensi end-to-end dikurangi latensi stub | Dari acuan | PRD §4a M-08 |
| Overhead sync, p99 | ≤ 100 ms | Usulan | belum ada di PRD atau ADR |
| Enqueue async, p99 | ≤ 200 ms | Usulan | belum ada di PRD atau ADR |
| Error 5xx dan timeout (di luar 429) | ≤ 0,1% per run | Usulan | belum ada di PRD atau ADR |
| Throughput | 11,6/s tercapai, `dropped_iterations` = 0 | Usulan | belum ada di PRD atau ADR |
| M-02: percobaan di atas `kuota_efektif` | 0 | Dari acuan | PRD §4a M-02 dan SHAA-205 AC |
| 429 `kuota_habis` pada run latensi | 0 (run tidak boleh kena kuota) | Usulan | syarat validitas run, lihat Profil beban |

Catatan untuk angka di atas:

- **Target PRD/ADR** (NFR-01, NFR-02, M-02) dan **target usulan** (p99, error, throughput) dilaporkan di tabel terpisah. Pass/fail target usulan tidak boleh dilaporkan sebagai pass/fail NFR (keputusan Huan, 2026-11-03).
- Overhead dihitung dengan latensi aktual mock dari run kontrol, bukan `LAT_MS` nominal. Definisinya ada di "Metode dan kebenaran angka".
- Porsi sync 20% dari 11,6/s = 2,32/s, sama dengan beban sync pada S3. Pada batas ini jitter bucket bisa memunculkan 429 `porsi_sync_habis`. Angka itu dilaporkan terpisah, tidak disembunyikan.

## Profil beban

| Parameter | Nilai | Sumber |
|---|---|---|
| Volume | 1.000.000 request/hari ≈ 41.667/jam ≈ 11,6/s rata-rata | ADR-0001 rev 15 §1 (K-22) |
| Puncak | Tidak ada faktor puncak resmi. ADR rev 15 §1 hanya memuat rata-rata, dan PRD M-08 menulis "puncak TBD". Langkah 1×, 2×, 4× dipakai sebagai stres tambahan dan dilaporkan terpisah dari profil resmi | Keputusan Huan 2026-11-03 |
| Porsi sync | 20% per upstream pool sebagai contoh (`SYNC_SHARE=0.2`). K-10 memberi "mis. 20%", bukan nilai yang ditetapkan per pool | ADR-0001 rev 15 §1 (K-10); PRD §5 K-10 |
| Latensi upstream (stub) | 0,5 s, 2 s, 5 s | Permintaan Huan; latensi Dukcapil asli belum diketahui (T-4) |
| Konkurensi yang dibutuhkan | rate × latensi: ±6 (0,5 s), ±23 (2 s), ±58 (5 s) panggilan paralel | ADR-0001 rev 15 §1 (rumus), dihitung ulang |
| Kuota K-22 | `kuota_harian` = 1.000.000; margin 0,01; `kuota_efektif = floor(1.000.000 × (1 − 0,01))` = **990.000**, dengan penegakan jendela bergulir 24 jam | ADR-0001 rev 15 §5a.1 (default margin 0,01 = 10.000/hari) |

Syarat validitas run latensi: run tidak boleh kena batas kuota. 30 menit pada 11,6/s = 20.880 percobaan, jauh di bawah 990.000. Tetapi jumlah percobaan dalam 24 jam terakhir, termasuk semua run di hari yang sama, harus tetap di bawah 990.000. Karena itu `terpakai` di Redis dicatat sebelum dan sesudah tiap run. Run yang mendapat 429 `kuota_habis` tidak dipakai sebagai angka latensi.

Pembuktian M-02 memakai kuota yang diskalakan (lihat bagian Mock). Itu terpisah dari run latensi.

## Skenario

Setiap skenario dijalankan dengan `perf/run-uji-beban.sh` (satu run per kombinasi). Durasi run utama minimal 30 menit sesuai PRD M-08.

| ID | Mode | Latensi stub | Rate | Durasi | Ulangan |
|---|---|---|---|---|---|
| K0-x | kontrol: k6 langsung ke mock (`BASE_URL=http://perf-mock:8080/p/<pool>`), tanpa gateway. Hanya untuk mengukur latensi aktual mock | 0,5 s / 2 s / 5 s | 11,6/s | 5 menit | 1 per nilai |
| S1-x | sync | 0,5 s / 2 s / 5 s | 11,6/s | 30 menit | 3 |
| S2-x | async (enqueue, sampel 10% sampai terminal) | 0,5 s / 2 s / 5 s | 11,6/s | 30 menit | 3 |
| S3-2s | mix (sync 20%, async 80%) | 2 s | 11,6/s | 30 menit | 3 |
| S4-2s | stres sync dan async, langkah 1×, 2×, 4× (dilaporkan terpisah) | 2 s (dan 5 s bila perlu) | 11,6 / 23,2 / 46,4/s | 5 menit per langkah | 1 |
| W | warm-up, dibuang dari hasil | tiap nilai latensi | 11,6/s | 2 menit | 1 per nilai latensi |

Laju dikirim sebagai bilangan bulat per 100 s (11,6/s = 1.160 per 100 s), jadi tepat tanpa pembulatan. Pada S3 sync 2,32/s dan async 9,28/s juga tepat, sehingga porsi 20% tidak bergeser. Laju yang tidak tepat sampai 2 desimal menggagalkan init k6 (diuji: 11,605/s gagal). Tidak ada penyimpangan dari profil K-22.

Urutan eksekusi (Huan, 2026-11-03: S2 dan S3 pada 2 s dan 5 s lebih dulu): W dan K0 untuk nilai latensi yang sedang dijalankan, lalu S2-2s, S2-5s, S1-2s, S1-5s, S3-2s, S1-0,5s, S2-0,5s, dan S4-2s.

Jumlah run: 21 run utama (S1 9, S2 9, S3 3), 630 menit, ditambah kontrol K0 (3 × 5 menit), warm-up (3 × 2 menit), dan stres S4 (3 × 5 menit). Satu eksekusi penuh kira-kira 11 jam di mesin lokal, tanpa waktu start-stop. Belum ada ulangan yang dikurangi. Bila pengurangan diajukan, penyimpangannya dicatat di laporan. Durasi run utama tidak boleh di bawah 30 menit (M-08).

Terbuka untuk Huan: permintaan 2026-11-03 menyebut S2 dan S3 pada 2 s dan 5 s, tetapi S3 di rencana ini hanya pada 2 s. S3-5s (3 run, 90 menit) belum dimasukkan. Mohon konfirmasi.

Setiap skenario mengukur:
- Sync: `http_req_duration` p50, p95, p99, max; overhead dan throughput efektif (sukses per detik); `dropped_iterations`.
- Async: `async_enqueue_ms` p50, p95, p99; `async_end_to_end_ms` pada sampel 10%; `async_poll_error_total` (poll non-200 atau respons bukan JSON).
- Error: 429 (dipisah dari 5xx, per `alasan` dari body), 502 (`provider_error_permanen`, `retry_habis`), 504 (`timeout_sync`), 5xx lain, dan timeout. Counter per kode ada di harness (`http_502_total`, `http_504_total`, `http_5xx_lain_total`, `http_429_total`). Selftest pemisahan adalah prasyarat (lihat Gating).
- M-02: `m02_over_kuota_total` dari mock.
- Kuota gateway: `terpakai` awal dan akhir run dari Redis.
- Diagnostik: `poolrouter_slot_kedaluwarsa_total` dan metrik konkurensi dari gateway, bila sudah ada (katalog metrik ADR-0001 rev 15 §5a.9).

## Metode dan kebenaran angka

- Baseline minimal tiga ulangan per skenario. Laporkan rata-rata, min, max, dan koefisien variasi (CV) untuk p95. Jika CV p95 di atas 10%, ulangan keempat dan kelima dijalankan dan dilaporkan terpisah. Angka jangan dipilih dari ulangan terbaik.
- Warm-up (skenario W) dijalankan sebelum ulangan pertama tiap nilai latensi dan dibuang.
- Semua angka dari mesin yang sama (Podman VM 2 CPU, 3,7 GiB, lihat keterbatasan).
- Angka lokal diberi label "smoke/relatif". PRD M-08 menyatakan Compose lokal tidak dipakai sebagai bukti metrik. Status M-08 tetap "belum dibuktikan" sampai ada run di lingkungan PoC BGN. Laporan tidak memakai kata "lulus" untuk NFR-01/02 dari angka lokal.
- **Overhead (definisi laporan).** Overhead tidak dihitung dari `LAT_MS` nominal. Langkahnya:
  1. Run kontrol K0 pada latensi yang sama, dengan k6 langsung ke mock. Dari `mock-stats.json`, ambil `latency_actual_ms` (count, mean, min, p50, p95, p99, max; rata-rata latensi aktual mock di server, dari terima request sampai tulis respons). Mock tidak menghitung stdev. Catat `LAT_AKTUAL_MEAN_MS` yang dicetak `run-uji-beban.sh`.
  2. Untuk tiap run S, overhead p95 = p95(`sync_latency_ms`) − `LAT_AKTUAL_MEAN_MS` dari K0 pada latensi itu. Hal yang sama untuk p99 dan untuk enqueue async (`async_enqueue_ms`).
  3. Selisih dicatat di laporan per nilai latensi: `selisih_mock = LAT_AKTUAL_MEAN_MS(K0) − LAT_MS`. Pada smoke 2 s selisihnya +0,448 ms. Selisih per run S juga dicatat, sebagai pemeriksaan drift.
  4. Keterbatasan: p95 dari (e2e − rata-rata) tidak sama dengan p95 dari overhead per request, karena jitter `time.sleep` ada per request. Laporan juga memuat p95 dan max latensi aktual K0, supaya besar jitter terlihat.
- **Lantai harness.** K0 juga memuat jarak k6 ke mock di loopback (proses Python, HTTP). Pada smoke 60 detik 2 s: p95 `sync_latency_ms` 2.027,1 ms dikurangi mean mock 2.000,4 ms = **26,7 ms**. Angka ini sudah mencakup lantai harness dan belum ada gateway. Dari anggaran 50 ms NFR-01, lebih dari separuh (26,7 ms) sudah terpakai oleh lantai itu sebelum ada gateway. Angka K0 wajib dilaporkan berdampingan dengan S-run. Ini satu run 60 detik, bukan ulangan. Angka ini belum dianggap representatif.
- **Validitas run.** Run tidak valid (diulang, atau dilaporkan tidak valid dengan alasan) bila: `dropped_iterations` > 0, kena batas kuota (429 `kuota_habis`, atau `m02_over_kuota_total` > 0), atau exit code dari `run-uji-beban.sh` ≠ 0 selain M-02 gagal. Run tidak valid tidak masuk agregat p95/p99. Keputusan Huan, 2026-11-03.
- **Dropped iterations.** Smoke K0 2 s (60 detik) dan positive control 20 detik sama-sama menunjukkan `dropped_iterations` = 1. Lihat "Yang belum diselidiki".

## Mock Dukcapil dan pembuktian M-02

- Lokasi: `perf/mock-dukcapil/` (Python stdlib, image dipin ke digest). Latensi diatur lewat env `MOCK_LATENCY_MS`; setiap panggilan yang sampai ke mock dihitung sebagai percobaan upstream.
- Body respons: default mengikuti ADR-0002 rev 5 D3. Ada envelope dengan `content` berisi tepat satu elemen, dan nilainya `Sesuai (100)`. Kunci pagination di luar `content` belum ditentukan di ADR-0002, jadi belum dibentuk. Body asli bisa diganti lewat `MOCK_BODY_FILE` bila kontrak sudah dikonfirmasi.
- Penghitung per pool: base URL setiap pool dibuat sebagai `http://perf-mock:8080/p/<pool>`, dan mock menghitung per `<pool>`. Ini asumsi desain. Konfirmasi saat SHAA-198/199 membangun adapter, karena base URL adalah konfigurasi provider (ADR-0002).
- M-02 = jumlah percobaan dengan nomor di atas `kuota_efektif` untuk pool itu. Mock tetap menjawab panggilan seperti itu, jadi pelanggaran terlihat dalam angka, bukan disembunyikan.
- `KUOTA_EFEKTIF` untuk run latensi = sisa kuota di awal run, yaitu `kuota_efektif` (990.000) dikurangi `terpakai` awal dari Redis. Jika `terpakai` awal 0, nilainya 990.000. Alasan: ADR-0001 rev 15 §5a.1 (rev 8) menegakkan kuota pada jendela bergulir 24 jam, dan counter mock mulai dari 0 di tiap run. Tanpa pengurangan itu, percobaan dari run sebelumnya di hari yang sama tidak terhitung dan pelanggaran jendela bisa lolos. Bila ada slot yang kedaluwarsa selama run, percobaan yang sebenarnya masih dalam jendela bisa tercatat sebagai pelanggaran. Arah kesalahannya aman: melaporkan lebih, bukan kurang.
- Run tidak valid (exit 2, bukan lulus): `KUOTA_EFEKTIF` kosong atau `{}`, total percobaan 0, ada pool di mock tanpa `kuota_efektif`, atau pool di `KUOTA_EFEKTIF` tidak pernah dipanggil (nama pool salah ketik).
- Pembuktian M-02: kuota pool di gateway dan mock diset ke nilai yang lebih kecil dari beban (misalnya 1.000 untuk run 10 menit dengan ±6.960 demand). Gateway harus berhenti di ±1.000, dan `m02_over_kuota_total` harus 0.
- Catatan kebijakan: ADR-0001 rev 15 §5a.1 butir 7 mengizinkan `kuota_efektif + slot_kedaluwarsa`. Acceptance criteria SHAA-205 lebih ketat (≤ `kuota_efektif`). Laporan memakai acceptance criteria. Bila ada pelanggaran, pembandingnya `poolrouter_slot_kedaluwarsa_total` dicatat sebagai diagnosis.
- Verifikasi harness: negative control (kuota 50, demand 240) menghasilkan `m02_over_kuota_total = 190` dan exit 1. Positive control (kuota 100.000, latensi 2 s) menghasilkan 0 dan exit 0. Kedua control diulang 2026-11-03 setelah body mock diganti, dengan hasil yang sama, dan diulang lagi setelah perbaikan review (validasi run dan laju k6), dengan hasil yang sama.
- Validasi run (2026-11-03, setelah perbaikan review): `KUOTA_EFEKTIF='{}'` → exit 2 sebelum mock dibangun. Nama pool salah (`pool-salah`) → exit 2. Mock tidak pernah dipanggil (BASE_URL tanpa `/p/<pool>`) → exit 2 dengan total percobaan 0.
- Laju k6 (`k6 inspect`, mix RATE 11,6, SYNC_SHARE 0,2): sync 232 dan async 928 per 1m40s, yaitu 2,32/s dan 9,28/s. RATE 11,605 gagal di init.

## Dampak gerbang T-01 (`EXISTS`) pada throughput worker

Gerbang T-01 menambah satu predikat `EXISTS` ke transaksi compare-and-set `dispatched → in_flight` (ADR-0001 rev 15 §3a.4). Rencana ukurnya:

1. Varian A: CAS tanpa predikat. Varian B: CAS dengan `AND EXISTS (...)` yang membaca enam entitas (client, kredensial, endpoint, downstream pool, upstream pool, provider) dalam transaksi yang sama, bukan dari cache.
2. Alat: `pgbench` dengan skrip kustom, di PostgreSQL 17 container (image dipin), dataset sintetis dengan ±1 juta baris `request` dan tabel konfigurasi berukuran realistis, skema dari `backend/migrations/`.
3. Ukuran: tps dan p95 per transaksi pada konkurensi 8 dan 16; `EXPLAIN (ANALYZE, BUFFERS)` untuk memastikan `EXISTS` memakai index primary key (bukan seq scan).
4. Dampak ke worker = penurunan tps varian B dibanding A, dibandingkan dengan kebutuhan worker pada 11,6/s (±6 tulis per request, ADR-0001 rev 15 §1).
5. Lalu diverifikasi end-to-end di run S2 dengan worker yang sebenarnya.

**Keterbatasan:** predikat CAS yang persis ada di kode worker (SHAA-199). Sampai itu ada, micro-benchmark memakai rekonstruksi dari T-01 dan angkanya belum bisa diklaim sebagai dampak produksi.

## Lingkungan dan keterbatasan

- Semua dependensi runtime (gateway, PostgreSQL, Redis, RabbitMQ, mock, k6) jalan sebagai container Podman di VM lokal yang sama (2 vCPU, 3,7 GiB). Konkurensi antar-container saling mengambil CPU. Angka absolut tidak bisa dipakai sebagai kapasitas produksi, dan tidak bisa dipakai sebagai bukti M-08.
- Staging F0 adalah lane CI efemeral `staging-smoke` (ADR-0009 §2.2): runner `ubuntu-latest` dengan Docker Engine, image berdigest dari GHCR, dan mock Dukcapil aktif. Hasil dari lane ini berlabel relatif, bukan angka absolut, karena spesifikasi runner tidak setara lingkungan PoC BGN. Kewajiban tiga ulangan dan run kontrol K0 di "Metode dan kebenaran angka" tetap berlaku di lane.
- PRD M-08 mensyaratkan lingkungan PoC BGN dengan spesifikasi setara produksi. Lingkungan itu belum ada (PRD Q-05: target deploy PoC/produksi BGN masih terbuka). Ini eskalasi ke Huan (`shaal-playbook` bagian 2): lingkungan lokal tidak representatif untuk M-08. M-08 tetap dinilai di Gerbang PoC (ADR-0009 §2.4 P-4), bukan dari lane staging atau mesin lokal.
- Latensi dan envelope asli Dukcapil belum diketahui (T-4). Default timeout ADR-0001 rev 15 §5a.4 (connect 5 s, total 20 s) ditetapkan ulang setelah PoC.
- Data: NIK sintetis yang dibangkitkan di k6 (berawalan 3201, dibuat acak). Tidak ada data penduduk nyata.
- Kredensial: `POOLROUTER_API_KEY` dan `ENDPOINT_ID` dibaca dari environment, tidak ditulis di repo atau log.

## Keamanan lingkungan (shaal-eng-standards §8)

- Setiap container, network, dan image yang dibangun diberi label `shaal.issue=SHAA-205`. Pembersihan di akhir run memakai tiga perintah berfilter label (`shaal-playbook` bagian 6). Image mock (`localhost/perf-mock-dukcapil:shaa205`) juga dihapus dengan `podman rmi` di akhir run, karena dibangun ulang setiap run. Image k6 dan base image Python dipin dan tetap di cache lokal; keduanya bukan resource run.
- Resource milik issue lain tidak disentuh. Di mesin saat ini ada `shaal-hindsight` (`shaal.service=hindsight`) dan container SHAA-195; keduanya tidak dihentikan.
- Tidak ada instalasi native. `k6` dan `python` dijalankan sebagai container dengan image dipin.

## Cara mengulang (langkah tertulis)

Prasyarat: syarat gating terpenuhi, gateway berjalan di network `<GATEWAY_NET>`, pool uji di gateway memakai `kuota_harian` 1.000.000 dan margin 0,01 (`kuota_efektif` 990.000), endpoint sudah dikonfigurasi dengan base URL `http://perf-mock:8080/p/<pool>`.

```sh
# dari akar repo PoolRouter
export POOLROUTER_API_KEY=...          # dari environment, bukan file
# KUOTA_EFEKTIF: 990000 hanya bila terpakai awal = 0; isi sisa kuota bila tidak (lihat bagian Mock).
RUN_NAME=sync-lat2s-r1 MODE=sync LAT_MS=2000 RATE=11.6 DURATION=30m \
KUOTA_EFEKTIF='{"<pool>":990000}' \
BASE_URL=http://poolrouter-gateway:8080/ ENDPOINT_ID=<uuid-sync> \
GATEWAY_NET=<network-gateway> \
  sh perf/run-uji-beban.sh
# hasil: <workdir>/logs/perf-shaa205/sync-lat2s-r1/{k6-summary.json,mock-stats.json,k6-stdout.log}
# kontrol K0 (wajib sebelum S-run pada latensi yang sama): k6 langsung ke mock, tanpa gateway.
# Catat LAT_AKTUAL_MEAN_MS dari baris keluaran; dipakai sebagai latensi stub untuk overhead.
RUN_NAME=k0-lat2s MODE=sync LAT_MS=2000 RATE=11.6 DURATION=5m \
KUOTA_EFEKTIF='{"pool-uji":990000}' \
BASE_URL=http://perf-mock:8080/p/pool-uji ENDPOINT_ID=<uuid-sync> \
  sh perf/run-uji-beban.sh
```

Untuk pembuktian M-02, pool di gateway dan `KUOTA_EFEKTIF` diset ke nilai terskala (mis. `1000`), lalu `m02_over_kuota_total` harus 0.

Bukti kesiapan (diulang 2026-11-03 dengan body mock baru; `dropped_iterations` = 1 pada positive control 20 detik, dicatat, belum dianalisis):
- Negative control: `RUN_NAME=selftest-kuota50 ... KUOTA_EFEKTIF='{"pool-uji":50}' BASE_URL=http://perf-mock:8080/p/pool-uji` → 240 percobaan, 190 di atas kuota, exit 1.
- Positive control: `RUN_NAME=selftest-kuota-big-lat2s ... LAT_MS=2000 KUOTA_EFEKTIF='{"pool-uji":100000}'` → 240 percobaan, 0 di atas kuota, latensi sync rata-rata 2 s, exit 0.
- Smoke kontrol K0 2 s (2026-11-03, 60 detik, k6 langsung ke mock): `RUN_NAME=kontrol-smoke-lat2s ... LAT_MS=2000 DURATION=60s BASE_URL=http://perf-mock:8080/p/pool-uji`. Hasil: 696 request, `LAT_AKTUAL_MEAN_MS` 2.000,448 (N=696), `m02_over_kuota_total` 0, exit 0. `sync_latency_ms` p95 2.027,137 ms, p99 2.043,767 ms. `dropped_iterations` = 1 (lihat "Yang belum diselidiki"). Ini uji mekanisme pengukuran, bukan angka S1.
- Setelah ketiga run, tidak ada container, network, atau image berlabel SHAA-205 yang tersisa.

## Open questions

1. **Target usulan** (overhead p99, enqueue p99, error ≤ 0,1%, throughput). Ke Huan untuk persetujuan. NFR-01/02 p95 sudah dari PRD. Sampai disetujui, laporan menandai angka itu "usulan", bukan lulus atau gagal.
2. **Faktor puncak.** Tidak ada nilai resmi. Huan mengizinkan 1×/2×/4× dan 20% sebagai stres tambahan yang dilaporkan terpisah. Konfirmasi di laporan.
3. **Lingkungan M-08.** Lingkungan PoC BGN belum ada. Ke Huan: terima hasil lokal sebagai smoke/relatif dengan status M-08 "belum dibuktikan", atau tunda pembuktian M-08 sampai PoC BGN tersedia.
4. **Envelope respons Dukcapil.** Placeholder sekarang mengikuti ADR-0002 D3. Kunci pagination dan elemen selain `NAMA_LGKP` belum ada di acuan. Ke Klaus (contract) dan pelaksana adapter SHAA-199.
5. **Pemetaan pool ke base URL** (`/p/<pool>`) dan konfirmasi bahwa satu panggilan ke mock = satu percobaan `attempt_count`. Ke pelaksana SHAA-198/199 saat kode ada.
6. **Pemisahan 502/504/429 di k6.** Kodenya sudah ada. Selftest dengan mock yang menginjeksi status adalah prasyarat (lihat Gating). Nama field alasan 429 di body (`error`, `code`, atau `reason`) masih asumsi sampai kontrak gateway SHAA-198 ada. Bila tidak cocok, alasannya `tanpa_kode` dan itu harus ditandai di laporan.
7. **S3 pada 5 s.** Lihat catatan di bagian Skenario. Mohon konfirmasi Huan.

## Yang belum diselidiki

- **`dropped_iterations` = 1** pada positive control 20 detik dan pada smoke K0 2 s (60 detik). Dugaan, belum diverifikasi: k6 `constant-arrival-rate` memakai `preAllocatedVUs` = `ceil(rate × 2)` = 24, sedangkan konkurensi yang dibutuhkan pada 2 s sekitar 23,2 (rate × latensi). Pada awal run, VU masih dialokasikan, sehingga satu iterasi bisa terlewat. Langkah: naikkan `preAllocatedVUs` (mis. `ceil(rate × latensi_maks × 1,5)`), ulangi K0 2 s, dan lihat apakah angkanya 0. Bila 0, dugaan itu benar. Bila belum, selidiki lebih lanjut sebelum S-run. Keputusan Huan: satu drop di positive control tetap diselidiki di eksekusi pertama, dan run dengan drop > 0 tidak valid.
