# Kode masalah (problem codes) PoolRouter

Dokumen ini adalah target dari field `type` pada setiap respons error RFC 9457 `application/problem+json` API downstream dan admin API (ADR-0001 rev 15 §5a.7): `type` = `https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#<kode>`. Setiap `kode` punya satu bagian dengan heading `### <kode>`; anchor sama persis dengan nilai `kode`. Himpunan anchor diuji otomatis terhadap enum `ProblemKode` (test `problem_kode_matches_docs` di `backend/crates/webserver/src/api/enums.rs`, dijalankan lewat `make -C backend test`).

Sumber: ADR-0001 rev 15 §4.1 (kontrak API downstream) dan §5a.7 (format error dan `Retry-After`); tambahan admin API dari ADR-0003 rev 5 dan inventaris desain `backend/openapi/DESIGN.md`. Nilai yang masih usulan desain (belum final di spec) diberi tanda *(usulan)*.

Aturan umum (§5a.7):
- Semua respons 4xx/5xx memakai `application/problem+json` dengan `type`, `title`, `status`, `kode`, dan opsional `detail`. Untuk 422 ada `errors` berisi nama field tanpa nilai.
- `type` dan `kode` berpasangan 1:1; client boleh mencocokkan `kode`.
- `Retry-After` (bila ada) berisi detik bulat, dibulatkan ke atas, minimal 1.
- `detail` tidak pernah memuat nilai input, NIK, atau secret.
- Body `penolakan_berlebih` hanya memuat `type`, `title`, `status`, dan `kode` (contract L-05).

---

### tidak_terautentikasi *(usulan, U-1)*

- **HTTP status:** 401.
- **Arti:** kredensial tidak valid, kedaluwarsa, hilang, atau tidak dikirim. Semua 401 memakai kode ini supaya penyebabnya tidak bocor; rincian alasan hanya ada di metrik MT-03. Tidak ada baris `request` dan tidak mengurangi kuota.
- **`Retry-After`:** tidak ada.
- **Tindakan client:** perbarui token (`POST /v1/oauth/token`) atau perbaiki kredensial, lalu ulangi request. Jangan mengulang tanpa mengganti kredensial.

### validasi_gagal

- **HTTP status:** 422 untuk request yang gagal validasi field; 400 untuk request yang tidak bisa diurai (JSON tidak valid, kunci ganda) atau parameter token endpoint tidak valid.
- **Arti:** payload di luar kontrak: field tak dikenal, nilai di luar rentang, template atau rujukan yang tidak seimbang, dsb. Untuk 422, `errors` berisi nama field tanpa nilai.
- **`Retry-After`:** tidak ada.
- **Tindakan client:** perbaiki field yang ditandai di `errors` (baca `GET /v1/endpoints/{endpoint_id}/kontrak` bila perlu), lalu ulangi dengan payload yang benar. Mengulang payload yang sama akan menghasilkan error yang sama.

### metadata_tidak_valid

- **HTTP status:** 422.
- **Arti:** field metadata melanggar kontrak endpoint (tipe, nilai di luar `nilai_diizinkan`, atau field wajib tidak ada).
- **`Retry-After`:** tidak ada.
- **Tindakan client:** perbaiki metadata sesuai kontrak endpoint, lalu ulangi.

### idempotency_key_tidak_didukung *(usulan, U-1)*

- **HTTP status:** 400.
- **Arti:** header `Idempotency-Key` dikirim ke endpoint sync. Mode sync F0 tidak mendukung header ini, agar client tidak mengira dirinya terlindungi (ADR-0001 §4.1).
- **`Retry-After`:** tidak ada.
- **Tindakan client:** hapus header `Idempotency-Key` dari request sync, atau pindah ke endpoint async bila proteksi idempotensi diinginkan.

### idempotency_key_konflik

- **HTTP status:** 422.
- **Arti:** header `Idempotency-Key` sama dengan request sebelumnya, tetapi sidik payload berbeda (ADR-0001 §4.1).
- **`Retry-After`:** tidak ada.
- **Tindakan client:** jangan mengulang request lama dengan kunci ini. Pakai kunci baru untuk payload baru; kunci sama hanya untuk retry payload yang identik.

