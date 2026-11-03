# ADR-0002: Adapter provider HTTP untuk Dukcapil Verify By Element — body pada GET, parsing respons status+skor, jaringan privat, dan trust TLS per provider

**Ringkasan:** Dokumentasi API Dukcapil "Verify By Element" (diterima owner 2026-11-03) memuat empat hal yang belum ditangani ADR-0001: GET dengan JSON body, respons berupa string `Sesuai`/`Tidak Sesuai` dengan skor opsional di dalam envelope pagination, base URL berupa IP privat lewat HTTPS port 8000, dan sertifikat yang kemungkinan self-signed. Selain itu, `transactionId` wajib dikirim tetapi tidak dikembalikan. ADR ini memutuskan lima hal: (D1) template request tidak lagi terikat method, sehingga body boleh dikirim pada GET; (D2) namespace template `{{system.kode_transaksi}}` untuk K-25; (D3) parser respons bawaan `status_skor` yang fail-closed; (D4) egress ke provider hanya dari webserver dan worker lewat IP egress tetap dan allowlist jaringan; (D5) trust TLS per provider dengan mode `system`, `ca_kustom`, atau `pin_spki`, tanpa mode yang mematikan verifikasi. Rev 2 menutup P-1 (elemen yang tidak dikirim client dihilangkan dari output) dan P-2 (TRESHOLD dari konfigurasi endpoint) sesuai keputusan produk Fitcher, serta mencatat keputusan keamanan Luca atas kredensial Dukcapil. Rev 3 menyelaraskan ADR dengan data contract v0.7.0 (SHAA-166): letak konfigurasi parameter output ditegaskan di provider, `NIK` ditegaskan bukan parameter output, `TRESHOLD` dirujuk lewat namespace tertutup `{{endpoint.*}}`, kolom `endpoint_config_version` dikonfirmasi, dan angka uji 3, 5, 6, 8, 9 dirujuk ke contract U-40…U-44. Rev 4 menambah D6: field input client yang tidak dikenal kontrak input endpoint ditolak `rejected`/`validasi_gagal` (422) sebelum cek kuota, bukan diabaikan (contract v0.7.0 §11 butir 29), dan mengganti kalimat basi di uji 9 dengan uji otomatis. Rev 5 menjalankan catatan mengikat Luca atas rev 4: limiter penolakan per client agar request `rejected` tidak lolos dari perlindungan beban (D6), koreksi rujukan D6 ke contract, dan klarifikasi kapan baris `rejected` ditulis. Rev 5 juga menambah mitigasi K-31 berupa `config_schema_version` pada dokumen konfigurasi provider dan endpoint (D3). Acuannya contract v0.7.4 final. Tidak ada keputusan K-* yang diubah. Format tanggal mengikuti K-30 (YYYY-MM-DD, mencabut K-23) dan tidak berdampak ke arsitektur.

- **Pemilik:** Shu
- **Tanggal:** 2026-11-03
- **Status:** **final rev 5**, difinalkan Shu sebagai lead Engineering (arahan owner 2026-11-03: Luca tidak ikut review project PoolRouter). Rev 4 di-approve Luca 2026-11-03. Kedua catatan mengikatnya sudah masuk rev 5 (D6, §7 uji 11–12). D1–D5 tidak berubah. Melengkapi ADR-0001 (bagian 2, 4.1, 5, dan 7; rujukan balik di ADR-0001 rev 6) tanpa mengubah keputusan yang sudah di-approve
- **Issue:** SHAA-165 (induk SHAA-162). Permintaan: komentar Fitcher di SHAA-162, 2026-11-03
- **Reviewer:** rev 1–4 Luca; rev 5 final oleh Shu sebagai lead (arahan owner 2026-11-03)
- **Lokasi akhir:** `docs/adr/0002-adapter-provider-dukcapil.md` di akar repo `github.com/faisalcayunda/PoolRouter` (ADR-0003 §5), di commit kerangka repo pertama. Sampai saat itu, lampiran di issue adalah salinan resmi.

## 1. Konteks

**Mengapa ADR terpisah.** ADR-0001 rev 4 dikunci Luca: isinya hanya koreksi wajib ditambah bagian 6 (hasil spike SHAA-177). Kalau temuan baru dimasukkan ke rev 4, aturan itu dilanggar dan review rev 4 tercampur dengan materi baru. ADR-0002 menambah keputusan pada batas yang sama, yaitu adapter HTTP provider di webserver dan worker (ADR-0001 §2) dan validator template (ADR-0001 §7). Setelah ADR-0002 di-approve, ADR-0001 diberi rujukan ke ADR ini di revisi berikutnya tanpa mengubah isi keputusannya.

Sumber: ringkasan dokumen Dukcapil yang sudah di-mask (lampiran komentar Fitcher di SHAA-162, 2026-11-03); PRD v0.4.6 final §6 butir 1, G-01, K-07, K-08, K-25, K-30, K-31; ADR-0001 rev 6 final §2, §4.1, §5, §7; data contract v0.7.4 final (SHAA-166): `status_akses`, `error_class`, R-01, R-04, §3.2.2, §3.2.2a L-01…L-08, §3.8 O-01…O-07, V-01…V-05, §11 butir 24–31.

| Fakta dari dokumen Dukcapil | Dampak |
|---|---|
| Method **GET dengan JSON body**, `Content-Type: application/json` | PRD §6 butir 1 mengasumsikan GET = path/query, body = POST. Model template harus diubah |
| Kredensial (`USER_ID`, `PASSWORD`, `IP_USER`) di body | Sesuai K-08: tipe auth No Auth, nilai dari `{{secret.*}}`. Redaksi contract R-04 sudah menanganinya |
| `transactionId` wajib, tidak dikembalikan di respons | Korelasi tidak boleh bergantung pada respons. Nilai = kode transaksi K-25 per percobaan |
| 18 field opsional dikirim sebagai string kosong | Template butuh nilai default per parameter |
| Respons: envelope pagination, hasil di `content[0]`, nilai per elemen `Sesuai`, `Tidak Sesuai`, opsional `(skor)` | Skema respons butuh ekstraksi path dan parser status+skor |
| Field yang dikirim kosong tetap dikembalikan `Tidak Sesuai` | Hasil bisa menyesatkan client. Diputuskan di P-1: elemen itu dihilangkan dari output |
| Base URL: IP privat, HTTPS, port 8000; sertifikat kemungkinan self-signed | Butuh jalur jaringan dan trust TLS per provider |
| Kode error, HTTP status, dan rate limit tidak dijelaskan | Pemetaan `error_class` tetap konfigurasi dan diisi saat PoC (ADR-0001 §7). Tidak diputuskan di sini |

## 2. D1 — Template request tidak terikat method

