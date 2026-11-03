# Kode masalah PoolRouter

Dokumen ini adalah daftar rujukan untuk field `kode` pada body
`application/problem+json`. Setiap judul (`# <anchor>`) berpasangan 1:1 dengan
satu `kode`; field `type` pada body problem menunjuk anchor di berkas ini.
Judul baru hanya boleh ditambahkan bersama nilai `kode` yang baru.

Dokumen ini dipakai dua API: API downstream (diserahkan ke BGN) dan API admin
(internal). Kode yang hanya berlaku di API admin ditandai "Admin saja".

Contoh di dokumen ini tidak memuat nilai input, NIK, atau secret.

# tidak_terautentikasi

401. Kredensial tidak valid, kedaluwarsa, atau tidak dikirim. Penyebab tidak
dibedakan di body; rincian alasan hanya ada di metrik internal. Downstream:
`WWW-Authenticate: Bearer`. Tidak ada baris request dan tidak memotong kuota.

# token_dibatasi

429 khusus `POST /v1/oauth/token`. Limiter GCRA per `client_id` (60/menit,
burst 10) atau per IP (120/menit) habis. Selalu dengan `Retry-After`; tanpa
field `error` RFC 6749.

# validasi_gagal

400 (body bukan JSON yang sah / `unsupported_grant_type` pada token) atau 422
(field tidak memenuhi aturan). 422 memuat `errors[]` per field; `batas` hanya
memuat parameter aturan, bukan nilai yang ditolak. Contoh: field
`NAMA_LGKAP` yang wajib tetapi kosong, atau nilai `NIK` yang panjangnya di luar
aturan. Nama field yang bentuknya tidak wajar (bukan `^[A-Za-z0-9_]{1,64}$`)
tidak dikembalikan; jumlahnya dilaporkan di `jumlah_nama_tidak_cocok`
(integer, minimal 1) dan field itu tidak muncul saat nilainya nol. Nama seperti
`TRESHOLD` bukan field kontrak, sehingga ditolak sebagai field tidak dikenal,
bukan sebagai nilai di bawah minimum.

# metadata_tidak_valid

422. Kunci atau nilai `metadata` tidak ada di kontrak endpoint
(`metadata_fields` versi terkini). `errors[]` menunjuk field metadata; nilai
tidak dikembalikan.

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
konfigurasi client). Body hanya empat field: `type`, `title`, `status`, `kode`.
Selalu dengan `Retry-After`.

# objek_nonaktif

403 atau 503, hanya untuk request baru saat submit. 403 =
downstream endpoint nonaktif (`jenis_objek: endpoint`). 503 = downstream pool,
upstream pool, atau provider di rute itu nonaktif (`jenis_objek`:
`downstream_pool`, `upstream_pool`, atau `provider`); request ditolak dan tidak
masuk antrean. Tanpa `Retry-After`.

Request yang sudah diterima tidak pernah dijawab 503; status akhirnya terlihat
lewat polling atau webhook (`cancelled` + `cancel_reason`, atau `failed` +
`failure_reason` `objek_nonaktif`).

# tidak_bisa_dibatalkan

409. `POST /v1/requests/{queue_id}/cancel` pada request berstatus `processing`
atau sudah terminal (`done`, `failed`, `expired`, `cancelled`). Body memuat
`status_api` saat ini.

# layanan_tidak_tersedia

503. Komponen tidak siap (Redis fail-closed atau ketergantungan lain). Selalu
dengan `Retry-After: 5`.

# retry_habis

502. Jendela retry habis; di F0 hanya ada satu percobaan efektif, jadi jendela
ini langsung berakhir. Body memuat `request_id`; tanpa `Retry-After`.

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

401 pada `POST /admin/v1/auth/login`. Admin saja. Username atau password salah;
keduanya tidak dibedakan.

# login_dibatasi

429 pada login. Admin saja. Lima kegagalan berturut dalam 15 menit per username;
limiter baru di-reset oleh login sukses. Selalu dengan `Retry-After`.

# origin_tidak_valid

403 pada operasi tulis admin. Header `Origin` tidak ada atau bukan origin yang
disetujui.

# akses_ditolak

403. Admin saja. Peran pengguna tidak termasuk kolom `Peran` operasi. Penolakan
tercatat di `audit_akses`.

# pencarian_nik_dilarang

403. Admin saja. Parameter pencarian log memuat NIK atau potongan NIK. Pencarian
berbasis NIK hanya melalui `searchLogByNik` (auditor, NIK persis 16 digit) dan
selalu tercatat di `audit_akses`.

# secret_dipakai

409. Admin saja. Secret masih direferensikan provider aktif; detail mencantumkan
daftar provider tersebut.

# kredensial_dicabut

409. Admin saja, saat mengaktifkan lagi kredensial yang sudah dicabut: pencabutan
bersifat permanen. Buat kredensial baru untuk mengganti.

# batas_kredensial

409. Admin saja. Klien sudah memiliki dua kredensial aktif, yaitu batas untuk
`createCredential`.

# webhook_domain_tidak_diizinkan

422. Admin saja. Host URL webhook tidak ada di allowlist; diperiksa saat
menyimpan konfigurasi dan saat pengiriman.

# webhook_tidak_https

422. Admin saja. URL webhook bukan `https://`.

# metadata_wajib_hilang

422. Admin saja. Endpoint Dukcapil disimpan tanpa field metadata wajib
`status_penerima_manfaat`.

# ekspor_terlalu_besar

422. Admin saja. Ekspor log auditor di atas 10.000 baris; berkas tidak dibuat dan
baris `audit_akses` berstatus `ditolak` tetap tercatat.
