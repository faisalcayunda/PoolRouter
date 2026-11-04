# Runbook alert F1 PoolRouter

- **Status:** draft, menunggu review Ridley (PR IaC) dan Huan (alert rule dan runbook)
- **Pemilik:** Platform & SRE (lead: Huan)
- **Tanggal:** 2026-11-04
- **Rujukan:** ADR-0006 §7 butir 3–5 dan §11; contract delta F1 v0.9.0 §9 N-4; SHAA-334

## Ringkasan

Dokumen ini menjelaskan setiap alert F1 di `deploy/monitoring/prometheus/rules/poolrouter-f1.rules.yml`: arti, severity, pemilik, dan langkah pertama. Ambang tidak ditulis di aturan. Aturan membaca gauge `poolrouter_ambang_alert{nama}` yang diekspor PoolRouter, sehingga admin cukup mengubah angka di konfigurasi PoolRouter tanpa menyentuh aturan. Perubahan berlaku pada evaluasi berikutnya.

## Severity

| Severity | Arti | Respons |
|---|---|---|
| S1 | Dampak langsung ke layanan atau keamanan | Segera, di luar jam kerja sekalipun, sesuai jalur insiden |
| S2 | Perlu tindakan di jam kerja yang sama | Dalam satu jam kerja |
| S3 | Perlu tindakan dalam sprint | Masuk backlog operasional |

## Ambang (gauge `poolrouter_ambang_alert{nama}`)

Himpunan `nama` tertutup. Backend wajib mengekspor tepat nama berikut. Nilai di kolom "Usulan" adalah usulan awal `[asumsi]`, belum disetujui Huan.

| `nama` | Dipakai alert | Satuan | Usulan |
|---|---|---|---|
| `upstream_error_rasio` | PoolRouterErrorUpstreamTinggi | rasio 0–1 | 0.05 |
| `redis_failclosed_naik` | PoolRouterRedisFailClosed | kenaikan counter per 5 menit | 0 |
| `kuota_peringatan_rasio` | PoolRouterKuotaMendekatiBatas | rasio 0–1 | 0.80 |
| `kuota_kritis_rasio` | PoolRouterKuotaKritis | rasio 0–1 | 0.95 |
| `antrean_umur_tertua_detik` | PoolRouterAntreanTertua | detik | 300 `[asumsi]` |
| `webhook_gagal_rasio` | PoolRouterWebhookGagalTinggi | rasio 0–1 | 0.10 `[asumsi]` |
| `data_balikan_gagal_permanen` | PoolRouterDataBalikanGagalPermanen | kenaikan per jam | 0 |
| `data_balikan_tertunda` | PoolRouterDataBalikanTertunda | jumlah item | 100 `[asumsi]` |
| `arsip_job_gagal` | PoolRouterArsipJobGagal | kenaikan per jam | 0 |
| `arsip_tertunda_hari` | PoolRouterArsipTertunda | hari | 1 |
| `dlq_pesan` | PoolRouterDLQTerisi | jumlah pesan | 0 |
| `admin_mfa_minimum` | PoolRouterAdminMFAKurang | jumlah admin | 2 (D7) |

Jika gauge hilang, `PoolRouterAmbangAlertHilang` menyala, karena tanpa gauge semua alert ambang diam.

## Alert

Setiap alert punya severity, pemilik (Platform & SRE), dan langkah pertama. Label alert hanya memakai himpunan contract (`upstream_pool_id`, `sumber_anggaran`, `komponen`, `hasil`, `error_class`). Tidak ada NIK, `request_id`, atau IP di label maupun anotasi.

### PoolRouterAmbangAlertHilang
- **Severity:** S2. **Pemilik:** Platform & SRE.
- **Arti:** gauge `poolrouter_ambang_alert` tidak ada di Prometheus lebih dari 5 menit. Semua alert ambang tidak aktif.
- **Langkah pertama:** cek apakah PoolRouter (admin API) hidup dan mengekspor `/metrics`. Cek log startup konfigurasi. Jika backend belum mengekspor gauge, eskalasi ke Shu (Engineering).

### PoolRouterErrorUpstreamTinggi
- **Severity:** S2. **Arti:** rasio percobaan upstream dengan `error_class` terisi di atas `upstream_error_rasio` selama 10 menit, per `upstream_pool_id`.
- **Langkah pertama:** lihat `error_class` yang dominan di dashboard. `timeout` atau `koneksi` berarti gangguan jaringan atau provider. `auth_ditolak` atau `kuota_provider_habis` butuh tindakan operator, bukan retry. Jangan menaikkan ambang untuk menyenyapkan alert tanpa catatan di issue.

### PoolRouterRedisFailClosed
- **Severity:** S1. **Arti:** Redis fail-closed terjadi pada komponen tertentu. Jalur request ditolak demi keamanan kuota.
- **Langkah pertama:** cek ketersediaan dan memori Redis, lalu log komponen yang disebut di label `komponen`. Ikuti pemulihan Redis sesuai ADR-0001 §5a.2. Jangan mengosongkan Redis tanpa rebuild dari PostgreSQL.

### PoolRouterKuotaMendekatiBatas
- **Severity:** S3. **Arti:** `poolrouter_anggaran_terpakai / poolrouter_anggaran_efektif` per `upstream_pool_id` dan `sumber_anggaran` di atas `kuota_peringatan_rasio` selama 5 menit.
- **Langkah pertama:** cek laju pemakaian harian dan sisa periode. Pastikan pemakaian sesuai perkiraan. Beri tahu Fitcher bila pemakaian mengubah proyeksi.