| Opsi | Kelebihan | Kekurangan | Biaya atau risiko |
|---|---|---|---|
| **A. Template = method + path + query + header + body opsional, untuk method apa pun** | Satu model untuk semua provider; Dukcapil cukup dikonfigurasi, tidak perlu kode khusus (G-01) | Admin bisa mengonfigurasi body pada GET untuk provider yang tidak membutuhkannya | Rendah |
| B. Adapter khusus Dukcapil di kode | Perilaku eksplisit | Melanggar G-01 (provider tanpa ubah kode); pola ini akan berulang untuk provider berikutnya | Sedang |
| C. Kirim sebagai POST atau pindahkan field ke query | Lazim di HTTP | Dukcapil menetapkan GET; NIK dan kredensial di query string akan tercatat di access log perantara | Tidak bisa dipakai |

**Keputusan: A.** Aturan pelaksanaannya:
- Body dikirim dengan `Content-Length`, bukan chunked transfer encoding. Sebagian server dan proxy menolak GET yang chunked.
- HTTP client upstream **tidak mengikuti redirect** dan **tidak me-retry sendiri**. Redirect yang diikuti akan mengirim ulang body berisi kredensial ke host lain. Retry tetap dikendalikan PoolRouter (ADR-0001 §5), sehingga setiap panggilan punya baris log (contract R-01).
- Setiap parameter template punya nilai default opsional. Untuk Dukcapil, default field opsional adalah `""`. Contoh dokumen yang mengirim `" "` untuk NO_RT/NO_RW tidak dijadikan aturan sampai PoC menunjukkan bahwa `""` ditolak.
- PRD §6 butir 1 ("path/query untuk GET, body untuk POST") perlu diselaraskan. Teks PRD milik Fitcher; ini bukan keputusan K-*.

## 3. D2 — `{{system.kode_transaksi}}` untuk `transactionId`

Validator template di ADR-0001 §7 hanya mengizinkan `{{input.*}}`, `{{secret.*}}`, dan keluaran fungsi middleware. ADR ini menambah dua namespace tertutup:
- `{{system.*}}`, yang di F0 hanya berisi `kode_transaksi`. Rujukan `{{system.X}}` lain ditolak 422, sama seperti variabel tak dikenal.
- `{{endpoint.*}}` (rev 3), untuk nilai literal template milik downstream endpoint. Di F0 hanya dipakai `TRESHOLD` Dukcapil. Aturannya ada di D3, butir TRESHOLD.

Urutan di worker dan webserver (sync) menjadi: buat UUIDv7 di memori (`Uuid::now_v7()`, K-25), render template dengan nilai itu, lalu satu transaksi CAS + insert log dengan `kode_transaksi` yang sama, dan baru kemudian panggilan upstream. Urutan contract R-01 tidak berubah karena render tidak menyentuh database. Titik antara render template dan panggilan upstream adalah posisi hook `pre_upstream`, yang menjadi titik ekstensi F2 dan tidak diimplementasikan di F0 (K-31).

Konsekuensi:
- Setiap percobaan mengirim `transactionId` berbeda, karena K-25 menetapkan kode per percobaan. Rujukan ke Dukcapil untuk satu percobaan tertentu selalu bisa ditelusuri dari `transaction_log`.
- Korelasi respons memakai pertukaran HTTP yang sama, bukan field di respons. Karena itu tidak adanya `transactionId` di respons tidak berdampak.
- `kode_transaksi` bukan secret, jadi muncul apa adanya di `request_payload` pada `full_payload`.

## 4. D3 — Skema respons: ekstraksi path dan parser `status_skor`

| Opsi | Kelebihan | Kekurangan | Biaya atau risiko |
|---|---|---|---|
| **A. Tipe parser bawaan yang dipilih lewat konfigurasi (`json`, `status_skor`)** | Aman; validasi konfigurasi sederhana; perilaku sama untuk semua provider sejenis | Format baru yang tidak tertangani butuh tipe parser baru (kode) | Rendah |
| B. Regex bebas di konfigurasi | Paling fleksibel | Validasi dan pengujian konfigurasi sulit; makna grup tangkapan bergantung admin | Sedang |
| C. Fungsi `post_upstream` WASM/gRPC | Fleksibel penuh | Runtime plugin baru ada di F2 (K-31, merevisi K-26) | Tidak tersedia di F0 |

**Keputusan: A.** Semua bagian di bawah adalah **konfigurasi provider** dan masuk `provider_config_version`, kecuali `TRESHOLD` (konfigurasi endpoint, lihat butir terakhir). Konfigurasi provider Dukcapil:
- **Ekstraksi:** `content` wajib berupa array dengan tepat satu elemen, dan hasil dibaca dari `content[0]`. Panjang 0 atau lebih dari 1 dianggap respons tidak valid. Arti `content` kosong (misalnya NIK tidak ditemukan) ditanyakan ke Dukcapil (P-3).
- **Parser `status_skor`** per elemen. Setelah spasi di tepi dipangkas, nilai harus cocok dengan pola `<literal>` atau `<literal> (<skor>)`. Literal dikonfigurasi per provider (Dukcapil: `Sesuai` → `sesuai`, `Tidak Sesuai` → `tidak_sesuai`). Skor berupa bilangan bulat 0–100 dan boleh tidak ada.
- **Bentuk output** per elemen: `{"status": "sesuai" | "tidak_sesuai", "skor": <0–100> | null}`. Nama elemen mengikuti parameter output yang dikonfigurasi.
- **Parameter output adalah konfigurasi provider (rev 3, contract §11 butir 25).** Daftar parameter output beserta atribut `parameter_input_sumber` dan `abaikan_jika_input_kosong` milik provider, bukan downstream endpoint. Perubahannya menaikkan `provider_config_version`. Alasannya:
  - PRD §6 butir 1 dan G-01 menempatkan parameter input, template, skema respons, dan parameter output di provider registry. Downstream endpoint hanya membawa kontrak input dari provider dan kontrak metadata.
  - `parameter_input_sumber` merujuk parameter input, yang juga milik provider. Validasinya (sumber wajib ada, kalau tidak 422) terjadi di satu konfigurasi saat provider disimpan, tanpa validasi silang antar-konfigurasi.
  - Daftar elemen yang di-parse fail-closed harus sama persis dengan daftar parameter output. Kalau keduanya dipisah di dua konfigurasi dengan versi berbeda, ada jendela saat keduanya tidak cocok.
  - Belum ada kebutuhan output yang berbeda antar-endpoint untuk provider yang sama. Kalau muncul, diusulkan lewat PRD.
  - Hand-off rev 2 ke Klaus (butir 5) menyebut konfigurasi endpoint. Itu tidak akurat dan dikoreksi di sini. Dampak data nol, karena kedua versi dicatat per baris (contract V-02).
