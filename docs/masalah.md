# Kode masalah PoolRouter

Dokumen ini adalah sumber rujukan untuk field `kode` pada body
`application/problem+json` (R-10, ADR-0001 section 5a.7). Setiap judul
(`# <anchor>`) berpasangan 1:1 dengan `kode` di `backend/openapi/*.yaml`; field
`type` pada body problem menunjuk anchor di berkas ini. Judul baru dilarang
tanpa perubahan `ProblemKode` (tests `masalah_anchors` menjaga kesetaraan).

Contoh di dokumen ini tidak memuat nilai input, NIK, atau secret (R-16).

# tidak_terautentikasi

401. Kredensial tidak valid, kedaluwarsa, atau tidak dikirim. Penyebab tidak
dibedakan di body; rincian alasan hanya ada di metrik MT-03. Downstream:
`WWW-Authenticate: Bearer`. Tidak ada baris request dan tidak memotong kuota.

# token_dibatasi

429 khusus `POST /v1/oauth/token`. Limiter GCRA per `client_id` (60/menit,
burst 10) atau per IP (120/menit) habis. Selalu dengan `Retry-After`; tanpa
field `error` RFC 6749.

# validasi_gagal

400 (body bukan JSON yang sah / `unsupported_grant_type` pada token) atau 422
(field tidak memenuhi aturan). 422 memuat `errors[]` per field; `batas` hanya
memuat parameter aturan, bukan nilai yang ditolak (R-16). Contoh: field
`NAMA_LGKAP` yang wajib tetapi kosong, atau nilai `NIK` yang panjangnya di luar
aturan. Nama field yang bentuknya tidak wajar (bukan `^[A-Za-z0-9_]{1,64}$`)
tidak di-echo; jumlahnya dilaporkan di `jumlah_nama_tidak_cocok` (integer,
minimal 1) dan field itu tidak muncul saat nilainya nol. Nama seperti
`TRESHOLD` bukan field kontrak, sehingga ditolak sebagai field tidak dikenal
(US-03), bukan sebagai nilai di bawah minimum.

# metadata_tidak_valid

422. Kunci atau nilai `metadata` tidak ada di kontrak endpoint
(`metadata_fields` versi terkini). `errors[]` menunjuk field metadata; nilai
tidak di-echo.

# idempotency_key_tidak_didukung

400. Header `Idempotency-Key` dikirim ke path sinkron (`/sync`). Header hanya
berlaku pada path asinkron.

# idempotency_key_konflik

422. Kunci `Idempotency-Key` yang sama dikirim ulang dengan sidik idempoten
yang berbeda. Request pertama tetap berjalan; klien harus memakai kunci baru
atau mengulang persis.

# kuota_habis

429. Kuota periode upstream pool (`bergulir_24j` atau `kalender` setelah
margin) sudah terpakai. Selalu dengan `Retry-After`: sisa waktu sampai ember
tertua keluar dari jendela (`bergulir_24j`) atau sampai `reset_kuota`
berikutnya (`kalender`), dalam detik bulat minimal 1.

# porsi_sync_habis

429. Porsi sync atau rate limit jam berjalan upstream pool habis. Satu kode
untuk keduanya. Selalu dengan `Retry-After` menuju token limiter berikutnya.

# penolakan_berlebih

429. Limiter penolakan client aktif (GCRA; burst dan isi ulang per detik dari
konfigurasi client, L-04). Body hanya empat field: `type`, `title`, `status`,
`kode` (R-10). Selalu dengan `Retry-After`.

# objek_nonaktif

403 atau 503, hanya untuk request baru saat submit (K-20). 403 =
downstream endpoint nonaktif (`jenis_objek: endpoint`). 503 = downstream pool,
upstream pool, atau provider di rute itu nonaktif (`jenis_objek`:
`downstream_pool`, `upstream_pool`, atau `provider`); request ditolak dan tidak
masuk antrean. Tanpa `Retry-After` (R-10).

Request yang sudah diterima tidak pernah dijawab 503; status akhirnya terlihat
lewat polling atau webhook (`cancelled` + `cancel_reason`, atau `failed` +
`failure_reason` `objek_nonaktif`).

# tidak_bisa_dibatalkan

409. `POST /v1/requests/{queue_id}/cancel` pada request berstatus `processing`
atau sudah terminal (`done`, `failed`, `expired`, `cancelled`). Body memuat
`status_api` saat ini.

# layanan_tidak_tersedia

503. Komponen tidak siap (Redis fail-closed atau ketergantungan lain). Selalu
dengan `Retry-After: 5` (ADR-0001 section 5a.7).

# retry_habis

502. Jendela retry habis; `attempt_count` efektif 1 di F0 (contract v0.7.10).
Body memuat `request_id`; tanpa `Retry-After`.

# provider_error_permanen

502. Kelas kegagalan provider tidak diulang (auth, kuota provider, kode tidak
dipetakan, respons tidak valid, TLS). Body memuat `request_id`; tanpa
`Retry-After`.

# timeout_sync

504. Panggilan sinkron tidak selesai dalam batas waktu (jeda maksimum + batas
waktu per percobaan). Body memuat `request_id`.

# tidak_ditemukan

404. Objek tidak dikenal. Pada endpoint downstream, resource milik client lain
dan yang tidak ada tidak dibedakan; penolakan tercatat di log akses.

# kredensial_salah

401 pada `POST /admin/v1/auth/login`. Username atau password salah; keduanya
tidak dibedakan.

# login_dibatasi

429 pada login. Lima kegagalan berturut dalam 15 menit per username; limiter
baru di-reset oleh login sukses (R-05). Selalu dengan `Retry-After`.

# origin_tidak_valid

403 pada operasi tulis admin. Header `Origin` tidak ada atau bukan origin yang
disetujui (rev 5 section 3.6b).

# akses_ditolak

403. Peran pengguna tidak termasuk kolom `Peran` operasi. Penolakan tercatat
di `audit_akses` (section 3.1).

# pencarian_nik_dilarang

403. Parameter pencarian log memuat NIK atau potongan NIK. Pencarian berbasis
NIK hanya melalui `searchLogByNik` (auditor, NIK persis 16 digit) dan selalu
tercatat di `audit_akses` (US-16).

# secret_dipakai

409. Secret masih direferensikan provider aktif; detail mencantumkan daftar
provider tersebut (US-02).

# kredensial_dicabut

409 saat mengaktifkan lagi kredensial yang sudah dicabut: pencabutan
permanen (ADR-0001 section 3a.2). Buat kredensial baru untuk mengganti.

# batas_kredensial

409. Klien sudah memiliki dua kredensial aktif (batas
`createCredential`, contract section 3.1b).

# webhook_domain_tidak_diizinkan

422. Host URL webhook tidak ada di allowlist; diperiksa saat menyimpan
konfigurasi dan saat pengiriman (US-06).

# webhook_tidak_https

422. URL webhook bukan `https://` (US-06).

# metadata_wajib_hilang

422. Endpoint Dukcapil disimpan tanpa field metadata wajib
`status_penerima_manfaat` (M-R1).

# ekspor_terlalu_besar

422. Ekspor log auditor di atas 10.000 baris; berkas tidak dibuat dan baris
`audit_akses` berstatus `ditolak` tetap tercatat (R-18).