### tidak_ditemukan *(usulan, U-1)*

- **HTTP status:** 404.
- **Arti:** resource tidak ada, bukan milik client yang memanggil, atau penugasan client↔endpoint tidak ada. Kasus "bukan milik client" tidak dibedakan dari "tidak ada" supaya tidak bocor; penolakan tetap tercatat.
- **`Retry-After`:** tidak ada.
- **Tindakan client:** periksa identifier dan kepemilikan resource. Jangan mengulang tanpa mengoreksi identifier.

### objek_nonaktif

- **HTTP status:** 403 bila objeknya endpoint; 503 bila objeknya downstream pool, upstream pool, atau provider.
- **Arti:** objek konfigurasi yang dituju sedang nonaktif (K-20). Respons 503 menyertakan `jenis_objek` ∈ `endpoint` | `downstream_pool` | `upstream_pool` | `provider`. Request async yang sudah masuk mengikuti alur K-20 (`cancelled` atau `failed`), bukan error ini.
- **`Retry-After`:** tidak ada, termasuk untuk 503 (putusan rev 6, tetap berlaku di rev 15).
- **Tindakan client:** berhenti mengirim ke objek tersebut; hubungi operator (peran PA) untuk reaktivasi lewat admin API. Jangan polling dengan interval pendek untuk menunggu objek aktif.

### kuota_habis

- **HTTP status:** 429.
- **Arti:** kuota efektif upstream pool (kuota harian − margin, atau batas harian atomik) terpakai habis; token tidak diberikan (ADR-0001 §5a.2).
- **`Retry-After`:** ada. Mode `bergulir_24j`: detik sampai ember tertua keluar dari jendela (kapasitas pertama pulih). Mode `kalender`: detik sampai `reset_kuota` berikutnya.
- **Tindakan client:** tunda pengiriman sesuai `Retry-After`, atau pakai mode async dan biarkan request masuk antrean (async 429 hanya terjadi saat limiternya menolak). Jangan menaikkan laju ulang di atas `Retry-After`.

### porsi_sync_habis

- **HTTP status:** 429.
- **Arti:** porsi laju request sync (`porsi_sync`) upstream pool sudah habis; request sync hanya diperbolehkan sebesar porsi itu (ADR-0001 §5a.2).
- **`Retry-After`:** ada; waktu sampai token GCRA berikutnya.
- **Tindakan client:** tunda request sync sesuai `Retry-After`, atau pindahkan beban ke mode async.

### penolakan_berlebih

- **HTTP status:** 429.
- **Arti:** limiter penolakan per client terpicu: client yang menerima penolakan beruntun dilimit agar tidak memperburuk beban (contract L-05; parameter limiter diatur per client di admin API).
- **`Retry-After`:** ada; waktu sampai token limiter penolakan berikutnya.
- **Tindakan client:** berhenti mengulang dan selidiki penyebab penolakan sebelumnya; body hanya berisi `type`, `title`, `status`, `kode`. Ulang setelah `Retry-After` dengan payload yang sudah diperbaiki.

### token_dibatasi *(usulan, U-1)*

- **HTTP status:** 429 di endpoint token (`POST /v1/oauth/token`).
- **Arti:** throttle endpoint token: GCRA 60/menit per `client_id` (burst 10) dan 120/menit per IP (ADR-0001 §5a.7).
- **`Retry-After`:** ada; waktu sampai token throttle berikutnya.
- **Tindakan client:** cache dan pakai ulang `access_token` sampai `expires_in` habis; jangan meminta token baru untuk setiap request. Ulang setelah `Retry-After`.

### tidak_bisa_dibatalkan

- **HTTP status:** 409.
- **Arti:** request yang dituju untuk pembatalan sedang `in_flight`, menunggu retry, atau sudah terminal, sehingga tidak bisa dibatalkan (ADR-0001 §4.1).
- **`Retry-After`:** tidak ada.
- **Tindakan client:** baca field `status` di body problem (status API saat ini); bila masih `queued`/`processing`, coba batalkan lagi atau tunggu hasil lewat polling/webhook. Bila sudah terminal, perlakukan respons terakhir sebagai hasil.