### PoolRouterKuotaKritis
- **Severity:** S2. **Arti:** sama dengan di atas, di atas `kuota_kritis_rasio`.
- **Langkah pertama:** hentikan penambahan beban yang tidak perlu dari client. Pastikan intake 429 bekerja. Eskalasi ke Luca bila kuota provider benar-benar habis (keputusan lintas pihak).

### PoolRouterAntreanTertua
- **Severity:** S2. **Arti:** umur request tertua di antrean `upstream_pool_id` di atas `antrean_umur_tertua_detik`.
- **Langkah pertama:** cek worker hidup dan konsumsi broker. Cek apakah controller leader menjalankan job aging. Request yang melewati TTL (K-18) ditutup sendiri.

### PoolRouterWebhookGagalTinggi
- **Severity:** S3. **Arti:** rasio `poolrouter_webhook_total{hasil="gagal"}` di atas `webhook_gagal_rasio` selama 15 menit.
- **Langkah pertama:** cek endpoint client yang gagal dan `error_class` webhook. Retry mengikuti kebijakan US-06B; jangan mengubah kebijakan retry dari runbook.

### PoolRouterDataBalikanGagalPermanen
- **Severity:** S2. **Arti:** ada batch data balikan berstatus `gagal_permanen` dalam satu jam terakhir.
- **Langkah pertama:** cek respons API Data Balikan untuk batch itu (tanpa menyalin isi body yang berisi data pribadi). Admin bisa menyusun ulang item dari batch `gagal_permanen` menjadi batch baru (contract §5.2). Jangan menyusun ulang sebelum penyebab diketahui.

### PoolRouterDataBalikanTertunda
- **Severity:** S3. **Arti:** jumlah item data balikan yang eligible dan menunggu melebihi `data_balikan_tertunda` selama 30 menit (DQ-20).
- **Langkah pertama:** cek jadwal batch dan worker. Cek apakah mesin batch berhenti karena adapter belum tersedia (D-08 masih terbuka).

### PoolRouterArsipJobGagal
- **Severity:** S2. **Arti:** job arsip gagal dalam satu jam terakhir (`poolrouter_arsip_job_total{hasil="gagal"}`).
- **Langkah pertama:** cek koneksi ke object storage arsip dan kredensial worker. Pastikan kredensial tanpa hak Delete tidak dianggap sebagai error sementara. Log tidak boleh memuat NIK.

### PoolRouterArsipTertunda
- **Severity:** S3. **Arti:** arsip log tertinggal lebih dari `arsip_tertunda_hari` hari.
- **Langkah pertama:** cek jadwal job arsip dan hasil job terakhir. Lalu cek kapasitas object storage.

### PoolRouterDLQTerisi
- **Severity:** S2. **Arti:** ada pesan di dead letter queue selama 5 menit.
- **Langkah pertama:** cek `komponen` dan jenis pesan di DLQ menurut prosedur R-29. Jangan memutar ulang pesan sebelum penyebabnya diketahui.

### PoolRouterAdminMFAKurang
- **Severity:** S1. **Arti:** jumlah platform admin aktif ber-MFA kurang dari `admin_mfa_minimum` (D7). Pada `poc` dan `prod`, perubahan peran yang membuat kondisi ini sudah ditolak sistem. Alert ini menangkap kondisi yang terjadi di luar jalur itu.
- **Langkah pertama:** cek audit `audit_akses` untuk perubahan peran atau `reset_mfa` terakhir. Jika ada `reset_mfa_darurat`, ikuti jalur insiden keamanan dan beri tahu Luca. Pemulihan MFA lewat break-glass wajib memakai rujukan insiden atau SPTJM.

## Yang belum tersedia

- **Kredensial kedaluwarsa ≤ 30 hari (F0)** disebut ADR-0006 §7 butir 4, tetapi belum ada metrik di katalog ADR-0001 §5a.9 maupun ADR-0006 §11. Alert ini belum dibuat. Butuh sinyal dari Kinnison atau Shu; usulan dibawa ke Huan.
- **Kanal dan penerima** (OQ-F1-05) belum diputuskan. Alertmanager belum punya receiver. Lihat bagian Opsi di bawah.

## Asumsi yang perlu dikonfirmasi

- `error_class` kosong (atau tidak terisi) untuk percobaan yang sukses. Aturan `PoolRouterErrorUpstreamTinggi` bergantung pada ini.
- `poolrouter_webhook_total` memakai `hasil="gagal"` untuk status gagal, sesuai ADR-0006 §8 (D3).
- Nilai usulan yang bertanda `[asumsi]` belum disetujui.

## Opsi OQ-F1-05 (kanal dan penerima)

Ini opsi, bukan keputusan. Keputusan diambil Platform & SRE bersama BGN lewat Luca.

1. Alertmanager ke email tim operasi BGN, dengan isi hanya label yang diizinkan dan nilai metrik.
2. Alertmanager ke kanal chat tim operasi. Kanal chat baru adalah wewenang Feather, sehingga butuh persetujuan Huan.
3. Dashboard Web UI saja, tanpa notifikasi aktif. Tidak memenuhi ADR-0006 §7 butir 5 kecuali disepakati.

Setiap opsi wajib tetap mematuhi batas isi notifikasi: tanpa NIK, `request_id`, atau IP.

## Uji

- `promtool check rules deploy/monitoring/prometheus/rules/poolrouter-f1.rules.yml`: SUCCESS, 13 aturan.
- `promtool test rules deploy/monitoring/prometheus/tests/poolrouter-f1.test.yml`: SUCCESS, uji untuk kuota kritis, kuota di bawah ambang, ambang hilang, MFA, DLQ, dan Redis fail-closed.
- Keduanya dijalankan di container `docker.io/prom/prometheus@sha256:63805ebb8d2b3920190daf1cb14a60871b16fd38bed42b857a3182bc621f4996` (v3.5.0), bukan instalasi native.
