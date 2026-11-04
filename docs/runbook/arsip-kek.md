## Ringkasan

Runbook ini mengatur kunci KEK arsip (R-24a, ADR-0006 rev 4 §8.1): cara membuat pasangan kunci per lingkungan, memasangnya sebagai secret, merotasinya, dan tindakan operator bila job arsip gagal tertutup. Proses arsip hanya memegang kunci publik. Proses restore, yang berjalan di deployment worker terpisah, memegang kunci privat. Rotasi KEK bukan respons kebocoran kunci privat; kebocoran ditangani sebagai insiden terpisah (F1-T12).

- **Pemilik:** Tetsu (Platform & SRE), reviewer Huan
- **Tanggal:** 2026-11-04
- **Status:** draf. Langkah rotasi (v-b) bergantung pada F1-T13 (fungsi audit untuk `pr_restore`, Klaus) dan belum bisa dijalankan sampai F1-T13 ditutup.

## Istilah dan pemetaan

| Nama | Isi | Dipasang di |
|---|---|---|
| `POOLROUTER_ARSIP_KEK_ID` | Pengenal pasangan kunci (`nik_kek_id`), bukan kunci | worker arsip dan restore (variabel biasa) |
| `POOLROUTER_ARSIP_KEK_PUBLIK_FILE` | Kunci publik X25519 aktif (secret `poolrouter_arsip_kek_publik`) | worker arsip saja |
| `POOLROUTER_ARSIP_KEK_PRIVAT_FILE` | Kunci privat X25519 untuk satu `nik_kek_id` (secret `poolrouter_arsip_kek_privat`) | worker restore saja |

Kunci privat tidak pernah ada di deployment worker arsip, worker request, controller, webserver, atau job rotasi. Variabel `POOLROUTER_ARSIP_KEK_PRIVAT_FILE` dan secret `poolrouter_arsip_kek_privat` hanya ada di `backend-worker-restore`. Kredensial database `pr_rotasi_kek` hanya ada di job `rotasi-kek-arsip`.

### Format `nik_kek_id`

- Panjang maksimum 100 karakter.
- Pola: `arsip-x25519-<YYYY>-<MM>` dengan sufiks opsional `-<a-z0-9, maks 20 karakter>`. Contoh: `arsip-x25519-2026-11`, `arsip-x25519-2027-05-rot2`.
- Pengenal ini dicatat di file env lingkungan dan di audit konfigurasi setiap rotasi. Ia tidak rahasia, tetapi tidak boleh berisi nilai kunci.

## Prasyarat

- Podman dengan machine yang sudah berjalan, dan OpenSSL 3 atau lebih baru (untuk `genpkey -algorithm X25519`). Jangan memasang atau mengubah keduanya dari runbook ini.
- Direktori secret di luar repo, mode `0700`. File secret mode `0444` sesuai `docs/runbook/deploy.md`.
- Tidak ada kunci di repo, issue, komentar, atau argumen perintah. Gunakan `${VAR}` di template.

## 4.1 Membuat pasangan kunci per lingkungan

Dikerjakan sekali per lingkungan, dan lagi saat rotasi (langkah (i) di 4.3). Jalankan di shell lokal, di direktori sementara di dalam direktori secret. Kunci privat tidak boleh masuk riwayat shell atau repo.

```sh
# Ganti <ID> dengan nik_kek_id yang akan dipakai, mis. arsip-x25519-2026-11.
ID=<ID>
SECRETS_DIR="$POOLROUTER_SECRETS_DIR"
umask 077
openssl genpkey -algorithm X25519 -out "$SECRETS_DIR/.arsip_priv_$ID.pem"
openssl pkey -in "$SECRETS_DIR/.arsip_priv_$ID.pem" -pubout -out "$SECRETS_DIR/.arsip_pub_$ID.pem"
```

Nama file secret di compose:

- kunci publik aktif: `poolrouter_arsip_kek_publik` (isi PEM publik)
- kunci privat aktif: `poolrouter_arsip_kek_privat` (isi PEM privat)

Pindahkan hasilnya ke nama secret, lalu atur mode dan hapus berkas sementara:

```sh
mv "$SECRETS_DIR/.arsip_pub_$ID.pem"  "$SECRETS_DIR/poolrouter_arsip_kek_publik"
mv "$SECRETS_DIR/.arsip_priv_$ID.pem" "$SECRETS_DIR/poolrouter_arsip_kek_privat"
chmod 0444 "$SECRETS_DIR/poolrouter_arsip_kek_publik" "$SECRETS_DIR/poolrouter_arsip_kek_privat"
```

Isi kunci tidak ditampilkan. Jangan `cat` berkas privat di terminal yang tercatat.

### Uji pasangan kunci (butir 11 b)

Kunci publik yang dihitung ulang dari kunci privat harus identik dengan kunci publik yang dipasang. Uji ini tidak menampilkan kunci:

