# Skema PostgreSQL F0: pemetaan ke data contract, daftar kolom NULL, dan penyimpangan

Dokumen ini untuk review desain data (Klaus) dan review kode (Ridley). Acuan: data contract v0.7.10, ADR-0001 rev 15 §7, ADR-0003 rev 4 dan Addendum A, ADR-0004 §8. Isi migrasi ada di `backend/migrations/`, bootstrap role di `backend/db/`, uji di `backend/db/test/`.

## 1. Cara menjalankan

- `make -C backend db-test` (juga dipanggil oleh `make test`): memulai container PostgreSQL 17 berlabel `arctiva.issue=ARC-193`, menjalankan `backend/db/bootstrap.sh` dua kali (idempoten), menerapkan semua migrasi dari database kosong sebagai `poolrouter_owner`, lalu menjalankan uji SQL. Container dihapus di akhir.
- Role dibuat oleh `backend/db/bootstrap.sh` (superuser, sekali per cluster), bukan oleh migrasi. Password dibaca dari file secret (`$POOLROUTER_SECRETS_DIR/<nama role>`) lewat psql, tidak lewat argumen. Migrasi hanya memuat skema, fungsi, dan grant, dan berjalan sebagai `poolrouter_owner`.
- `.sqlx/`: belum ada kueri `query!` di issue ini, jadi belum ada isi untuk di-commit (ADR-0004 §8). Isi `.sqlx/` mulai saat kode akses data ditulis.
- Subcommand `migrate` (`compose.yaml` service `migrate`) belum menjalankan migrasi ini. Itu pekerjaan kode runtime, bukan skema. Kebutuhan wiring compose (role bootstrap sebelum `migrate`) adalah hand-off ke Platform & SRE.

## 2. Daftar kolom NULL untuk body yang tidak bisa di-parse (AC SHAA-193)

Aturan (contract §3.2.2 `validasi_gagal`, keputusan review desain Klaus — putaran 1, komentar 2026-11-03): kolom yang nilainya hanya bisa diperoleh dari body boleh NULL **hanya** pada baris `status = 'rejected'`, `reject_reason = 'validasi_gagal'`, `reject_http_status = 400`.

**Daftar final yang boleh NULL (kosong):** tidak ada kolom yang boleh NULL pada baris `validasi_gagal`/400. Alasan Klaus: body F0 hanya memuat field `{{input.*}}` dan metadata; field lain ditolak `validasi_gagal`/422 (contract §3.2.2, ADR-0002 D6), jadi pemilih pool di body tidak mungkin ada. Pool diambil dari konfigurasi (`downstream_endpoint.downstream_pool_id NOT NULL`); `prioritas`, `prioritas_efektif`, dan `ttl_expires_at` turun dari pool itu, dan `mode` turun dari path. Konsekuensinya `ttl_expires_at` tetap mengikuti `mode` di setiap baris, termasuk baris `rejected` async.

**Tetap wajib (NOT NULL), karena berasal dari autentikasi, path, atau konfigurasi:** `request_id`, `client_id`, `credential_id`, `downstream_endpoint_id`, `downstream_pool_id`, `prioritas`, `prioritas_efektif`, `provider_id`, `provider_config_version`, `endpoint_config_version`, `mode`, `status`, `metadata` (= `{}`), `received_at`, `completed_at` (baris `rejected` terminal), `attempt_count` (= 0), dan `ttl_expires_at` (mengikuti `mode`). `input_payload` NULL pada setiap baris `rejected` (P-01).

**Constraint (kata demi kata dari migrasi):**

```sql
CONSTRAINT chk_pool_kolom_not_null CHECK (downstream_pool_id IS NOT NULL AND prioritas IS NOT NULL AND prioritas_efektif IS NOT NULL),
CONSTRAINT chk_null_body_tak_terbaca CHECK (
  NOT (reject_reason = 'validasi_gagal' AND reject_http_status = 400) OR metadata = '{}'::jsonb),
CONSTRAINT chk_ttl_async CHECK ((mode = 'async') = (ttl_expires_at IS NOT NULL))
```

