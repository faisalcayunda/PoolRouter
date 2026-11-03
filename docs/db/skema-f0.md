# Skema PostgreSQL F0: pemetaan ke data contract, daftar kolom NULL, dan penyimpangan

Dokumen ini untuk review desain data (Klaus) dan review kode (Ridley). Acuan: data contract v0.7.10, ADR-0001 rev 15 §7, ADR-0003 rev 4 dan Addendum A, ADR-0004 §8. Isi migrasi ada di `backend/migrations/`, bootstrap role di `backend/db/`, uji di `backend/db/test/`.

## 1. Cara menjalankan

- `make -C backend db-test` (juga dipanggil oleh `make test`): memulai container PostgreSQL 17 berlabel `shaal.issue=SHAA-193`, menjalankan `backend/db/bootstrap.sh` dua kali (idempoten), menerapkan semua migrasi dari database kosong sebagai `poolrouter_owner`, lalu menjalankan uji SQL. Container dihapus di akhir.
- Role dibuat oleh `backend/db/bootstrap.sh` (superuser, sekali per cluster), bukan oleh migrasi. Password dibaca dari file secret (`$POOLROUTER_SECRETS_DIR/<nama role>`) lewat psql, tidak lewat argumen. Migrasi hanya memuat skema, fungsi, dan grant, dan berjalan sebagai `poolrouter_owner`.
- `.sqlx/`: belum ada kueri `query!` di issue ini, jadi belum ada isi untuk di-commit (ADR-0004 §8). Isi `.sqlx/` mulai saat kode akses data ditulis.
- Subcommand `migrate` (`compose.yaml` service `migrate`) belum menjalankan migrasi ini. Itu pekerjaan kode runtime, bukan skema. Kebutuhan wiring compose (role bootstrap sebelum `migrate`) adalah hand-off ke Platform & SRE.

## 2. Daftar kolom NULL untuk body yang tidak bisa di-parse (AC SHAA-193)

Aturan (contract §3.2.2 `validasi_gagal`): kolom Wajib §3.1 yang nilainya hanya bisa diperoleh dari body boleh NULL **hanya** pada baris `status = 'rejected'`, `reject_reason = 'validasi_gagal'`, `reject_http_status = 400`.

**Daftar final yang boleh NULL (4 kolom):**

| Kolom | Alasan boleh NULL |
|---|---|
| `downstream_pool_id` | Pool "dipilih manual atau otomatis" (§3.1). Pilihan manual berasal dari input client |
| `prioritas` | Diturunkan dari downstream pool |
| `prioritas_efektif` | Diturunkan dari `prioritas` (K-11) |
| `ttl_expires_at` | `received_at` + TTL downstream pool, jadi ikut pool |

**Tetap wajib (NOT NULL), karena berasal dari autentikasi, path, atau konfigurasi:** `request_id`, `client_id`, `credential_id`, `downstream_endpoint_id`, `provider_id`, `provider_config_version`, `endpoint_config_version`, `mode`, `status`, `metadata` (= `{}`), `received_at`, `completed_at` (baris `rejected` terminal), `attempt_count` (= 0). `input_payload` NULL pada setiap baris `rejected` (P-01).

**Constraint (kata demi kata dari migrasi):**

```sql
CONSTRAINT chk_pool_kolom_serempak CHECK (
  (downstream_pool_id IS NULL) = (prioritas IS NULL) AND (downstream_pool_id IS NULL) = (prioritas_efektif IS NULL)),
CONSTRAINT chk_null_body_tak_terbaca CHECK (
  (downstream_pool_id IS NOT NULL AND (mode = 'sync' OR ttl_expires_at IS NOT NULL))
  OR (status = 'rejected' AND reject_reason = 'validasi_gagal' AND reject_http_status = 400
      AND metadata = '{}'::jsonb AND input_payload IS NULL AND ttl_expires_at IS NULL)
)
```

Diuji di `backend/db/test/20_constraints.sql`: NULL diterima pada `validasi_gagal`/400; ditolak pada `validasi_gagal`/422, `kuota_habis`, baris `queued`, `metadata` tidak kosong, dan `input_payload` terisi; `credential_id`, `received_at`, dan versi konfigurasi tetap ditolak NULL.