```sh
diff -q <(openssl pkey -in "$SECRETS_DIR/poolrouter_arsip_kek_privat" -pubout) \
        "$SECRETS_DIR/poolrouter_arsip_kek_publik" \
  && echo "pasangan kunci cocok" || echo "pasangan kunci TIDAK cocok"
```

Lulus bila keluarannya `pasangan kunci cocok` dan exit code 0. Jangan lanjut bila tidak cocok.

Uji ujung ke ujung (butir 11 a, di `poc`/`prod`) dilakukan dengan `uji-buka-kek` setelah proses restore dan arsip berjalan. Di dev, cukup uji pasangan kunci di atas.

## 4.2 Memasang secret di compose

1. Isi `POOLROUTER_ARSIP_KEK_ID` di file env lingkungan dengan `nik_kek_id` yang aktif.
2. Pastikan secret `poolrouter_arsip_kek_publik` dan `poolrouter_arsip_kek_privat` ada di `POOLROUTER_SECRETS_DIR`.
3. `make compose-config ENV=<env> ENV_FILE=<file>`. Harus lulus.
4. Periksa penempatan kunci di hasil render. Perintah ini hanya menampilkan nama variabel dan nama secret, tanpa nilai:

   ```sh
   podman compose ... config | grep -E 'ARSIP_KEK_(PUBLIK|PRIVAT)_FILE|poolrouter_arsip_kek_'
   ```

   Hasil yang benar: `PRIVAT` hanya muncul di blok `backend-worker-restore`, `PUBLIK` hanya di `backend-worker-arsip`.

## 4.3 Rotasi KEK arsip (ADR-0006 rev 4 §8.1 butir 13, U-90)

Rotasi tidak menulis ulang objek arsip. Ia hanya membungkus ulang `dek_terbungkus` di manifest, sehingga `sha256` tetap sama. Setiap langkah dicatat di audit konfigurasi dengan alasan. Di `poc`/`prod`, perintah rotasi memerlukan verifikasi password dan TOTP pelaku (§9 butir 6a). Job rotasi dijalankan oleh identitas dan role sendiri, bukan oleh operator dengan kunci privat.

Urutan wajib:

- **(i) Buat pasangan baru.** Ikuti 4.1 dengan `nik_kek_id` baru. Pasang kunci privat baru di worker restore, dan biarkan kunci lama tetap terpasang. Dalam compose, tambahkan secret dan slot kedua sesuai keputusan Platform & SRE saat itu.
- **(ii) Uji pasangan kunci.** Jalankan uji 4.1 untuk id baru.
- **(iii) Pasang kunci publik baru.** Ganti `POOLROUTER_ARSIP_KEK_ID` dan `poolrouter_arsip_kek_publik` ke id baru, lalu `compose-up` untuk worker arsip. Objek baru memakai id baru.
- **(iv-0) Cadangan database terverifikasi sebelum rotasi.** Ambil cadangan `pg_dump -Fc` dan buktikan dapat dibaca dengan `pg_restore --list`. Catat jendela PITR yang tersedia. Cadangan dan jendela PITR ini dipakai sebagai dasar gerbang (v-c). Sampai prosedur cadangan Platform & SRE ditetapkan, langkah ini tidak bisa dijalankan dan rotasi ditunda.
- **(iv) Bungkus ulang.** Job rotasi membuka setiap `dek_terbungkus` ber-id lama, membungkusnya ulang dengan kunci publik baru, lalu memperbarui `dek_terbungkus` dan `nik_kek_id` dalam satu transaksi per manifest.
  - Baris yang dilewati CAS (baris yang berubah saat diproses) dicatat dan diulang di putaran berikutnya. Jangan memaksa update pada baris itu.
  - Hitung sisa manifest dengan `arsip_manifest` dan id lama, dan pantau hingga nol.
- **(v) Pemusnahan kunci privat lama.** Dilakukan hanya setelah tiga gerbang berikut lolos.
  - **(v-a) Nol manifest merujuk id lama.** Verifikasi dengan kueri hitung. Hasil harus `0`:

    ```sh
    psql "$DATABASE_URL_ROTASI" -v kid="$ID_LAMA" -At -c \
      'SELECT count(*) FROM arsip_manifest WHERE nik_kek_id = :'\''kid'\'';'
    ```

    `DATABASE_URL_ROTASI` tidak boleh berisi password. Pakai `PGPASSFILE` atau sesi interaktif. Perintah ini tidak boleh dijalankan dengan password di argumen.
  - **(v-b) Uji buka seluruh manifest id baru oleh restore (`uji-buka-kek`).** Restore membuka `dek_terbungkus` setiap manifest ber-id baru, tanpa menulis keluaran. Gerbang ini bergantung pada F1-T13 (EXECUTE fungsi audit untuk `pr_restore`, Klaus). Sampai F1-T13 ditutup, gerbang (v-b) tidak lulus dan kunci privat lama tidak boleh dimusnahkan.
  - **(v-c) Cadangan dan jendela PITR ber-id lama kedaluwarsa, atau dinyatakan tertulis tidak dipakai untuk restore arsip.** Cadangan dari (iv-0) dan jendela PITR sebelum rotasi harus sudah kedaluwarsa, atau owner dan Platform & SRE menandatangani pernyataan tertulis di issue bahwa cadangan itu tidak akan dipakai untuk restore arsip. Pernyataan ini mention Luca bila menyangkut keputusan owner.