### layanan_tidak_tersedia

- **HTTP status:** 503.
- **Arti:** layanan sementara tidak dapat melayani, misalnya Redis gagal (fail-closed) atau aplikasi masih `belum_siap` saat start.
- **`Retry-After`:** ada; nilai 5 detik.
- **Tindakan client:** ulang setelah `Retry-After`. Bila berlanjut, gunakan backoff dan laporkan ke operator.

### retry_habis

- **HTTP status:** 502, hanya pada mode sync (`failed` tanpa error HTTP pada mode async).
- **Arti:** semua percobaan ke provider gagal dan batas percobaan (`batas_percobaan`) habis; `failure_reason` request = `retry_habis` (contract §3.2.5). Respons menyertakan `request_id`.
- **`Retry-After`:** tidak ada (putusan Shu SHAA-200, rev 15 §4.1).
- **Tindakan client:** jangan langsung mengulang; periksa status via polling/webhook bila datanya juga dikirim async, atau ajukan request baru setelah penyebab provider diketahui.

### provider_error_permanen

- **HTTP status:** 502, hanya pada mode sync.
- **Arti:** provider menolak secara permanen (bukan galat sementara); percobaan ulang tidak akan mengubah hasil. `failure_reason` request = `provider_error_permanen`. Respons menyertakan `request_id`.
- **`Retry-After`:** tidak ada.
- **Tindakan client:** perbaiki data atau konfigurasi penyebab penolakan (melalui operator/admin API), lalu ajukan request baru. Jangan mengulang payload yang sama.

### timeout_sync

- **HTTP status:** 504, hanya pada mode sync.
- **Arti:** panggilan provider melebihi batas waktu sync; hasil tidak diketahui pada saat respons. `failure_reason` request = `timeout_sync`. Respons menyertakan `request_id`.
- **`Retry-After`:** tidak ada.
- **Tindakan client:** jangan menganggap request gagal atau berhasil; idempotensi tidak dijamin pada sync. Bila data kritis, ulang melalui mode async atau gunakan pencarian berbasis `kode_transaksi` untuk cek hasil.

### ekspor_terlalu_besar

- **HTTP status:** 422 (admin API, ekspor auditor).
- **Arti:** hasil ekspor melebihi batas maksimal (default 10.000 baris, dapat dikonfigurasi); tidak ada berkas yang dibuat. Satu baris `audit_akses` (`hasil = ditolak`) tetap tercatat (ADR-0003 §3.6d).
- **`Retry-After`:** tidak ada.
- **Tindakan client (auditor):** persempit rentang waktu atau filter, lalu ulangi ekspor.

### kredensial_salah *(usulan)*

- **HTTP status:** 401 (login admin API/Web UI).
- **Arti:** username atau password salah. Pesan gagal tidak membedakan keduanya (ADR-0003 §3.6).
- **`Retry-After`:** tidak ada.
- **Tindakan client:** ulangi login dengan kredensial yang benar; perhatikan batas kegagalan berikutnya.

### login_dibatasi *(usulan)*

- **HTTP status:** 429 (login admin API/Web UI).
- **Arti:** 5 kegagalan login per username dalam 15 menit, sehingga login untuk username itu ditolak selama 15 menit (Redis; ADR-0003 §3.6).
- **`Retry-After`:** ada; sisa detik jendela penolakan.
- **Tindakan client:** tunggu sesuai `Retry-After`; jangan mencoba lagi sebelum itu. Bila memang lupa password, gunakan proses pemulihan yang disediakan operator.

### origin_tidak_valid *(usulan)*

- **HTTP status:** 403 (admin API, method selain GET/HEAD).
- **Arti:** pemeriksaan CSRF gagal: header `Origin` tidak sama dengan origin Web UI yang dikonfigurasi, atau `Sec-Fetch-Site` bukan `same-origin` (ADR-0003 §3.6).
- **`Retry-After`:** tidak ada.
- **Tindakan client:** panggil admin API dari Web UI resmi, bukan dari origin lain atau tool eksternal. Bukan kondisi yang bisa diperbaiki dengan mengulang.