**Pertanyaan terbuka untuk Klaus (asumsi yang saya ambil):** tidak ada sumber (contract, ADR-0001 §4.1) yang menyatakan bagaimana client memilih pool "manual". Saya mengasumsikan pilihan itu bisa berada di body, maka empat kolom di atas dibolehkan NULL. Bila pilihan pool ternyata lewat path atau header (selalu diketahui), daftar menyusut menjadi kosong: hapus cabang kedua `chk_null_body_tak_terbaca` dan `chk_pool_kolom_serempak`, tanpa migrasi data. `mode` saya anggap diturunkan dari path (`POST` sync/async terpisah, ADR-0001 §4), jadi wajib. Mohon dikonfirmasi lalu dimasukkan ke contract v0.7.x.

## 3. Pemetaan contract §3 ke tabel

| Contract | Tabel | Catatan |
|---|---|---|
| §3.1 `request` | `request` | Semua 40 kolom ada (diuji `10_mapping.sql` dari daftar kolom contract) |
| §3.3 `transaction_log` | `transaction_log` | Indeks `(upstream_pool_id, requested_at)` ditambah untuk DQ-34 (d) dan DQ-05 (komentar v0.7.8) |
| §3.3a KP-01..KP-07 | `upstream_pool` (kolom konfigurasi), `upstream_attempt_daily` | Unik `(upstream_pool_id, periode_mulai)`; `EXCLUDE` agar periode satu pool tidak tumpang tindih; `tanggal` hanya label. Perubahan `reset_kuota_*` disimpan di `reset_kuota_*_berikut` + `reset_kuota_berlaku_mulai` (KP-02) |
| §3.4 P-02, P-03, H-01, H-02 | `fn_purge_p02`, `fn_purge_p03`, `fn_ditahan_data_balikan` | Predikat di fungsi database |
| §3.5 `request_status_history` | `request_status_history` | Append-only; `terjadi_at DEFAULT now()` (satu nilai per transaksi, EN-08); trigger tertunda memastikan T-04 |
| §3.6 `webhook_log` | `webhook_log` | Tanpa UPDATE |
| §3.7 `client_webhook`, allowlist | `client_webhook`, `webhook_domain_allowlist` | 8 kolom W-02 untuk `pr_webhook` |
| §3.8 V-01..V-05 | `provider_config_versi`, `endpoint_config_versi` | Immutable (trigger), `config_schema_version = 1` |
| §3.2.6 EN-01..EN-09 | `client.aktif`, `client_kredensial.status/dicabut_at/dicabut_oleh`, pool/endpoint/provider `aktif`, `fn_nonaktifkan_entitas`, `fn_aktifkan_entitas`, `fn_pratinjau_nonaktif`, `fn_entitas_nonaktif_pertama` | `dicabut` terminal di trigger (semua jalur tulis) dan di fungsi |
| §4 | `endpoint_metadata_field` | |
| §5 | `data_balikan_batch`, `data_balikan_item` | Tabel dibuat sekarang agar H-01/H-02 bisa ditulis; job-nya F1 |
| §8.2 `audit_akses` | `audit_akses`, `fn_auditor_*`, `fn_ekspor_log_auditor`, `fn_lihat_audit` | Checksum ekspor dihitung di fungsi (E-01, E-02) |
| ADR-0001 §7 audit konfigurasi | `audit_konfigurasi` | `jumlah_cancelled`, `jumlah_failed` (EN-08) |
| ADR-0003 §8 | `admin_user`, `admin_user_peran`, `admin_session` | `bahasa_ui TEXT NULL CHECK IN ('id','en')` tanpa default (Addendum A §7.2); `admin_user_peran` menegakkan A-02 lewat CHECK |

## 4. Penyimpangan dari contract (untuk disetujui Klaus)

