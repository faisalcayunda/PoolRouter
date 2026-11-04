# Panduan Web UI PoolRouter F0: dasar untuk semua peran

Dokumen ini menjelaskan hal yang sama untuk keempat peran Web UI PoolRouter F0: cara masuk, cara membaca layar, perilaku sesi, dan cara mengganti bahasa tampilan. Panduan per peran ada di dokumen terpisah (daftar di bagian akhir). Semua contoh memakai data sintetis; NIK contoh hanya `0000000000000000`.

- **Pembaca:** pengguna Web UI PoolRouter F0 (platform admin, client owner, operator, auditor)
- **Tanggal:** 2026-11-04
- **Versi:** draf 0.1, dicocokkan dengan `main` hash `c6829a25`
- **Pemilik:** tim PoolRouter
- **Status:** draf; bagian bertanda **Belum diuji** belum dicek pada aplikasi yang berjalan

## Acuan dan status verifikasi

Nama tombol, menu, dan pesan mengikuti katalog UX copy v0.14. Perilaku sesi mengikuti spec UI v0.3.2.

Penanda di seluruh panduan:

| Penanda | Arti |
|---|---|
| **Cocok dengan kode** | Langkah dicocokkan dengan kode layar di `main` (`c6829a25`) lewat pembacaan kode. Aplikasi belum dijalankan untuk panduan ini. |
| **Belum diuji** | Layar atau perilaku belum ada di `main`, jadi langkah hanya berdasar spec dan katalog. Dicek ulang setelah layarnya tersedia. |
| **Selisih label** | Kode di `main` masih memakai teks katalog lama (v0.13); teks v0.14 dipakai di panduan. |

Tangkapan layar belum disertakan: build belum bisa dijalankan dan difoto dengan data sintetis. Tangkapan layar ditambahkan setelah UI F0 lengkap.

## Menu per peran

Menu yang tidak sesuai peran tidak tampil. Mengetik alamat halaman secara langsung tetap ditolak dan percobaannya dicatat.

| Menu | Platform admin | Client owner | Operator | Auditor |
|---|---|---|---|---|
| Provider, Secret, Upstream pool, Downstream pool | ✔ | – | – | – |
| Client dan kredensial, Webhook | ✔ | ✔ (milik sendiri) | – | – |
| Kuota | ✔ | – | ✔ (baca-saja) | – |
| Log transaksi | – | – | ✔ | ✔ |
| Pencarian log per NIK, Audit akses | – | – | – | ✔ |
| Pengaturan | ✔ | ✔ | ✔ | ✔ |

Operator hanya dapat membaca Kuota.

## Masuk

1. Buka alamat PoolRouter yang diberikan Pusdatin BGN.
2. Isi **Nama pengguna** dan **Kata sandi**, lalu pilih **Masuk**.
3. Setelah berhasil, Anda masuk ke halaman awal sesuai peran.

Pesan yang mungkin tampil:

| Pesan | Arti | Yang dilakukan |
|---|---|---|
| Nama pengguna atau kata sandi tidak cocok. | Data masuk salah. | Periksa isian, coba lagi. |
| Terlalu banyak percobaan. Coba lagi dalam 15 menit. | Terlalu banyak percobaan gagal. | Tunggu 15 menit, lalu coba lagi. |
| Sesi Anda berakhir. Masuk lagi untuk melanjutkan. | Sesi habis (lihat bagian berikut). | Masuk lagi. |

Halaman Masuk selalu berbahasa Indonesia, termasuk untuk pengguna yang memilih English. Status: **belum diuji** (layar Masuk belum ada di `main`).

## Mengenali layar

- **Lompat ke konten:** tautan pertama saat menekan Tab; langsung menuju isi halaman.
- **Menu navigasi:** di layar lebar (1200 px ke atas) menu selalu tampil di kiri. Di layar lebih sempit, buka lewat tombol **Buka menu navigasi**; Esc menutupnya.
- **Judul tab browser:** berpola `{Layar} · PoolRouter`, misalnya `Kuota · PoolRouter`. NIK tidak pernah tampil di judul.
- **Tombol Simpan** tidak dimatikan ketika isian belum valid. Tekan Simpan, maka pesan galat tampil di bawah kolom yang bermasalah dan fokus pindah ke sana.
- **Notifikasi singkat** (misalnya "Provider tersimpan") hilang sendiri setelah 4 detik. Notifikasi tidak memuat NIK atau secret.
- **Gagal memuat:** pesan galat tampil dengan tombol **Coba lagi**. Data yang sudah tampil tetap dipertahankan beserta waktu pemuatannya.
- **403:** "Anda tidak punya akses ke halaman ini". Percobaan dicatat. Hubungi platform admin bila Anda perlu akses; tombol **Ke halaman awal** membawa Anda kembali.

Status bagian ini: **cocok dengan kode** untuk menu navigasi, Lompat ke konten, judul tab, dan pesan 403; sisanya **belum diuji**.

## Sesi: batas waktu dan peringatan

PoolRouter membatasi sesi dengan dua cara. Angka ini tidak dapat diubah pengguna.

| Batas | Nilai | Bisa diperpanjang? |
|---|---|---|
| Tidak aktif (idle) | 30 menit sejak permintaan terakhir ke server | Ya |
| Batas mutlak | 8 jam sejak masuk | Tidak (kontrol keamanan) |

