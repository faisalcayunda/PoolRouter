# Skema PostgreSQL F1: delta aditif atas F0, pemetaan ke data contract, cek kualitas, dan rollback

Dokumen ini untuk review desain data (Klaus) dan review SQL (Ridley). Acuan: data contract v0.9.5-final (Bagian 13, F1-2 … F1-10), ADR-0006 rev 3 final (`docs/adr/0006-delta-arsitektur-f1.md`; untuk D6 rev 4 §8 R-24a berlaku saat terbit). Isi migrasi ada di `backend/migrations/20261104*`, skrip rollback di `backend/db/rollback/`, uji di `backend/db/test/5*.sql` dan `60_f1_roles.sql`. Skema F0 ada di `docs/db/skema-f0.md`; migrasi F0 tidak diubah.

## 1. Cara menjalankan dan menguji

- `make -C backend db-test CONTAINER_LABEL=arctiva.issue=ARC-333`: container PostgreSQL 17 sementara (Podman), bootstrap role dua kali, semua migrasi dari database kosong, lalu uji. Container dihapus di akhir.
- Setelah uji skema: database kedua, F0 saja → F1 → rollback F1 → bandingkan `pg_dump -s` dengan keadaan F0 (harus identik) → F1 naik lagi. Rollback yang ditolak karena ada data F1 juga diuji.
- Urutan migrasi: `20261104000001` (D1–D3 dan pengaturan platform), `…02` (audit_akses, D4, D6), `…03` (D7), `…04` (hak). Semuanya sesudah `20261103000004` (migrasi F0 terakhir di `main`).
- **Role baru lebih dulu.** `backend/db/bootstrap.sh` kini membuat `pr_job_balikan`, `pr_job_arsip`, `pr_restore` dan membaca tiga file secret baru (`$POOLROUTER_SECRETS_DIR/<role>`). Jalankan ulang bootstrap (idempoten) sebelum migrasi F1; tanpa itu migrasi `…04` gagal karena role belum ada. Wiring secret di compose dan runbook adalah hand-off ke Platform & SRE.

## 2. Pemetaan contract → objek

| Contract | Objek fisik | Ditegakkan oleh |
|---|---|---|
| F1-2 D1 kolom `request.aging_ambang_detik`, `prioritas_diubah_at`; `request_status_history.prioritas_dari/ke` | kolom nullable baru | CHECK rentang (0 atau 60–86.400), `chk_aging_hanya_async` (sync/rejected/F0 NULL), `chk_prioritas_diubah_at`, `chk_aging_prioritas_terisi`, `chk_aging_baris` (AG-04) |
| AG-01 `1 ≤ efektif ≤ prioritas` | `chk_prioritas_efektif_le_prioritas` | CHECK (ditambah `NOT VALID` lalu `VALIDATE`) |
| AG-02/AG-03 snapshot dan satu langkah | `tg_request_f1_guard` + constraint trigger tertunda `request_aging_history_wajib` | `prioritas` dan snapshot tidak berubah; `prioritas_efektif` hanya turun 1 pada request `queued` dengan `prioritas_diubah_at = now()`; baris aging wajib ada di transaksi yang sama |
| F1-2.1 konfigurasi | `downstream_pool.aging_ambang_detik` (default 600), `konfigurasi_platform.prioritas_maks` (default 5) | CHECK; `tg_konfigurasi_f1`, `tg_downstream_pool_prioritas` |
| F1-3 D2 `downstream_pool_rute.alokasi_jenis/alokasi_nilai/sifat/boleh_pinjam` | 4 kolom | CHECK (i), (ii), (iii) kata demi kata dari contract |
| AL-01 `Σ a_i ≤ Q` | `fn_alokasi_kelebihan(upstream_pool_id)` + constraint trigger tertunda pada rute, `upstream_pool` (kuota/margin), dan pengaktifan ulang `downstream_pool` | menolak dengan jumlah kelebihan di pesan (U-64) |
| `transaction_log.sumber_anggaran`, `downstream_pool_id`; `request.reject_lingkup` | kolom baru | CHECK bentuk (`sisa` atau UUID); trigger `tg_log_f1_insert` (DQ-44 b); `tg_log_update` diperluas (R-01: tidak diubah sesudah INSERT); `chk_reject_lingkup` (DQ-44 a) |
| F1-4 D3 `request.webhook_not_before`; kebijakan retry | kolom + 4 kunci `konfigurasi_platform` | `chk_webhook_not_before` (DQ-46 c); rentang di `tg_konfigurasi_f1` |
| F1-5 D4 `data_balikan_batch.kirim_berikutnya_at`, `config_ref_version`; `data_balikan_item.aktif` | kolom baru | `chk_kirim_berikutnya` (DQ-47 b); unique partial index `data_balikan_item_aktif_uq` (DB-01); trigger `data_balikan_batch_gagal_permanen` dan `data_balikan_item_sesuai_batch` (DQ-47 a) |
| `config_schema_version = 2` | `provider_config_versi` menerima 1 dan 2 | validasi isi blok `data_balikan` (V-05, 422) tetap di admin API |
| F1-6.2 `arsip_manifest` | tabel | CHECK per contract (AR-02, DQ-48 c); unik `(tabel_sumber, provider_id, tanggal_wib)` dengan `NULLS NOT DISTINCT` |
| F1-6.3 `arsip_restore_permintaan` | tabel | CHECK; `tg_restore_rentang` (AR-05); constraint trigger tertunda `arsip_restore_audit_wajib` (DQ-49 a); `tg_restore_identitas` (identitas beku) |
| AR-06 skema `arsip_pulih` | `arsip_pulih.transaction_log` = kolom dan CHECK `transaction_log` + `restore_id` | tanpa hak untuk siapa pun selain owner dan `pr_restore` (INSERT/DELETE); `fn_arsip_pulih_kedaluwarsa` (AR-07) |
| F1-6.4 `audit_akses` | 5 kolom, 7 nilai `aksi` baru | `chk_aksi`, `chk_alasan_wajib` diperluas, `chk_audit_*` (DQ-49 d, DQ-50 d) |
| F1-7 D7 `pengguna_mfa`, `kode_pemulihan_mfa`, `kode_daftar_mfa`, `admin_session.mfa_terverifikasi_at` | tabel dan kolom | unique partial index satu kode aktif (MF-10), trigger anti-turun `langkah_terakhir` (MF-03), kode pemulihan sekali pakai (MF-02), kode pendaftaran beku |
| MF-07 pencabutan sesi | `DELETE FROM admin_session` pemegang peran auditor/platform admin di migrasi `…03` | diuji di `run.sh` |
| Hak role (F1-5.4, F1-6.5, MF-09, WR-07, ringkasan F1-9) | `20261104000004_f1_grants.sql` | uji `60_f1_roles.sql` |