- Setelah (v-a), (v-b), dan (v-c) lolos, hapus secret kunci privat lama dari `POOLROUTER_SECRETS_DIR` dan dari deployment restore. Catat pemusnahan di audit konfigurasi dan di issue tanpa menulis nilai kunci.

Rotasi bukan respons kebocoran kunci privat. Bila ada dugaan kebocoran, hentikan prosedur, lalu ikuti jalur insiden (F1-T12). Jangan memusnahkan kunci sebagai tanggapan atas kebocoran tanpa keputusan Huan.

## 4a. Job arsip gagal tertutup atau gagal berulang (ADR-0006 rev 4 §8.1 butir 7)

Job arsip gagal tertutup bila jumlah sel melewati L = 2^28 per objek (RA-1 belum diputuskan), atau bila gagal berulang. Saat itu objek tidak ditulis, manifest tidak dibuat, dan baris online tetap ada. Alert job arsip gagal (D5) dan aturan alert Shiro menyala dengan severity tertinggi dan tenggat tindakan 1 hari kerja.

Tindakan operator:

1. **Tahan retensi partisi.** Jangan drop atau detach partisi yang satuan arsipnya gagal. Penghapusan partisi hanya boleh dilakukan setelah objek terverifikasi.
2. **Diagnosis.** Lihat metrik `poolrouter_arsip_job_total{hasil}` dan log job tanpa NIK atau isi sel. Catat `tabel_sumber`, `provider_id`, dan tanggal partisi WIB.
3. **Tentukan penyebab.**
   - Batas L terlampaui: jangan pecah objek secara manual. Pemecahan belum punya bentuk di manifest (RA-1).
   - Gagal berulang karena kesalahan konfigurasi atau kunci: perbaiki konfigurasi lalu jalankan ulang satuan yang sama. Jalan ulang menimpa objek dan DEK dengan kunci yang sama (idempoten).
   - Kunci publik tidak cocok dengan kunci privat restore: hentikan job arsip dan lakukan uji 4.1 sebelum jalan ulang.
4. **Keputusan jalan ulang.** Jalan ulang hanya setelah penyebab jelas dan dicatat di issue. Jika tidak jelas, jangan jalankan ulang.
5. **Eskalasi ke Shu** dengan mention link, bila L terlampaui, jika jalan ulang gagal dua kali, atau jika ada indikasi data tidak konsisten. Jangan menunggu tenggat 1 hari kerja tanpa laporan.

## Hardening proses arsip dan restore (butir 8)

Konfigurasi di `deploy/compose/compose.yaml` (`x-worker-hardened`) untuk `backend-worker-arsip` dan `backend-worker-restore`:

- tanpa `ports`, tanpa inbound
- `read_only: true`, `tmpfs: [/tmp]`
- `cap_drop: [ALL]`, `no-new-privileges:true`
- `ulimits.core: 0` (core dump mati)
- `mem_limit` sama dengan `memswap_limit` (tanpa swap)

Egress hanya ke PostgreSQL, bucket, dan secret manager tidak bisa dinyatakan di compose. Pembatasannya dilakukan di jaringan host atau di firewall host target, dan belum terpasang di dev. Ini dicatat sebagai temuan.

## Batasan saat ini

- Mode worker (`POOLROUTER_WORKER_MODE=arsip|restore`) dan kontrak variabel `POOLROUTER_ARSIP_KEK_*_FILE` bergantung pada implementasi D6 (SHAA-341). Sampai itu, service ini tidak bisa berjalan.
- Role `pr_job_arsip` dan `pr_restore` belum dibuat (SHAA-193). Job `rotasi-kek-arsip` dikerjakan Sid.
- Gerbang (v-b) bergantung pada F1-T13.
- Format pasangan kunci di secret (PEM X25519) menunggu konfirmasi kontrak implementasi (SHAA-341).

## Rujukan

- ADR-0006 rev 4 §8.1 butir 7 sampai 13 (`ADR-0006-delta-arsitektur-f1-poolrouter-rev4.md`, lampiran SHAA-336)
- Data contract v0.9.6: AR-02b, AR-02c, F1-6.2, §6.5, U-90, U-91
- `docs/runbook/deploy.md` untuk perintah compose, secret, dan pembersihan