- **`NIK` bukan parameter output (rev 3, contract §11 butir 27).** Parameter output provider Dukcapil adalah `NAMA_LGKP` dan 18 elemen opsional, total 19, masing-masing dengan `parameter_input_sumber` bernama sama. Kunci `NIK` di `content[0]` tidak dikonfigurasi, sehingga tidak di-parse dan tidak masuk output (contract O-04). Alasannya:
  - `NIK` adalah kunci pencarian, bukan elemen biodata yang dicocokkan. Client sudah memegang nilainya.
  - Arti `NIK: Tidak Sesuai` belum diketahui. Nilai itu bisa berarti NIK tidak ditemukan, yang merupakan hasil bisnis. Meneruskannya sebelum ada jawaban Dukcapil berarti memberi client sinyal yang belum bisa ditafsirkan. Pertanyaan ini masuk P-3(f), dengan syarat yang sama seperti P-3(b): kalau artinya NIK tidak ditemukan, ADR ini dan contract wajib direvisi sebelum PoC.
  - P-1 mengatur elemen yang **tidak** dikirim client. P-1 tidak mewajibkan setiap elemen yang dikirim muncul di output. Karena output yang terlihat client berubah, konfirmasi produk diminta di P-5.
  - Risiko yang diterima sampai P-3(f) dijawab: request dengan NIK yang tidak terdaftar mungkin menghasilkan `berhasil` dengan semua elemen `tidak_sesuai`. Hasil itu tidak salah secara isi, tetapi kurang informatif.
- **Fail-closed:** jika satu elemen yang diharapkan tidak bisa di-parse, atau envelope tidak valid, seluruh respons dianggap tidak valid. Tidak ada hasil parsial ke client. Log ditulis `status_akses = gagal_provider`, `error_class = respons_tidak_valid`. Request tidak di-retry karena respons yang sama akan gagal lagi. Kode `failure_reason`-nya mengikuti contract untuk error permanen (`provider_error_permanen`). Respons mentah tetap tersimpan sesuai `log_mode`, termasuk pemeriksaan gema secret contract R-04.
- **Respons valid** → `status_akses = berhasil`, apa pun hasil kesesuaiannya. Ini sudah sesuai definisi contract.
- **Status tidak dihitung ulang dari skor.** PoolRouter memakai literal status dari Dukcapil dan tidak membandingkan skor dengan TRESHOLD sendiri. Apakah Dukcapil sudah menerapkan TRESHOLD pada status ditanyakan di P-3.
- Field tambahan di `content[0]` yang tidak dikonfigurasi tidak masuk output, tetapi tetap ada di respons mentah pada `full_payload`.
- **Elemen yang tidak dikirim client dihilangkan dari output (P-1, keputusan produk Fitcher 2026-11-03).** Atribut `abaikan_jika_input_kosong` per parameter output (konfigurasi provider) bernilai default **aktif**, dan aktif untuk semua parameter output Dukcapil. "Tidak dikirim" berarti parameter input yang menjadi sumber elemen itu tidak ada di request client atau berupa string kosong setelah dipangkas. Kedua kasus sama-sama dikirim sebagai `""` ke Dukcapil, sehingga hasilnya tidak bermakna.
  - Urutan: parse dan validasi fail-closed dilakukan untuk **semua** elemen yang dikonfigurasi, baru kemudian elemen yang tidak dikirim dibuang dari output. Dengan begitu perubahan format di sisi Dukcapil tetap terdeteksi, walaupun muncul di elemen yang tidak diminta client. Biayanya, satu request bisa gagal `respons_tidak_valid` karena elemen yang tidak dikirim client. Biaya itu diterima karena kondisi ini berarti kontrak Dukcapil berubah dan memang perlu ditangani.
  - Respons mentah Dukcapil tetap tercatat utuh, termasuk elemen yang dibuang, bila `log_mode = full_payload` (K-02).
  - Output tidak pernah berisi elemen yang tidak dikirim client, juga tidak dalam bentuk `null` atau penanda lain. Client hanya menerima elemen yang ia kirim.
- **TRESHOLD dari konfigurasi endpoint (P-2, keputusan produk Fitcher 2026-11-03).** Nilai `TRESHOLD` adalah nilai literal di konfigurasi downstream endpoint. Nilai ini tidak bersumber dari `{{input.*}}`, sehingga client tidak bisa mengubahnya di F0. Tujuannya agar hasil validasi konsisten antar-aplikasi BGN. Perubahan nilai hanya oleh admin, diaudit, dan menaikkan `endpoint_config_version` (ADR-0001 §7, contract V-01). Kebutuhan TRESHOLD per client, jika muncul, diusulkan lewat PRD dan bukan diputuskan di implementasi.
  - **Mekanisme (rev 3, menutup celah rev 2).** Template milik provider, sedangkan nilainya milik endpoint. Rev 2 tidak menyebut cara template merujuk nilai itu. Template Dukcapil menulis `"TRESHOLD": "{{endpoint.TRESHOLD}}"`. Konfigurasi endpoint memuat peta `parameter_literal` berupa nama → string literal.
  - Nilai literal tidak dirender ulang. Teks `{{...}}` di dalam nilai diperlakukan sebagai teks biasa, sehingga literal tidak bisa dipakai untuk menyelundupkan `{{input.*}}` atau `{{secret.*}}`.
  - Validasi saat simpan (422): menyimpan endpoint yang terikat ke provider yang templatenya merujuk `{{endpoint.X}}` ditolak bila endpoint tidak punya `X`. Menyimpan template provider yang menambah rujukan `{{endpoint.X}}` ditolak bila ada endpoint terikat yang belum punya `X`. Dengan begitu render tidak pernah menemukan nilai yang hilang.
  - `parameter_literal` terpisah dari kontrak metadata. Metadata adalah data client per request dan tetap tidak pernah dikirim ke provider (`{{metadata.*}}` ditolak 422, ADR-0001 §7).
  - Opsi yang ditolak: literal di template provider. Opsi itu lebih sederhana, tetapi membalik teks P-2 yang sudah di-approve dan mengubah contract U-44. Biaya `{{endpoint.*}}` hanya satu namespace tertutup dan satu aturan validasi.