Nama fisik mengikuti F0 (`timestamptz`, `text` + CHECK, `jsonb`); `sesi` di contract adalah `admin_session` di F0.

## 3. Asumsi dan keputusan desain yang perlu disetujui Klaus

1. **`aging_ambang_detik` ada di `downstream_pool`.** F1-2.1 menyebutnya konfigurasi level downstream pool, tetapi daftar kolom di F1-9 tidak memuatnya. Saya menambah kolom `downstream_pool.aging_ambang_detik NOT NULL DEFAULT 600`. Disetujui Klaus sebagai koreksi contract (patch v0.9.6).
2. **"Rute aktif" tidak punya kolom di F0.** Definisi (sesuai kode): rute aktif = rute yang `downstream_pool.aktif = true`. Upstream pool hanya kunci pengelompokan AL-01, jadi tidak diuji terpisah. Tidak menambah kolom `aktif` pada `downstream_pool_rute` karena contract tidak menambahnya.
3. **`boleh_pinjam` di database default `true`** supaya rute F0 (tanpa alokasi) memenuhi CHECK (ii). Admin API menulis `false` secara eksplisit untuk rute beralokasi (contract 3.1: "default false pada rute beralokasi").
4. **AL-02/AL-04 (domain dan izin `sumber_anggaran`) tidak dibuat trigger.** Trigger di jalur tulis log bisa menggagalkan INSERT log setelah token diberikan dan melanggar M-03 (log ditulis sebelum panggilan). Tetap cek pipeline DQ-42.
5. **Kolom "wajib sejak aktivasi F1" yang berlaku untuk baris baru saja** (`aging_ambang_detik`/`prioritas_diubah_at` pada async, `reject_lingkup` pada `kuota_habis`, `downstream_pool_id` log, `kunci_hmac_versi` audit) tidak bisa menjadi NOT NULL atau CHECK tanpa menggagalkan UPDATE baris F0. Tetap cek pipeline (bagian 4).
6. **Fungsi penghapusan online AR-04 belum ada.** `pr_job_arsip` tidak punya DELETE. Fungsi `SECURITY DEFINER` itu bagian implementasi D6 (stage 2) dan butuh keputusan: trigger F0 `tg_append_only` mengizinkan DELETE hanya bila `current_user = 'pr_job_arsip'`, sedangkan fungsi `SECURITY DEFINER` berjalan sebagai `poolrouter_owner`. Usul: fungsi menyetel variabel sesi lokal transaksi yang dibaca `tg_append_only`. Dibahas di issue D6.
7. **Fungsi auditor untuk `arsip_pulih` (AR-06) belum ada.** Skema dan hak sudah siap (hanya owner yang bisa SELECT); fungsi `SECURITY DEFINER` yang menulis `audit_akses` dengan `sumber = arsip` dibuat di D6.
8. **`pr_controller` kehilangan `UPDATE (archived_at)`** (baris "Job arsip" §8.1 diganti `pr_job_arsip`). Belum ada kode F0 yang memakainya.
9. **`pr_job_balikan`** mendapat SELECT `request` (tanpa `output_payload`), `transaction_log` (tanpa `request_payload`/`response_payload`, termasuk `nik`), `provider`, `provider_config_versi`, `endpoint_metadata_field`, `downstream_endpoint`, dan INSERT/UPDATE `data_balikan_*`. Tidak ada akses `provider_secret`: contract menyebut resolve secret "sudah ada di worker"; saya baca itu sebagai koneksi `pr_worker`. Mohon konfirmasi.
10. **Role CLI pengguna F0** (MF-09) tidak punya role database sendiri di F0 (ADR-0003 §3.6c: satu jalur kode dengan admin API), jadi hak MFA hanya untuk `pr_admin_api`. DQ-50 (e) diuji dengan tafsir itu.
11. **`provider_config_versi.config_schema_version`** dilonggarkan dari `= 1` menjadi `IN (1, 2)`; satu uji F0 yang mengharapkan penolakan nilai 2 dipindah ke nilai 3.
12. **`data_balikan_batch.config_ref_version NOT NULL` tanpa default** (contract: wajib). Satu fixture uji F0 (`40_functions.sql`) mengisi kolom ini. F0 belum pernah menyusun batch, jadi tidak ada baris yang perlu diisi ulang; bila ada, migrasi mengisinya dari `provider.config_versi_aktif`.