| Contract | Skema | Alasan |
|---|---|---|
| `TIMESTAMP` (UTC) | `timestamptz` | Instan yang tegas; tidak bergantung zona sesi |
| `JSON` | `jsonb` | Operator `-`/`?` dipakai purge metadata dan uji |
| `VARCHAR` tanpa panjang | `text` + `CHECK` domain | Domain tertutup ditegakkan database |
| `request.reject_http_status` dan `reject_reason` | `CHECK` pasangan sah | Memindahkan DQ-32 butir pasangan ke constraint |
| `NIK CHAR(16)` | `char(16)` + `CHECK ~ '^[0-9]{16}$'` | |
| Kolom fisik status K-20 (§3.2.6 menyerahkan penamaan) | entitas: `aktif boolean`; kredensial: `status`, `dicabut_at`, `dicabut_oleh` | Nama sama dengan contract EN-02/EN-03 |
| ID UUIDv7 | Tanpa default di database; `gen_random_uuid()` (v4) hanya untuk `audit_id` baris yang ditulis fungsi database | PostgreSQL 17 tidak punya `uuidv7()`; aplikasi membuat ID |
| `audit_akses`: ADR-0001 §7.2 memberi `pr_admin_api` INSERT/SELECT | INSERT saja; baca lewat `fn_lihat_audit` | Setiap baca menulis baris `lihat_audit` (E-06); hanya auditor |
| T-04 "dalam transaksi yang sama" | Constraint trigger tertunda: status `request` harus punya baris history dengan `status_ke` sama saat commit | Menjadikan T-04 bisa diuji |
| Tabel di luar §3 yang dibutuhkan | `client_endpoint`, `downstream_pool_rute`, `provider_secret`, `konfigurasi_platform` (batas ekspor E-03 = 10.000), `admin_*`, `audit_konfigurasi` | Contract menyebutnya tanpa mendefinisikan kolom (ADR-0001 §7, EN-04, E-03) |
| Parameter dispatch pool (rate K-22, konkurensi, jeda acak K-05) | `upstream_pool.parameter_dispatch jsonb` | Contract §3 tidak mendefinisikannya; dikunci saat kode dispatcher ditulis |
| Login berhasil/gagal admin "dicatat di audit" (ADR-0003 §3.6) | Belum ada tabel | `audit_akses.aksi` tidak memuat nilai login; perlu keputusan Klaus (tambah nilai, atau tabel terpisah) |
| Batas sinkronisasi `endpoint.log_mode` "hanya lebih ketat" | Belum ditegakkan di database | Contract menyerahkannya ke admin API (422) |
| M-R1 (endpoint Dukcapil wajib punya `status_penerima_manfaat`) | Belum ditegakkan di database | Aturan aktivasi endpoint, di admin API |

## 5. Role dan hak (ADR-0001 §7.2, contract §8.1, §8.1a, §8.1b)

Hak diberikan per kolom pada `request`, `transaction_log`, `client`, `client_kredensial`, dan `client_webhook`. Fungsi `SECURITY DEFINER` mengunci `search_path`, dan `EXECUTE` dicabut dari `PUBLIC`.

- `pr_api` memiliki akses SELECT `nik` pada `transaction_log` untuk jalur sync; ini adalah kesesuaian kontrak §8.1, bukan akses admin/webhook.
- `pr_controller`: tanpa SELECT `nik`, `input_payload`, `output_payload`, `metadata`, `request_payload`, `response_payload`; menutup log `tidak_diketahui` hanya lewat `fn_tutup_log_tidak_diketahui`; purge lewat fungsi.
- `pr_worker`: membaca `input_payload`; kolom status entitas saja (tanpa hash kredensial); `EXECUTE fn_purge_p02` (jalur gerbang, §8.1b).
- `pr_webhook`: 8 kolom `client_webhook`, allowlist, `output_payload` hanya lewat view `v_request_webhook` (baris terminal async); tidak punya hak atas `transaction_log`, `provider_secret`, `client`.
- `pr_admin_api`: tidak ada SELECT `nik`/payload; secret write-only; `aktif` entitas dan `status` kredensial hanya lewat fungsi (audit + pembatalan massal); view tersamar `v_request_tersamar`, `v_transaction_log_tersamar`; client owner lewat `fn_request_milik_client`.
- Uji: `30_roles.sql` (U-23, U-55, A-01, DQ-24 (a)(d), DQ-37 (f)), `40_functions.sql` (U-54, EN-08/DQ-37 (e), P-02/P-03/H-01, ekspor E-01..E-06, DQ-35).


Migrasi membuat grant kolom berdasarkan snapshot `information_schema` saat migrasi berjalan. Kolom yang ditambahkan migrasi berikutnya tidak otomatis mendapat grant; setiap grant baru harus eksplisit dan diuji.
