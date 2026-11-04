# Diff v0.15 terhadap main

## id
- **common.date.format_error — ubah dari `main`**
  - Lama: `Tanggal belum sah. Tulis dengan format 03/11/2026.`
  - Baru: `Tanggal tidak valid. Tulis dengan format 03/11/2026.`
- **nik.page.export_help — ubah dari `main`**
  - Lama: `Masukkan NIK 16 digit yang sah untuk mengaktifkan ekspor.`
  - Baru: `Masukkan NIK 16 digit yang valid untuk mengaktifkan ekspor.`
- **quota.page.pool_unknown — ubah dari `main`**
  - Lama: `Pool upstream`
  - Baru: `Upstream pool`
- **log.col.request_id — ubah dari `main`**
  - Lama: `ID permintaan`
  - Baru: `ID request`
- **log.filter.code_help — ubah dari `main`**
  - Lama: `Cari rekaman menggunakan kode transaksi.`
  - Baru: `Cari catatan menggunakan kode transaksi.`
- **export.error.too_large — ubah dari `main`**
  - Lama: `Hasil terlalu besar. Persempit rentang waktu hingga paling banyak 10.000 record.`
  - Baru: `Hasil terlalu besar. Persempit rentang waktu hingga paling banyak 10.000 catatan.`
- **log.payload.reason.error — ubah dari `main`**
  - Lama: `Alasan wajib diisi dengan 10–500 karakter.`
  - Baru: `Isi alasan sepanjang 10–500 karakter.`
- **common.date.order_error — ubah dari `main`**
  - Lama: `Tanggal akhir harus sama dengan atau setelah tanggal awal.`
  - Baru: `Tanggal akhir harus sama dengan atau setelah tanggal mulai.`
- **log.col.queue — ubah dari `main`**
  - Lama: `Queue ID`
  - Baru: `queue_id`
- **log.filter.queue — ubah dari `main`**
  - Lama: `Queue ID`
  - Baru: `queue_id`
- **log.payload.reason.supporting — ubah dari `main`**
  - Lama: `Wajib diisi. Pembukaan payload tercatat di audit.`
  - Baru: `Wajib diisi. Pembukaan payload tercatat di audit akses.`
- **log.payload.snackbar.opened — ubah dari `main`**
  - Lama: `Payload dibuka. Akses ini tercatat di audit.`
  - Baru: `Payload dibuka. Akses ini tercatat di audit akses.`
## en
- **common.date.format_error — ubah dari `main`**
  - Lama: `The date is not valid yet. Write it as 03/11/2026.`
  - Baru: `The date is not valid. Write it as 03/11/2026.`
- **log.col.queue — ubah dari `main`**
  - Lama: `Queue ID`
  - Baru: `queue_id`
- **log.filter.queue — ubah dari `main`**
  - Lama: `Queue ID`
  - Baru: `queue_id`
- **log.payload.reason.supporting — ubah dari `main`**
  - Lama: `Required. Opening the payload is recorded in the audit log.`
  - Baru: `Required. Opening the payload is recorded in the access audit.`
- **log.payload.snackbar.opened — ubah dari `main`**
  - Lama: `Payload opened. This access was recorded in the audit log.`
  - Baru: `Payload opened. This access was recorded in the access audit.`

Perbedaan key katalog: 9 key `session.*` ada di v0.15 tetapi tidak ada di `main`.