## 4. Cek kualitas: constraint/trigger vs cek pipeline

Cek pipeline ditulis sebagai SQL native terjadwal (tiap jam, jendela 2 jam, bersama DQ-01…DQ-10) dengan hasil ke tabel hasil cek, dan dibuat di issue lanjutan. Pemilik usulan: penulis dan operasi cek = Jowy (Data Engineer); tindak lanjut temuan `error` = Klaus. Mohon dikonfirmasi.

| Cek | Diwujudkan di database | Sisa sebagai cek pipeline |
|---|---|---|
| DQ-40 aging | (a) rentang dan rantai satu langkah per baris, (c) kolom aging pada sync/rejected dan `prioritas_diubah_at ≥ received_at` | (a) jumlah baris aging = `prioritas − prioritas_efektif` dan rantai antar baris; (b) selisih waktu antar langkah; (d) macet (warn) dan async tanpa snapshot |
| DQ-41 | (a) AL-01 lewat trigger tertunda; (b) CHECK | (c) rute aktif tanpa alokasi dengan `boleh_pinjam = false` pada pool beralokasi |
| DQ-42 | bentuk `sumber_anggaran` | (a)–(d) domain, izin pinjam, NULL pada pool beralokasi |
| DQ-43 | tidak ada | seluruh cek (beban per anggaran, bergulir dan kalender) |
| DQ-44 | (a) `reject_lingkup` hanya pada `kuota_habis` dan domain, (b) trigger | `reject_lingkup` NULL pada `kuota_habis` sejak aktivasi; `anggaran` pada pool tanpa alokasi |
| DQ-45 | tidak ada (info) | seluruh cek |
| DQ-46 | (c) `webhook_not_before` hanya pada `menunggu` | (a), (b), (d), (e) |
| DQ-47 | (a), (b) lewat unique index, trigger, CHECK | (b) `attempt_count > kirim_batas_percobaan`, (c), (d), (e) |
| DQ-48 | (c) kelengkapan manifest | (a), (b), (d) gerbang R-24 per `POOLROUTER_ENV` (database tidak tahu lingkungannya) |
| DQ-49 | (a) audit padanan, (b) rentang, (c) pseudonim kedaluwarsa, (d) `sumber = arsip` tanpa restore id | permintaan `siap` tanpa baris `restore_arsip`; baris `arsip_pulih` lewat `kedaluwarsa_at` + 1 hari |
| DQ-50 | (d) sebagian lewat CHECK `audit_akses`; (e) hak di katalog (diuji); (g)(1)(2) sebagian | (a), (b), (c), (f), (g)(3)–(5), (h), (j), (k), dan (d) bagian yang butuh join |
| DQ-15 diperluas | tidak ada | pekerjaan job arsip |