### akses_ditolak *(usulan)*

- **HTTP status:** 403.
- **Arti:** sesi valid tetapi peran atau kebijakan akses tidak memperbolehkan aksi itu (termasuk penolakan allowlist IP jendela PoC). Setiap penolakan tercatat di `audit_akses` (`aksi = akses_ditolak`).
- **`Retry-After`:** tidak ada.
- **Tindakan client:** jangan mengulang dengan sesi yang sama; minta peran atau akses yang sesuai kepada peran PA melalui proses yang berlaku.

### pencarian_nik_dilarang *(usulan)*

- **HTTP status:** 403 (admin API).
- **Arti:** filter pencarian log memuat NIK atau potongannya, padahal pencarian berbasis NIK untuk operator dan client owner tidak ada (ADR-0001 §6; ADR-0003 §3.6b/§3.6e). Tercatat `akses_ditolak`.
- **`Retry-After`:** tidak ada.
- **Tindakan client:** gunakan kunci penelusuran yang diizinkan (`kode_transaksi`, `queue_id`, client, rentang waktu). Pencarian per NIK utuh hanya lewat endpoint auditor.

### secret_dipakai *(usulan)*

- **HTTP status:** 409 (admin API, `DELETE /admin/v1/secrets/{nama}`).
- **Arti:** secret masih dipakai minimal satu provider aktif, sehingga tidak boleh dihapus (US-02 AC4). Body menyertakan daftar provider aktif yang memakainya.
- **`Retry-After`:** tidak ada.
- **Tindakan client (PA):** alihkan provider yang memakai secret ke secret lain lewat admin API, baru hapus.

### kredensial_dicabut *(usulan)*

- **HTTP status:** 409 (admin API, `POST /admin/v1/credentials/{id}/aktifkan`).
- **Arti:** kredensial yang statusnya `dicabut` tidak bisa diaktifkan kembali (U-54b); rotasi/penggantian hanya lewat kredensial baru.
- **`Retry-After`:** tidak ada.
- **Tindakan client (PA/CO):** buat kredensial baru dengan `POST /admin/v1/clients/{id}/credentials`; secret baru ditampilkan sekali.

### batas_kredensial *(usulan)*

- **HTTP status:** 409 (admin API, `POST /admin/v1/clients/{id}/credentials`).
- **Arti:** jumlah kredensial aktif sudah mencapai batas (maksimal 2 aktif).
- **`Retry-After`:** tidak ada.
- **Tindakan client (PA/CO):** cabut kredensial aktif yang tidak terpakai (lewat pratinjau cabut → cabut), lalu buat kredensial baru.

### webhook_tidak_https *(usulan)*

- **HTTP status:** 422 (admin API, `PUT /admin/v1/clients/{id}/webhook`).
- **Arti:** `webhook_url` bukan HTTPS; webhook hanya dikirim ke HTTPS (ADR-0001 §5a.6).
- **`Retry-After`:** tidak ada.
- **Tindakan client (PA/CO):** daftarkan URL HTTPS (dengan domain yang diizinkan), lalu simpan ulang.

### webhook_domain_tidak_diizinkan *(usulan)*

- **HTTP status:** 422 (admin API, `PUT /admin/v1/clients/{id}/webhook`).
- **Arti:** domain `webhook_url` tidak ada di allowlist domain webhook; allowlist dicek saat simpan dan saat kirim.
- **`Retry-After`:** tidak ada.
- **Tindakan client (PA/CO):** gunakan domain yang sudah ada di allowlist, atau minta peran PA menambah domain lewat `PUT /admin/v1/webhook-allowlist/{domain}`.

### metadata_wajib_hilang *(usulan)*

- **HTTP status:** 422 (admin API, create/update endpoint).
- **Arti:** endpoint Dukcapil dikonfigurasi tanpa metadata wajib `status_penerima_manfaat` (aturan M-R1).
- **`Retry-After`:** tidak ada.
- **Tindakan client (PA):** tambahkan field metadata wajib pada konfigurasi endpoint, lalu simpan ulang.