- **Versi konfigurasi (rev 3, contract §11 butir 24).** Nama kolom `endpoint_config_version` dikonfirmasi, dengan pola yang sama seperti `provider_config_version`: `request` mencatat versi yang aktif saat request diterima, `transaction_log` mencatat versi yang dipakai saat render percobaan itu. Isi `endpoint_config_version` adalah kontrak metadata, `parameter_literal` (termasuk `TRESHOLD`), dan `log_mode` endpoint. Parameter output tidak termasuk, karena masuk `provider_config_version`.
- **Versi skema konfigurasi (rev 5, mitigasi K-31; contract v0.7.4 V-05, diterima Shu).** Risiko K-31: konfigurasi provider F0 mungkin perlu field baru saat hook F2 datang. Mitigasinya:
  - **Apa.** Dokumen konfigurasi provider **dan** dokumen konfigurasi endpoint sama-sama membawa `config_schema_version` (INTEGER, wajib). Nilai ini menyatakan bentuk dokumen dan terpisah dari `provider_config_version`/`endpoint_config_version`, yang menyatakan isi.
  - **Kenapa keduanya.** Fungsi middleware ada di konfigurasi endpoint (contract V-01), sedangkan titik `pre_upstream` ada di pipeline provider (D2). Letak field hook F2 **tidak** diputuskan di rev 5.
  - **F0.** Satu-satunya nilai yang diterima adalah `1`. Skema 1 tidak memuat field hook, middleware, atau plugin dalam bentuk apa pun, termasuk field kosong.
  - **Validasi admin API.** Dokumen tanpa versi skema, dengan versi yang tidak didukung, atau dengan field tak dikenal untuk versi skema yang dinyatakan → 422, tanpa versi baru. Percobaannya dicatat di audit konfigurasi.
  - **Kompatibilitas.** Layanan yang mendukung skema N menerima skema 1…N tanpa migrasi. Menaikkan versi skema adalah perubahan isi, sehingga versi isi ikut naik (contract V-01, V-03).
  - **Pencatatan.** Tidak ada kolom baru di `request` maupun `transaction_log` (contract V-02).
  - **Uji.** Contract v0.7.4 U-46a…f dan DQ-33. Angkanya tidak ditulis ulang di sini.
  - Ditinjau ulang bila jawaban P-3 memerlukan transformasi input sebelum F2.

  Contoh bentuk (hanya field versi; field lain mengikuti D1–D5 dan contract §3.8, nilai ilustratif):

  ```yaml
  # dokumen konfigurasi provider
  config_schema_version: 1        # bentuk dokumen (contract V-05)
  provider_config_version: 7      # isi; naik di setiap perubahan (contract V-01)
  base_url: ${DUKCAPIL_BASE_URL}
  tls_trust: { mode: ca_kustom }  # D5
  # tidak ada field pre_upstream / middleware / plugin di skema 1
  ```

  ```yaml
  # dokumen konfigurasi endpoint
  config_schema_version: 1
  endpoint_config_version: 3
  parameter_literal: { TRESHOLD: "80" }   # D3, P-2
  # tidak ada field middleware di skema 1
  ```

## 5. D4 — Akses jaringan ke provider privat

**Keputusan:**
- **Hanya webserver (jalur sync) dan worker yang boleh membuka koneksi ke provider.** Controller dan function-runner tidak punya egress ke provider. Aturan ini ditegakkan di lapisan jaringan (Kubernetes NetworkPolicy atau firewall egress di lingkungan PoC/produksi), bukan hanya di kode.
- **Egress lewat IP tetap** yang didaftarkan ke Dukcapil, yaitu nilai `IP_USER` (T-6 ADR-0001). Allowlist tujuan berupa pasangan host:port per provider.
- **Base URL adalah konfigurasi provider** di PostgreSQL. Perubahannya hanya oleh admin dan diaudit (ADR-0001 §7). Base URL tidak ditulis di repo, issue, atau dokumen; dokumentasi memakai `${DUKCAPIL_BASE_URL}`. Admin API hanya menerima skema `https`.
- **Dev dan CI** memakai mock Dukcapil di Compose yang meniru GET+body, envelope `content[0]`, string status+skor, dan sertifikat self-signed. Dengan begitu D1–D5 teruji end-to-end tanpa akses ke jaringan Dukcapil.
- **Pemeriksaan PoC (Platform & SRE):** body GET benar-benar sampai melewati jalur egress yang sebenarnya. Proxy atau firewall L7 di jalur itu bisa membuang body pada GET. Gejala yang perlu dicari adalah Dukcapil membalas seolah semua field kosong.
- Jalur jaringan ke IP privat Dukcapil (VPN, site-to-site, atau link yang sudah dimiliki BGN) adalah prasyarat PoC dan di luar kendali Shaal (P-4). Tanpa jalur ini PoC tertahan walaupun F0 sudah siap (dicatat Fitcher sebagai prasyarat gerbang PoC).
- **Kredensial Dukcapil (keputusan keamanan Luca, 2026-11-03).** Kredensial asli yang tercantum di dokumen yang beredar lewat chat **dianggap sudah terpapar**.
  - Kredensial lama tidak dipakai di lingkungan mana pun, baik PoC, uji, maupun konfigurasi.
  - Rotasi diminta ke Dukcapil lewat jalur BGN sebelum PoC, bersama paket P-3.
  - Kredensial baru tidak dikirim lewat chat, issue, atau dokumen. Kredensial itu hanya masuk ke secret store lingkungan PoC sebagai `${DUKCAPIL_USER_ID}` dan `${DUKCAPIL_PASSWORD}`, dan diisi owner.
  - Sampai rotasi selesai, semua pengembangan dan pengujian memakai mock Dukcapil dan kredensial sintetis.

## 6. D5 — Trust TLS per provider

| Opsi | Kelebihan | Kekurangan | Biaya atau risiko |
|---|---|---|---|
| A. Matikan verifikasi sertifikat | Langsung jalan | Kredensial dan NIK bisa dikirim ke server palsu di jaringan yang sama | **Ditolak.** Tidak disediakan sama sekali, termasuk di dev |
| **B. CA kustom per provider (`ca_kustom`)** | Verifikasi rantai dan nama (termasuk IP SAN) tetap penuh | Butuh sertifikat dengan SAN yang benar dan CA atau sertifikatnya diserahkan Dukcapil | Rendah |
| **C. Pin SPKI per provider (`pin_spki`)** | Bisa dipakai untuk sertifikat self-signed tanpa IP SAN | Rotasi kunci di sisi Dukcapil memutus layanan sampai pin diperbarui | Sedang, dimitigasi dua pin dan alert |
| D. Trust store sistem saja (`system`) | Tanpa konfigurasi | Gagal untuk sertifikat self-signed atau CA privat | Hanya untuk provider bersertifikat publik |

**Keputusan: konfigurasi `tls_trust` per provider dengan tiga mode, `system` (default), `ca_kustom`, dan `pin_spki`.** Tidak ada mode yang mematikan verifikasi.
- Pilihan untuk Dukcapil: `ca_kustom` jika sertifikatnya memuat IP SAN yang benar, `pin_spki` jika tidak. Keputusan akhir dibuat saat PoC berdasarkan sertifikat yang sebenarnya (P-3).
- `pin_spki` menerima satu atau lebih hash SHA-256 SPKI. Dua pin dianjurkan (kunci aktif dan kunci cadangan) supaya rotasi tidak memutus layanan. Pada mode ini kepercayaan berpijak pada kunci, jadi masa berlaku sertifikat tidak memblokir. Sebagai gantinya ada alert 30 hari sebelum `notAfter`. Sertifikat Dukcapil yang kedaluwarsa tetapi tidak memblokir layanan adalah **risiko yang diterima secara sadar** (catatan mengikat Luca), bukan celah. Runbook Platform & SRE wajib menetapkan pemilik alert 30 hari dan prosedur penggantian pin.
- PEM CA dan pin bukan secret, tetapi sensitif terhadap keamanan: hanya admin yang boleh mengubahnya, perubahan diaudit, dan konfigurasi tersimpan sebagai bagian dari versi konfigurasi provider (`provider_config_version`).
- TLS minimal 1.2 (rustls). Setiap provider punya HTTP client dan connection pool sendiri yang dibangun dari konfigurasinya, dan dibangun ulang saat versi konfigurasi berubah. Trust satu provider tidak pernah berlaku untuk provider lain.
- Kegagalan verifikasi sertifikat → `status_akses = gagal_jaringan`, `error_class = tls`, tidak di-retry, dan memicu alert. Verifikasi terjadi saat handshake, sebelum body dikirim, sehingga kredensial tidak pernah terkirim ke server yang gagal verifikasi. Gangguan handshake sementara (koneksi putus) tetap `koneksi` dan mengikuti retry biasa.
- mTLS (sertifikat client) tidak diminta dokumen Dukcapil dan tidak dibangun di F0. Konfigurasi `tls_trust` dirancang agar bisa ditambah tanpa mengubah model.