## 5. Dampak ke data dan rencana rollback

- **Dampak.** Aditif: 4 kolom `request`, 2 `request_status_history`, 2 `transaction_log`, 4 `downstream_pool_rute`, 1 `downstream_pool`, 2+1 `data_balikan_*`, 5 `audit_akses`, 1 `admin_session`; 5 tabel dan 1 skema baru; 8 baris `konfigurasi_platform`. Tidak ada baris F0 yang di-UPDATE. `ADD COLUMN` dengan default konstan atau NULL tidak menulis ulang tabel. `ADD CONSTRAINT … CHECK` pada `request` dan `audit_akses` memindai tabel sekali; yang pada `request` memakai `NOT VALID` + `VALIDATE`. Satu-satunya penghapusan data: sesi pemegang peran auditor/platform admin dicabut (MF-07, AC US-24 butir 8); sesi berumur 8 jam dan tidak dipulihkan oleh rollback (pengguna login ulang).
- **Rollback.** Jalankan `backend/db/rollback/20261104000004…`, `…03`, `…02`, `…01` (`.down.sql`) berurutan sebagai `poolrouter_owner`. Setiap skrip menolak berjalan bila ada data F1 (tabel baru berisi, kolom baru terisi, nilai `aksi` baru, dll.) kecuali `SET poolrouter.izinkan_hapus_data_f1 = 'ya'`, yang hanya dipakai setelah persetujuan Klaus dan owner. Hasil rollback tanpa data F1 diuji identik dengan skema F0.
- **Produksi.** Tidak ada migrasi dijalankan di produksi dari issue ini. Penerapan ke lingkungan nyata menunggu persetujuan Klaus dan owner serta rilis F1.
- **Temuan di F0 (diperbaiki di F1).** `ALTER DEFAULT PRIVILEGES … IN SCHEMA public REVOKE EXECUTE … FROM PUBLIC` di migrasi F0 `…04` tidak berefek: default tingkat skema tidak bisa mengurangi hak `EXECUTE` PUBLIC bawaan, sehingga fungsi yang ditambah migrasi berikutnya tetap bisa dieksekusi PUBLIC (uji "no function is executable by PUBLIC" gagal pada fungsi F1). Migrasi `…04` F1 memasang default tingkat global untuk `poolrouter_owner` dan mencabut PUBLIC dari semua fungsi yang ada. Uji F0 "default privileges" di `30_roles.sql` berjalan sebagai superuser sehingga tidak menangkap ini; usul memperkuatnya di issue lanjutan.

## 6. Perubahan pada berkas uji F0

Semua uji F0 tetap hijau; empat penyesuaian karena kontraknya memang berubah: `20_constraints.sql` (nilai `config_schema_version` yang ditolak jadi 3), `30_roles.sql` (tabel MFA masuk daftar DELETE yang sah untuk `pr_admin_api`, dipin ulang di `60_f1_roles.sql`), `40_functions.sql` (fixture batch mengisi `config_ref_version`), `run.sh` (tiga role baru, rollback). Tidak ada NIK nyata: semua NIK uji sintetis berprefiks `00`.

## 6. Hak atas `arsip_manifest` (perbaikan review desain)

- `pr_admin_api` hanya mendapat SELECT kolom `manifest_id`, `tabel_sumber`, `provider_id`, `tanggal_wib`, `kunci_hmac_versi`, `status`: cukup untuk menentukan `kunci_hmac_versi` objek dalam rentang restore (`nik_hmac_per_versi`). Tanpa `dek_terbungkus` (Rahasia), `objek_kunci`, `sha256`, dan kolom lain. Hanya `pr_restore` membaca manifest penuh (AR-02c).
- `request.webhook_not_before`: SELECT dan UPDATE hanya `pr_webhook` (WR-07); `pr_api`, `pr_worker`, `pr_controller` tidak punya hak apa pun atas kolom itu (WR-08).
- Dicatat untuk PR: temuan `ALTER DEFAULT PRIVILEGES` F0 (ARC-193) berarti grant `EXECUTE` fungsi F0 tidak pernah efektif sebelum F1; perlu diketahui Ridley dan Platform & SRE.
