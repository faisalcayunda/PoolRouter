# SHAA-203 64 keys — v0.15

Keputusan: 64 key dipertahankan/diterbitkan dalam katalog v0.15. Sepuluh keputusan editorial di bawah ditandai **ubah dari `main`**; seluruh perubahan hanya pada key SHAA-203.

## Ringkasan
- ID: 469 key; EN: 475 key setelah diratakan.
- Diff v0.15 terhadap v0.14: 64 tambahan, 0 hilang, 0 berubah.
- `log.col.queue` dan `log.filter.queue` adalah key SHAA-203; final ID/EN: `queue_id` / `queue_id`.
- “Audit akses” final: `log.payload.reason.supporting` = `Wajib diisi. Pembukaan payload tercatat di audit akses.` / `Required. Opening the payload is recorded in the access audit.`; `log.payload.snackbar.opened` = `Payload dibuka. Akses ini tercatat di audit akses.` / `Payload opened. This access was recorded in the access audit.`

## Keputusan per key
| Key | Keputusan | Catatan |
|---|---|---|
| common.date.format_error | ubah dari `main` | istilah “tidak valid” / “not valid” |
| nik.page.export_help | ubah dari `main` | “valid”, bukan “sah” |
| quota.page.pool_unknown | ubah dari `main` | “Upstream pool” |
| log.col.request_id | ubah dari `main` | “ID request” / “Request ID” |
| log.filter.code_help | ubah dari `main` | “catatan” / “record” |
| export.error.too_large | ubah dari `main` | “catatan” / “records” |
| log.payload.reason.error | ubah dari `main` | instruksi “Isi alasan...” / “Enter a reason...” |
| common.date.order_error | ubah dari `main` | “tanggal mulai” / “start date” |
| log.col.queue | ubah dari `main` | `queue_id` |
| log.filter.queue | ubah dari `main` | `queue_id` |
| log.payload.reason.supporting | ubah dari `main` | “audit akses” / “access audit” |
| log.payload.snackbar.opened | ubah dari `main` | “audit akses” / “access audit” |

Daftar lengkap 64 key dan teks final ada pada `id.json` dan `en.json`; diff per key v0.14 tersedia di `diff-v0.15-v0.14-id.txt` dan `diff-v0.15-v0.14-en.txt`.