## 6a. D6 — Field input client yang tidak dikenal (rev 4, contract §11 butir 29)

**Konteks.** Contract v0.7.0 U-44a (client menyertakan `TRESHOLD`) berlaku untuk dua kemungkinan, ditolak atau diabaikan, tetapi pilihannya menentukan status `rejected`, pemakaian kuota, dan isi `input_payload`. ADR-0001 §4.1 dan contract §3.2.2 belum menetapkannya.

**Kontrak input endpoint** = parameter input provider yang bersumber dari `{{input.*}}` di template provider. Parameter yang diisi dari `{{endpoint.*}}` (mis. `TRESHOLD`), `{{secret.*}}` (`USER_ID`, `PASSWORD`, `IP_USER`), dan `{{system.*}}` (`transactionId`) **bukan** bagian dari kontrak input client.

| Opsi | Kelebihan | Kekurangan | Biaya atau risiko |
|---|---|---|---|
| **A. Tolak: `rejected` / `validasi_gagal`, HTTP 422, sebelum cek kuota** | Kesalahan client terlihat seketika; `input_payload` hanya berisi field yang dikenal; konsisten dengan contract M-R3 (metadata di luar kontrak ditolak) | Client yang mengirim field berlebih harus diperbaiki | Rendah |
| B. Abaikan diam-diam, simpan apa adanya | Paling longgar bagi client | Menyimpan request apa adanya bertentangan dengan definisi `input_payload`, yaitu parameter input sesuai kontrak input provider (contract v0.7.4 §3.1 baris `input_payload` dan §3.4; nomornya sama sejak v0.6.1). Field tak dikenal yang bisa memuat data pribadi ikut tersimpan tanpa tujuan. Salah ketik nama elemen (mis. `NAMA_LGKAP`) membuat elemen itu dianggap tidak dikirim, sehingga dibuang dari output oleh P-1 tanpa error | Tinggi: hasil salah yang tidak terlihat |
| C. Abaikan dan buang sebelum disimpan | Tidak menyimpan data berlebih | Masalah salah ketik di B tetap ada | Sedang |

**Keputusan: A.**
- Nama field dicocokkan persis (peka huruf besar-kecil, tanpa normalisasi) terhadap kontrak input endpoint pada `provider_config_version` yang aktif saat request diterima.
- Field yang tidak dikenal → `rejected` dengan `reject_reason = validasi_gagal`, HTTP 422, sebelum cek kuota dan rate limit upstream pool. Tidak ada token kuota atau token rate pool yang terpakai, dan tidak ada baris `transaction_log` karena provider tidak dipanggil. `input_payload`, field metadata Rahasia, dan NIK tidak disimpan (contract v0.7.4 §3.4 P-01). Penolakan ini mengambil 1 token limiter penolakan client (lihat di bawah). Berlaku sama untuk sync dan async.
- Body error memuat daftar **nama** field yang ditolak, tanpa nilainya. Nama yang tidak cocok pola `^[A-Za-z0-9_]{1,64}$` tidak digemakan dan diganti jumlahnya saja, supaya body error tidak menjadi jalur pantulan data.
- JSON dengan kunci ganda di objek input atau metadata, dan body yang bukan JSON valid, ditolak HTTP 400 dengan `reject_reason = validasi_gagal` (rev 5; turunan Klaus yang ditetapkan Shu, contract v0.7.4 §3.2.2). Tidak ada kode `reject_reason` baru. Parser yang berbeda memilih kunci pertama atau terakhir, dan perbedaan itu bisa dipakai untuk menyelundupkan nilai yang lolos validasi. Selama token limiter ada, penolakan ini dicatat sebagai baris `rejected` tanpa `input_payload`, metadata Rahasia, dan NIK (contract v0.7.4 §3.4 P-01). Untuk body yang tidak bisa di-parse, `metadata` diisi `{}`, dan kolom wajib yang nilainya hanya ada di body boleh NULL **khusus** pada `validasi_gagal` dengan `reject_http_status = 400`. Daftar kolomnya ditetapkan di desain skema (contract v0.7.4 §3.2.2).
- Akibat untuk contract U-44a: 10 request menghasilkan 10 baris `rejected`/`validasi_gagal` (kasus (a) di bawah), 0 panggilan ke stub, dan 0 pemakaian kuota. Ukuran "panggilan ke stub dengan `TRESHOLD` selain `"80"` = 0" tetap berlaku.
- Field yang dikenal tetapi kosong tidak termasuk aturan ini. Perlakuannya tetap P-1 (dikirim `""`, elemen dibuang dari output).
- Penambahan parameter input di konfigurasi provider memperluas kontrak input; client baru boleh mengirim field itu setelah versi konfigurasi aktif. Penghapusan parameter input memutus client dan diperlakukan seperti contract M-R4 (diumumkan ke client owner sebelum aktif).

**Apa yang tercatat untuk penolakan (rev 5, klarifikasi atas pertanyaan Fitcher).** Ada dua kasus, dan keduanya tidak pernah membuat baris `transaction_log`, karena provider tidak dipanggil:
- **(a) Ditolak selama token limiter penolakan masih ada** → satu baris `request` berstatus `rejected` dengan `reject_reason`, **tanpa** `input_payload`, field metadata Rahasia, dan NIK (contract v0.7.4 §3.4 P-01). Contract U-44a (10 request) termasuk kasus ini dan menghasilkan 10 baris `rejected`.
- **(b) Token limiter penolakan habis** → 429 `penolakan_berlebih` + `Retry-After`, **tanpa baris `request` sama sekali**.

### Limiter penolakan per client (rev 5, catatan mengikat 1 Luca atas rev 4)

Request `rejected` tidak boleh lolos dari perlindungan beban. Bucket rate limit ADR-0001 §5 bekerja per upstream pool dan hanya untuk request yang diterima, sehingga client yang rusak bisa memicu penolakan tanpa batas: setiap penolakan menulis satu baris dan memakan CPU.

| Opsi | Kelebihan | Kekurangan | Keputusan |
|---|---|---|---|
| (a) Penolakan ikut memakai bucket GCRA upstream pool | Tanpa komponen baru | Bucket pool dipakai bersama semua client. Satu client rusak memakan kapasitas client lain, termasuk request yang sah | Ditolak |
| **(b) Limiter penolakan terpisah per client** | Isolasi per client; request yang diterima tidak tersentuh | Satu skrip Lua dan satu kunci Redis per client | **Dipilih** (Shu, SHAA-162, balasan atas komentar Luca 01a19f35) |