Diuji di `backend/db/test/20_constraints.sql`: `validasi_gagal`/400 diterima dengan kolom pool terisi dan `metadata = {}`; NULL pool ditolak pada `validasi_gagal`/400, `validasi_gagal`/422, `kuota_habis`, dan baris `queued`; `metadata` tidak kosong dan `input_payload` terisi ditolak; `ttl_expires_at` wajib pada baris `rejected` async dan dilarang pada baris `rejected` sync; `credential_id`, `received_at`, dan versi konfigurasi tetap ditolak NULL.

**Pertanyaan terbuka pada putaran 1 sudah terjawab:** pemilihan pool "manual" tidak lewat body — tidak ada sumber (contract, ADR-0001 §4.1) yang mengizinkan client memilih pool lewat body, dan pool diambil dari konfigurasi (`downstream_endpoint.downstream_pool_id NOT NULL`). Klaus mengonfirmasi pada putaran 1 (komentar 2026-11-03) bahwa daftar final yang boleh NULL adalah **kosong**, sehingga cabang NULL pada constraint lama dihapus tanpa migrasi data. `mode` diturunkan dari path (`POST` sync/async terpisah, ADR-0001 §4), jadi wajib.

## 3. Pemetaan contract §3 ke tabel

| Contract | Tabel | Catatan |
|---|---|---|
| §3.1 `request` | `request` | Semua 41 kolom ada (diuji `10_mapping.sql` dari daftar kolom contract) |
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
| ADR-0003 §3.6 audit login admin | `audit_login` (tabel terpisah, keputusan Klaus 2; kelas Terbatas, §6) | Append-only; `pr_admin_api` INSERT saja; login gagal tanpa `pengguna_id`; dilengkapi uji `20_constraints.sql` dan `30_roles.sql` |
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
| ID UUIDv7 | `uuidv7()` (SQL) untuk ID baris yang ditulis fungsi database (`audit_akses.audit_id`, `audit_konfigurasi.audit_konfigurasi_id`, `request_status_history`); ID baris lain dibuat aplikasi | PostgreSQL 17 tidak punya `uuidv7()`; fungsinya menyusun RFC 9562 dalam tepat 16 byte: 48 bit `unix_ts_ms`, versi 7, varian `10`, sisanya acak dari nibble/bit `gen_random_uuid()` yang tidak membawa bit tetap, jadi entropi unik ±66 bit (bukan 74). Uji bit versi/varian/`unix_ts_ms`, `length(uuid_send()) = 16`, dan keunikan di `40_functions.sql` |
| `audit_akses`: ADR-0001 §7.2 memberi `pr_admin_api` INSERT/SELECT | INSERT saja; baca lewat `fn_lihat_audit` | Setiap baca menulis baris `lihat_audit` (E-06); hanya auditor |
| T-04 "dalam transaksi yang sama" | Constraint trigger tertunda: status `request` harus punya baris history dengan `status_ke` sama saat commit | Menjadikan T-04 bisa diuji |
| Tabel di luar §3 yang dibutuhkan | `client_endpoint`, `downstream_pool_rute`, `provider_secret`, `konfigurasi_platform` (batas ekspor E-03 = 10.000), `admin_*`, `audit_konfigurasi`, `audit_login` | Contract menyebutnya tanpa mendefinisikan kolom (ADR-0001 §7, EN-04, E-03, ADR-0003 §3.6). Kelas data tiap tabel ada di §6 |
| Parameter dispatch pool (rate K-22, konkurensi, jeda acak K-05) | `upstream_pool.parameter_dispatch jsonb` | Contract §3 tidak mendefinisikannya. Dikunci ARC-70 (`ParameterDispatch` di `core::kuota::sumber_pg`): `rate.{tps,tpm,tph}.{t_mikro,burst}`, `timeout_total_upstream_detik` (default 30), `maks_konkurensi` (default 10, minimal 1), `jeda_acak_maks_mikro` (opsional; selalu dipotong ke satu slot token, maksimal 1 s). Semua kunci opsional; tanpa `rate` dispatcher tidak mendapat token |
| Login berhasil/gagal admin (ADR-0003 §3.6) | Tabel terpisah `audit_login` (keputusan Klaus 2) | `audit_akses` khusus akses data NIK dan `pengguna_id` NOT NULL, sedangkan login gagal untuk username tak dikenal tidak punya pengguna. Kolom: `audit_id`, `pengguna_id` (NULL bila username tak dikenal), `aksi IN ('login','logout')`, `hasil IN ('berhasil','gagal','terkunci')` (dengan `chk_sesi_wajib`: login `berhasil` dan logout wajib ber-`sesi_id`, logout hanya boleh `berhasil`; login `gagal`/`terkunci` tidak ber-`sesi_id`), `sesi_id`, `sumber_ip`, `user_agent`, `terjadi_at`; append-only; ditulis lewat `fn_catat_login` (`SECURITY DEFINER`, id UUIDv7), tidak ada peran yang bisa menulis tabelnya langsung; **belum ada yang bisa membacanya** — fungsi baca khusus auditor (spesifikasi review putaran 1, keputusan 2: hanya `auditor`, dan setiap panggilan menulis satu baris `audit_akses` `lihat_audit`) belum dibuat di issue ini; pembuatannya adalah pekerjaan issue backend Web UI **SHAA-201**. Sampai itu mendarat, trail login tidak dapat dibaca peran mana pun |
| Batas sinkronisasi `endpoint.log_mode` "hanya lebih ketat" | Belum ditegakkan di database | Contract menyerahkannya ke admin API (422) |
| M-R1 (endpoint Dukcapil wajib punya `status_penerima_manfaat`) | Belum ditegakkan di database | Aturan aktivasi endpoint, di admin API |
| Log server tidak boleh memuat NIK (R-04) | `bootstrap.sql`: `log_parameter_max_length_on_error = 0` dan `log_parameter_max_length = 0` per role aplikasi | `log_statement` bersifat superuser-only, jadi pilihan mencatat pernyataan tetap milik konfigurasi server target (ADR-0001 §7.1), bukan migrasi |