Yang dihitung sebagai aktivitas adalah permintaan ke server. Mengetik di formulir atau menggeser halaman tidak mereset batas idle. Karena itu pengguna yang sedang mengetik di formulir panjang tanpa menyimpan tetap bisa mendapat peringatan; ini perilaku yang benar.

### Peringatan idle (2 menit sebelum berakhir)

1. Dialog **Sesi akan berakhir** tampil. Isinya: "Sesi Anda berakhir pukul {HH.mm} WIB. Pilih "Perpanjang sesi" untuk tetap masuk." Fokus pindah ke tombol **Perpanjang sesi**.
2. Pilih **Perpanjang sesi** (Enter atau klik) untuk tetap masuk. Dialog menutup, fokus kembali ke kolom yang Anda pakai, dan tampil notifikasi "Sesi diperpanjang". Isian yang sedang diketik tidak hilang.
3. Pilih **Keluar** bila ingin mengakhiri sesi.
4. Esc dan klik di luar dialog tidak menutup dialog ini.
5. Bila perpanjangan gagal karena jaringan atau galat server, dialog tetap terbuka dengan pesan "Sesi belum bisa diperpanjang. Coba lagi." Bila sesi ternyata sudah berakhir, Anda diarahkan ke halaman Masuk.
6. Bila tidak ada tindakan, Anda diarahkan ke halaman Masuk dengan pesan "Sesi Anda berakhir. Masuk lagi untuk melanjutkan."

Waktu berakhir tertulis sebagai teks tetap. Hitung mundur "Sisa waktu m:dd" boleh tampil di layar tetapi tidak dibacakan pembaca layar setiap detik.

### Peringatan batas 8 jam

1. Dua menit sebelum batas 8 jam, dialog **Batas sesi hampir tercapai** tampil: "Demi keamanan, sesi dibatasi 8 jam dan tidak bisa diperpanjang. Sesi berakhir pukul {HH.mm} WIB. Simpan isian Anda sekarang, lalu masuk lagi."
2. Tidak ada tombol perpanjang. Pilih **Mengerti** (atau Esc) untuk menutup dialog.
3. Setelah ditutup, banner kuning menetap di bawah bilah atas semua halaman: "Sesi berakhir pukul {HH.mm} WIB dan tidak bisa diperpanjang. Simpan isian Anda sekarang." Banner tidak bisa ditutup.
4. Simpan pekerjaan Anda sebelum waktu berakhir.

### Setelah masuk kembali

Isi halaman yang memuat NIK (pencarian per NIK, kolom NIK di log) tidak dipulihkan setelah masuk kembali. Layar lain tidak menjanjikan pemulihan isian; PoolRouter tidak menyimpan draf otomatis.

Status bagian sesi: **belum diuji**. Dialog dan banner sesi belum ada di `main` (katalog `session.*` baru terbit di v0.14). Perilaku mengikuti spec v0.3.2 §12.

## Mengganti bahasa tampilan (Pengaturan, layar S17)

Tersedia untuk semua peran. Bahasa yang tersedia: **Bahasa Indonesia** (bawaan) dan **English**.

1. Buka menu navigasi, bagian **Akun**, lalu pilih **Pengaturan**. Cara lain pada layar lebar: pilih nama pengguna di bilah atas, lalu **Pengaturan**.
2. Pada kartu **Bahasa**, pilih **Bahasa Indonesia** atau **English** di bawah "Bahasa tampilan".
3. Pilihan terkunci sebentar dengan teks "Menyimpan…". Setelah tersimpan, tampilan langsung berganti dan tampil notifikasi dalam bahasa baru: "Bahasa tampilan diubah ke {bahasa}."
4. Bila gagal, pilihan kembali ke bahasa semula dan tampil "Bahasa belum bisa disimpan. Tampilan tidak berubah. Coba lagi." dengan tombol **Coba lagi**.

Yang perlu diketahui:

- Pilihan tersimpan di akun Anda di server, sehingga berlaku di semua perangkat.
- Akun yang belum pernah memilih memakai Bahasa Indonesia.
- Yang diterjemahkan: menu, label, dan pesan di layar. Format tanggal, waktu, dan angka mengikuti bahasa yang dipilih; zona waktu tetap WIB. Contoh format Indonesia: `3 Nov 2026, 14.30 WIB · 1.000.000`.
- Yang tidak diterjemahkan: data yang Anda isi (nama provider, nama pool, dan sejenisnya), isi respons API untuk client, dan log.
- Halaman Masuk dan pesan sesi berakhir selalu berbahasa Indonesia, apa pun pilihan Anda.
- Bahasa lain di luar dua bahasa itu belum tersedia.

Status: **belum diuji** (layar Pengaturan belum ada di `main`).

## Panduan per peran

| Peran | Dokumen |
|---|---|
| Platform admin | `web-ui-f0-platform-admin.md` |
| Client owner | `web-ui-f0-client-owner.md` |
| Operator | `web-ui-f0-operator.md` |
| Auditor | `web-ui-f0-auditor.md` |

Riwayat perubahan: 0.1 (2026-11-04), draf awal.