Mekanisme (normatif di contract v0.7.4 §3.2.2a; teks ini merujuk, bukan mendefinisikan ulang):
- **Bucket.** Bucket GCRA Redis per `client_id` dengan kunci terpisah dari bucket pool, dijalankan lewat skrip Lua atomik (L-01). Bucket hanya diperiksa di jalur penolakan submit, setelah autentikasi dan setelah hasil penolakan diketahui.
- **Yang mengambil 1 token (L-02).** Setiap request yang hasilnya `rejected`, apa pun `reject_reason`-nya, termasuk `kuota_habis`, `porsi_sync_habis`, `objek_nonaktif`, dan 400 untuk JSON tidak valid atau kunci ganda. Juga dua penolakan submit yang tidak membuat baris: header `Idempotency-Key` pada sync (400) dan `Idempotency-Key` sama dengan sidik berbeda (422). Keduanya tetap tanpa baris walau token ada (contract v0.7.4 U-45h).
- **Yang tidak mengambil token.** Request yang diterima, `Idempotency-Key` berulang dengan sidik sama (200), polling dan pembatalan (404, 409), gagal autentikasi, dan 503 *fail-closed*. Request yang diterima tetap berjalan walaupun bucket penolakan client itu kosong, dan tidak ada token kuota yang terpakai oleh limiter.
- **Angka (L-03, L-04).** Burst 20 dan isi ulang 1 token/detik per client. Keduanya bisa diubah platform admin per client dengan alasan wajib, diaudit, dan langsung berlaku. Tidak ada nilai yang mematikan limiter.
- **Token habis (L-05).** 429 + `Retry-After`. Body hanya berisi kode `penolakan_berlebih`, yang **bukan** `reject_reason`. Body tidak memuat alasan asal, nama field, nilai, atau `queue_id`. Tidak ada baris `request`, `request_status_history`, maupun `transaction_log`. Jejaknya hanya metrik dan access log yang membawa `request_id` intake (contract v0.7.4 DQ-32 (d)). Contract mencantumkan kondisi ini di tabel "penolakan yang tidak disimpan" §3.2.2, bersama gagal autentikasi dan 503 *fail-closed*.
- **Volume (turunan Klaus, ditetapkan Shu).** Batas baris `rejected` per client = burst + isi ulang, yaitu paling banyak sekitar 86.420 baris/hari, tanpa payload. Klaus menerima angka ini dan memantaunya lewat contract v0.7.4 DQ-32 (e).
- **Redis tidak tersedia (L-07).** 503 *fail-closed* tanpa baris. Request tidak boleh menjadi baris `rejected` tanpa token.
- **Metrik (L-06).** `poolrouter_penolakan_total{client_id, downstream_endpoint_id, reject_reason, http_status}`, `poolrouter_penolakan_dibatasi_total{client_id, downstream_endpoint_id}`, `poolrouter_penolakan_tanpa_baris_total{client_id, downstream_endpoint_id, http_status}`, dan gauge `poolrouter_limiter_penolakan_token{client_id}`. Nama lama `rejected_throttled_total` tidak dipakai. Label tidak boleh memuat NIK, nama field, IP, atau `request_id`. Alert Platform & SRE dipicu bila satu client terus-menerus ter-throttle, berdasarkan `poolrouter_penolakan_dibatasi_total`.
- **Dampak analisis (L-08).** Jumlah baris `rejected` adalah batas bawah penolakan. Jumlah sebenarnya didapat dari baris ditambah dua counter tanpa baris.
- **Di luar cakupan.** Flood tanpa autentikasi ditangani access log dan ingress.

## 7. Uji wajib

Semua uji memakai data sintetis dan mock Dukcapil. Sejak rev 3, angka harapan uji 3, 5, 6, 8, dan 9 ditetapkan di data contract v0.7.0 §6.2 sebagai U-40…U-44 (contract §11 butir 28). Deskripsi di bawah hanya menjelaskan maksud uji, dan bila ada selisih, contract yang berlaku. Uji 1, 2, 4, 7, dan 10 tetap di ADR karena menguji perilaku jaringan, validator, dan isolasi, bukan data. Uji 12 dan 13 (rev 5) hanya merujuk contract v0.7.4 U-45 dan U-46.
1. **GET+body:** mock menerima GET dengan body JSON utuh dan `Content-Length`. 0 request chunked.
2. **Redirect:** mock membalas 302 ke host lain → 0 koneksi ke host tujuan redirect, dan percobaan dicatat gagal.
3. **Kode transaksi (contract U-40):** `transactionId` di body yang diterima mock = `kode_transaksi` baris log untuk percobaan itu. Request dengan 2 percobaan → 2 nilai berbeda, keduanya UUIDv7.
4. **Validator:** template dengan `{{system.lain}}` atau `{{metadata.x}}` ditolak 422. Rev 3: endpoint tanpa `parameter_literal.TRESHOLD` yang terikat ke provider Dukcapil ditolak 422. Template provider yang menambah `{{endpoint.Y}}` ditolak 422 selama ada endpoint terikat tanpa `Y`. Nilai literal `"{{secret.DUKCAPIL_PASSWORD}}"` dikirim sebagai teks apa adanya, bukan nilai secret.
5. **Parser (uji tabel, contract U-41):** `Sesuai`, `Tidak Sesuai`, `Sesuai (100)`, `Tidak Sesuai (0)`, dan spasi di tepi → hasil benar. `Sesuai (101)`, `Cocok`, `""`, `content` kosong, dan `content` dua elemen → `respons_tidak_valid`, 0 output parsial, 0 retry.
6. **TLS (contract U-42):** mock bersertifikat self-signed. Mode `system` → gagal `tls`. `ca_kustom` dengan CA yang benar → berhasil. `pin_spki` dengan pin salah → gagal `tls`, dan server palsu menerima 0 byte data aplikasi (body berisi kredensial sintetis tidak pernah terkirim).
7. **Isolasi trust:** CA kustom provider A tidak membuat panggilan ke provider B dengan sertifikat dari CA itu berhasil.
8. **Elemen tidak dikirim (P-1, contract U-43):** client mengirim NIK dan `NAMA_LGKP` saja, mock membalas semua elemen → output hanya berisi elemen `NAMA_LGKP`. Pada `full_payload`, respons mentah di log memuat semua elemen. Input string kosong atau berisi spasi diperlakukan sama dengan tidak dikirim. Elemen tidak dikirim yang bernilai tak dikenal (`Cocok`) tetap menghasilkan `respons_tidak_valid`.
9. **TRESHOLD (P-2, contract U-44):** request client yang menyertakan field `TRESHOLD` ditolak `validasi_gagal` sebelum panggilan (D6), sehingga tidak ada nilai `TRESHOLD` dari client yang sampai ke mock. Rev 4, menggantikan pemeriksaan manual: uji otomatis menyimpan endpoint dengan `parameter_literal.TRESHOLD = "{{input.TRESHOLD}}"`, lalu memastikan mock menerima teks `{{input.TRESHOLD}}` apa adanya, karena literal tidak dirender.
11. **Field tidak dikenal (D6):** request dengan field `NAMA_LGKAP` (salah ketik), `TRESHOLD`, `PASSWORD`, atau `transactionId` → 422 `validasi_gagal` dengan baris `rejected`, 0 panggilan ke mock, counter kuota tidak berubah, `input_payload` NULL. Body error memuat nama field, bukan nilainya. Objek input dengan kunci `NIK` ganda, atau body yang bukan JSON valid → 400 `validasi_gagal` dengan baris `rejected` (rev 5; angka di contract v0.7.4 U-45e).
12. **Limiter penolakan (rev 5, catatan mengikat 1 Luca; = contract v0.7.4 U-45a).** Satu client mengirim 21 request 422 berturut-turut dalam < 1 detik. Hasilnya 20 × 422 dengan 20 baris `rejected`, ditambah 1 × 429 `penolakan_berlebih` dengan `Retry-After` dan tanpa baris. Pemakaian kuota 0. Request valid dari client itu tepat sesudahnya tetap diterima (U-45b). Penolakan dari client lain pada saat yang sama tetap 422 (isolasi, U-45c). Uji turunan hanya dirujuk ke contract v0.7.4: U-45d (setelah `Retry-After`), U-45e (kunci ganda dan JSON tidak valid), U-45f (ubah burst per client + audit), U-45g (Redis mati → 503), dan U-45h (`Idempotency-Key` pada sync). Angka harapan hanya ada di contract.
13. **Versi skema konfigurasi (rev 5, D3):** dirujuk ke contract v0.7.4 U-46a…f dan DQ-33.
10. **Egress (PoC, Platform & SRE):** controller tidak bisa membuka koneksi ke host:port Dukcapil. Webserver dan worker bisa. Body GET sampai lewat jalur egress PoC.