## 5. Role dan hak (ADR-0001 §7.2, contract §8.1, §8.1a, §8.1b)

Hak diberikan per kolom pada `request`, `transaction_log`, `client`, `client_kredensial`, dan `client_webhook`. Fungsi `SECURITY DEFINER` mengunci `search_path`, dan `EXECUTE` dicabut dari `PUBLIC`. Sejak `20261103000004_grants.sql` ini juga default: `ALTER DEFAULT PRIVILEGES FOR ROLE poolrouter_owner IN SCHEMA public REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC`, sehingga fungsi yang ditambahkan migrasi berikutnya tidak otomatis dapat dieksekusi `PUBLIC` (diuji `30_roles.sql`).

- `pr_api` **tidak** memiliki SELECT `nik`, `request_payload`, atau `response_payload` pada `transaction_log`. Sesuai §8.1 (akun layanan webserver punya hak **Tulis** NIK dan **Tulis input**, bukan baca) dan A-01 (NIK utuh dan payload hanya dibaca lewat fungsi `SECURITY DEFINER` beraudit, `pr_worker`, dan `pr_webhook`). Grant berbentuk kolom (bukan tingkat tabel) dan INSERT/UPDATE tetap memberi `pr_api` hak menulis NIK dan payload untuk jalur sync; trigger R-03 yang membatasi UPDATE. Uji: `30_roles.sql` (`42501` untuk `nik` / `request_payload` / `response_payload` / `SELECT *`, plus asersi katalog `information_schema.column_privileges`).
- `pr_controller`: tanpa SELECT `nik`, `input_payload`, `output_payload`, `metadata`, `request_payload`, `response_payload`; menutup log `tidak_diketahui` hanya lewat `fn_tutup_log_tidak_diketahui`; purge lewat fungsi.
- `pr_worker`: membaca `input_payload`; kolom status entitas saja (tanpa hash kredensial); `EXECUTE fn_purge_p02` (jalur gerbang, §8.1b).
- `pr_webhook`: 8 kolom `client_webhook`, allowlist, `output_payload` hanya lewat view `v_request_webhook` (baris terminal async); tidak punya hak atas `transaction_log`, `provider_secret`, `client`.
- `pr_admin_api`: tidak ada SELECT `nik`/payload; secret write-only; `aktif` entitas dan `status` kredensial hanya lewat fungsi (audit + pembatalan massal); view tersamar `v_request_tersamar`, `v_transaction_log_tersamar`; client owner lewat `fn_request_milik_client`.
- Uji: `30_roles.sql` (U-23, U-55, A-01, DQ-24 (a)(d), DQ-37 (f)), `40_functions.sql` (U-54, EN-08/DQ-37 (e), P-02/P-03/H-01, ekspor E-01..E-06, DQ-35).