## 8. Dampak ke dokumen lain

| Dokumen | Perubahan | Pemilik |
|---|---|---|
| ADR-0001 | Rujukan ke ADR-0002 di §2 (egress) dan §7 (namespace `{{system.*}}` dan `{{endpoint.*}}`), serta penandaan `pre_upstream` sebagai titik ekstensi F2 (K-31), pada revisi setelah rev 4. Isi keputusan rev 4 tidak berubah | Shu |
| Data contract | `request_payload` memuat `transactionId`; bentuk `output_payload` per elemen `{status, skor}`, hanya untuk elemen yang dikirim client (P-1); `respons_tidak_valid` dan `tls` tidak di-retry; konfigurasi `tls_trust` masuk versi konfigurasi provider; `TRESHOLD` masuk versi konfigurasi endpoint (P-2) | Klaus (ADR sudah approve) |
| PRD §6 butir 1 | Template: body opsional untuk method apa pun | Fitcher |
| PRD v0.4.3 / v0.4.4 | P-1, P-2, §6 butir 1 (sudah); K-30, K-31 (sudah) | Fitcher |
| Data contract v0.7.0 (rev 3) | §11 butir 25: `parameter_input_sumber` dan `abaikan_jika_input_kosong` pindah dari isi `endpoint_config_version` ke `provider_config_version` (V-01, §3.1 `output_payload` dan `endpoint_config_version`, O-01, O-02). Butir 24, 27, dan 28 dikonfirmasi tanpa perubahan. Opsional: V-01 menyebut `TRESHOLD` dirujuk lewat `{{endpoint.TRESHOLD}}` | Klaus |
| Data contract v0.7.x (rev 4) | §11 butir 29 dijawab D6: field tidak dikenal → `rejected`/`validasi_gagal` sebelum kuota. Opsional: U-44a mencatat 10 `rejected` dan 0 pemakaian kuota. Tidak ada kolom atau kode baru | Klaus |
| BRD / user story (SHAA-164) | Kontrak output kesesuaian per elemen; elemen yang tidak dikirim dihilangkan (P-1); TRESHOLD tidak bisa diubah client (P-2); `NIK` tidak ada di output (P-5). Rev 4: AC US-03 bahwa field input di luar kontrak ditolak 422 `validasi_gagal` sebelum memakai kuota (D6) | Fitcher |
| Runbook Platform & SRE | Pemilik alert 30 hari `notAfter` dan prosedur penggantian pin `pin_spki`; verifikasi GET+body di jalur egress PoC. Rev 5: ambang dan routing alert `poolrouter_penolakan_dibatasi_total` | Platform & SRE, saat PoC |
| ADR-0001 rev 6 (rev 5) | Rujukan ADR-0002 di §2, §4.1, §5, §7 dan penyelarasan K-31. **Selesai** | Shu |
| Data contract v0.7.4 (rev 5) | Limiter penolakan §3.2.2a (L-01…L-08), kunci ganda/JSON tidak valid sebagai `validasi_gagal` 400, `reject_http_status`, V-05 `config_schema_version`, U-45, U-46, DQ-32, DQ-33. **Sudah final**; ADR ini selaras dengannya | Klaus |

## 9. Konsekuensi

- **Lebih mudah:** provider dengan pola Dukcapil cukup dikonfigurasi; server palsu tidak bisa menerima kredensial; hasil kesesuaian punya bentuk yang bisa diproses mesin.
- **Lebih sulit atau perlu dikerjakan:** limiter penolakan per client menambah satu skrip Lua dan satu kunci Redis per client, dan jumlah baris `rejected` hanya menjadi batas bawah penolakan (contract v0.7.4 L-08); verifier pin SPKI di rustls ditulis sendiri dan wajib diuji (uji 6, 7); rotasi sertifikat Dukcapil pada mode `pin_spki` menjadi prosedur operasional; jalur egress PoC harus diverifikasi untuk GET+body sebelum uji fungsional.
- **K-*:** tidak ada yang diubah. K-08 (kredensial di body sebagai variabel secret) dan K-25 (kode transaksi per percobaan) dijalankan apa adanya. K-30 (YYYY-MM-DD, mencabut K-23) dan K-31 (hook `pre_upstream` ke F2) tidak berdampak ke keputusan ADR ini: `TGL_LHR` diteruskan apa adanya, dan F0 tidak memuat fungsi middleware.

## 10. Pertanyaan terbuka

| ID | Pertanyaan | Untuk | Default bila belum dijawab |
|---|---|---|---|
| P-1 | ~~Elemen yang tidak dikirim client: dihilangkan dari output, atau dikembalikan apa adanya?~~ | Fitcher | **Ditutup 2026-11-03 (keputusan produk Fitcher):** dihilangkan dari output; respons mentah tetap utuh di log `full_payload`. Lihat D3 |
| P-2 | ~~Sumber nilai TRESHOLD: per downstream endpoint atau input client?~~ | Fitcher | **Ditutup 2026-11-03 (keputusan produk Fitcher):** konfigurasi endpoint; client tidak bisa mengubahnya di F0. Lihat D3 |
| P-3 | Ke Dukcapil: (a) apakah status per elemen sudah memperhitungkan TRESHOLD; (b) arti `content` kosong; (c) kode error, HTTP status, dan rate limit; (d) sertifikat TLS: CA atau self-signed, dan apakah memuat IP SAN; (e) apakah `transactionId` dipakai Dukcapil sebagai rujukan pengaduan; (f) (rev 3) arti nilai kunci `NIK` di `content[0]`, khususnya `NIK: Tidak Sesuai`. Ditambah permintaan rotasi kredensial (D4) | Dukcapil, lewat Luca → owner → BGN | (a) status Dukcapil dipakai apa adanya; (b) `respons_tidak_valid`, **dengan syarat mengikat Luca:** jika `content` kosong berarti "NIK tidak ditemukan", itu hasil bisnis, bukan kegagalan provider, dan ADR ini wajib direvisi sebelum PoC; (c) kode tak dikenal → `lainnya`; (d) diputuskan saat PoC; (f) `NIK` tidak di-parse dan tidak masuk output; syarat sama dengan (b) bila artinya NIK tidak ditemukan. **Putusan Luca saat approve rev 3:** jawaban (b) dan (f) adalah syarat gerbang PoC. Tanpa jawaban Dukcapil, PoC tidak dinyatakan lulus. Risiko NIK tidak terdaftar yang menghasilkan `berhasil` dengan semua elemen `tidak_sesuai` diterima hanya sampai gerbang PoC |
| P-5 | (rev 3) Apakah `NIK` boleh tidak muncul di output ke client? Client mengirim `NIK`, tetapi output hanya memuat elemen biodata yang dicocokkan | Fitcher | `NIK` tidak ada di output (D3). Kalau produk ingin `NIK` muncul, keputusan itu dibuat setelah P-3(f) dijawab, dan contract U-41 serta U-43 direvisi |
| P-4 | Jalur jaringan dari lingkungan PoC BGN ke IP privat Dukcapil, dan IP egress tetap (memperluas T-6 ADR-0001) | BGN, lewat Luca → owner; Platform & SRE | PoC tidak bisa memanggil Dukcapil |

## Rujukan

- Ringkasan "Dokumentasi API: Dukcapil Verify By Element" (masked), lampiran komentar Fitcher di SHAA-162, 2026-11-03. Dokumen asli tidak dilampirkan karena memuat kredensial dan data pribadi.
- ADR-0001 rev 6 final (SHAA-165), §2, §4.1, §5, §7. ADR-0004 rev 3 (crate Rust) dan ADR-0003 rev 3 (Web UI, monorepo).
- PRD PoolRouter v0.4.6 final, §6 butir 1, G-01, K-07, K-08, K-25, K-30, K-31.
- Data contract v0.7.4 final (SHAA-166): `status_akses`, `error_class`, R-01, R-04, R-06…R-08, §3.1, §3.2.2, §3.2.2a L-01…L-08, §3.4 P-01, §3.8 V-01…V-05, §6.2 U-40…U-46, DQ-32, DQ-33, §11 butir 24–31.

## Riwayat revisi

| Rev | Tanggal | Perubahan |
|---|---|---|
| 1 | 2026-11-03 | Draf pertama, diajukan ke Luca. Atas permintaan Fitcher (SHAA-162): butir 2–5 temuan dokumen Dukcapil. Di-approve Luca 2026-11-03 |
| 2 | 2026-11-03 | Status approved. P-1 dan P-2 ditutup dengan keputusan produk Fitcher (D3, uji 8–9). Keputusan keamanan Luca atas kredensial Dukcapil dicatat (D4). Catatan mengikat Luca dicatat: P-3(b) dan risiko `pin_spki` beserta runbook (D5). Jalur P-3/P-4 diperbarui: lewat Luca ke owner. D1–D5 tidak berubah |
| 3 | 2026-11-03 | Di-approve Luca 2026-11-03 dan menjadi acuan. Putusan approve: P-3(b) dan P-3(f) menjadi syarat gerbang PoC (§10). Koreksi kalimat kedua uji 9 menjadi uji otomatis dibawa ke revisi berikutnya, bersama rujukan ADR-0002 di ADR-0001. Penyelarasan dengan data contract v0.7.0 §11: butir 24 `endpoint_config_version` dikonfirmasi; butir 25 parameter output beserta `parameter_input_sumber`/`abaikan_jika_input_kosong` ditetapkan sebagai konfigurasi provider (hand-off rev 2 butir 5 dikoreksi); butir 27 `NIK` bukan parameter output, ditambah P-3(f) dan P-5; butir 28 uji 3, 5, 6, 8, 9 dirujuk ke U-40…U-44. Celah rev 2 ditutup: namespace tertutup `{{endpoint.*}}` dan `parameter_literal` untuk `TRESHOLD` (D2, D3, uji 4). Rujukan K-23 diganti K-30, dan `pre_upstream` ditandai sebagai titik ekstensi F2 (K-31). D1, D4, D5 tidak berubah |
| 4 | 2026-11-03 | Di-approve Luca 2026-11-03 dan menjadi acuan. Catatan mengikat untuk rev 5: (1) request `rejected` tidak boleh lolos dari perlindungan beban; tetapkan perlakuannya dan tambah uji "N request 422 berturut-turut dari satu client → 429"; (2) opsi B D6 salah merujuk K-30 (format tanggal) dan "(P-01)" tanpa nama dokumen; ganti dengan contract v0.6.1 §3.1 baris `input_payload` dan §3.4 P-01. D6 baru: field input client di luar kontrak input endpoint ditolak `rejected`/`validasi_gagal` (422) sebelum cek kuota, termasuk field yang diisi `{{endpoint.*}}`/`{{secret.*}}`/`{{system.*}}`; kunci JSON ganda ditolak 400 (contract v0.7.0 §11 butir 29). Uji 9 diganti uji otomatis literal tidak dirender (koreksi approve rev 3); uji 11 baru. D1–D5 tidak berubah |
| 5 | 2026-11-03 | Final rev 5 (Shu sebagai lead, arahan owner 2026-11-03). Catatan mengikat Luca atas rev 4: (1) limiter penolakan per client (opsi b; contract v0.7.4 §3.2.2a L-01…L-08), dengan uji 12 = contract U-45a dan uji turunan U-45b…h dirujuk; (2) rujukan D6 opsi B diganti ke contract §3.1 baris `input_payload` dan §3.4, dan "(P-01)" diganti contract §3.4 P-01; rujukan R-xx, U-xx, M-Rx tanpa nama dokumen diberi nama. Klarifikasi D6 dua kasus (baris `rejected` tanpa payload saat token ada; 429 tanpa baris saat token habis). Turunan Klaus: 429 limiter tanpa baris dan tanpa `reject_reason`, batas ±86.420 baris/hari per client (DQ-32 (e)); JSON tidak valid dan kunci ganda → baris `rejected`/`validasi_gagal` 400 dengan aturan NULL `reject_http_status`; penolakan tanpa baris (Idempotency-Key) tetap mengambil token. Mitigasi K-31 di D3: `config_schema_version` di dokumen provider dan endpoint (contract V-05, U-46, DQ-33), dengan contoh dokumen. Sumber diperbarui ke PRD v0.4.6, contract v0.7.4, ADR-0001 rev 6. D1–D5 tidak berubah |