Migrasi membuat grant kolom berdasarkan snapshot `information_schema` saat migrasi berjalan. Kolom yang ditambahkan migrasi berikutnya tidak otomatis mendapat grant; setiap grant baru harus eksplisit dan diuji.

## 6. Klasifikasi tabel tambahan (Klaus, blocking 7)

Kelas mengikuti kosakata data contract (Terbatas / Rahasia / Internal). Titik menandai kolom pembeda kelas; kolom lain Internal.

| Tabel | Kelas | Kolom |
|---|---|---|
| `admin_user` | Internal, dengan satu kolom **Terbatas** dan satu **Rahasia** | Terbatas: `username` (identitas login). Rahasia: `password_hash` (argon2id PHC; tidak pernah dikembalikan API). Internal: `pengguna_id`, `aktif`, `client_id`, `bahasa_ui`, `dibuat_at`, `diubah_at` |
| `admin_user_peran` | Internal | Semua kolom (`pengguna_id`, `peran`, `diajukan_oleh`, `disetujui_oleh`, `berlaku_sampai`, `diberikan_at`) |
| `admin_session` | Internal, dengan satu kolom **Rahasia** | Rahasia: `token_sha256` (hash token sesi; pemegang hash dapat menyamar). Internal: `sesi_id`, `pengguna_id`, `dibuat_at`, `terakhir_aktif_at`, `kedaluwarsa_at`, `user_agent` |
| `client_endpoint` | Internal | Semua kolom |
| `downstream_pool_rute` | Internal | Semua kolom |
| `provider_secret` | Internal, dengan kolom-kolom **Rahasia** | Rahasia: `secret_ciphertext`, `secret_dek_terbungkus`, `secret_nonce`, `kek_id` (bersama-sama materi kunci). Internal: `provider_id`, `nama_secret`, `secret_versi`, `diubah_at`, `diubah_oleh` |
| `konfigurasi_platform` | Internal | Semua kolom (`kunci`, `nilai`, `diubah_at`, `diubah_oleh`) |
| `audit_konfigurasi` | Internal | Semua kolom; `nilai_lama` / `nilai_baru` tidak pernah memuat nilai rahasia (komentar kolom) |
| `audit_login` (baru, §4) | **Terbatas** | `sumber_ip` dan `user_agent` bersifat identitas; username yang diketik tidak disimpan |

Rahasia tidak pernah keluar sebagai kolom yang dibaca peran aplikasi: `pr_api` hanya membaca `secret_ciphertext` `provider_secret` untuk dekripsi di proses, `pr_admin_api` menulis secret (write-only), dan `password_hash` / `token_sha256` tidak di-grant ke peran aplikasi mana pun.
