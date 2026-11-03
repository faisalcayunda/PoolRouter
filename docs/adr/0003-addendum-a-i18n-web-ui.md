# ADR-0003 Addendum A: Internasionalisasi (i18n) Web UI PoolRouter, Bahasa Indonesia dan English

**Ringkasan:** Keputusan owner (ganti bahasa di Pengaturan, dicatat sebagai K-39 di SHAA-214) memasukkan dua bahasa UI ke F0: Bahasa Indonesia (default) dan English, dengan preferensi per pengguna yang tersimpan di server. Arahan owner tambahan (SHAA-214, 2026-11-03) mewajibkan pustaka open-source yang mature dan melarang mekanisme i18n buatan sendiri. **Rev 2** menerapkan arahan itu dengan bukti per kriteria (§3), dan mengganti pilihan rev 1 (react-intl, karena lisensi intinya BSD-3-Clause) menjadi **i18next + react-i18next** (MIT) dengan **i18next-cli** untuk pengecekan CI. Addendum ini memutuskan: (1) katalog JSON per bahasa di `frontend/src/i18n/messages/` dengan format JSON v4 i18next (plural lewat `Intl.PluralRules`, interpolasi bernama), key bertipe hasil `i18next-cli types`; (2) larangan string hardcode lewat `i18next-cli lint` dan tipe TypeScript; (3) semua tanggal, waktu, dan angka lewat `Intl` dengan zona tetap `Asia/Jakarta` dan locale format `id-ID` atau `en-GB`; (4) kolom `admin_user.bahasa_ui`, field di `GET /auth/me`, dan endpoint `PATCH /auth/me/preferensi`; sebelum login selalu Bahasa Indonesia; (5) backend hanya mengirim `kode` error stabil, sedangkan teks error berasal dari katalog frontend; (6) key atau terjemahan yang hilang menggagalkan CI. Biaya implementasi tetap 3–4 hari kerja frontend dan 1 hari kerja backend di atas rencana F0. Penambahan dependensi tetap lewat review Ridley (ADR-0003 §6).

- **Pemilik:** Shu
- **Tanggal:** 2026-11-03
- **Status:** final rev 2. Difinalkan Shu sebagai lead Engineering (arahan owner 2026-11-03, SHAA-180). Pemberi tugas: Fitcher. Rev 2 menggantikan §2 (I-1 sampai I-5), §3, sebagian §4, §8, dan §9 rev 1. Bagian lain sama dengan rev 1.
- **Issue:** SHAA-214 (induk SHAA-162)
- **Melengkapi:** ADR-0003 rev 5 (lampiran SHAA-196) §2, §3.4, §3.6f, §3.7, §5, §7.1, §7.6; ADR-0001 rev 11 §5a.7 (format error). Teks ADR induk tidak diubah. Bila ada pertentangan, addendum ini yang berlaku, lalu ADR terkait diselaraskan di revisi berikutnya.
- **Lokasi akhir:** `docs/adr/0003-addendum-a-i18n-web-ui.md` di repo `faisalcayunda/PoolRouter`, di-commit bersama kerangka frontend (SHAA-197). Sampai saat itu, lampiran di SHAA-214 adalah salinan resmi.

## 1. Konteks

| Sumber | Isi | Dampak |
|---|---|---|
| Keputusan owner, ganti bahasa (SHAA-214; scope oleh Fitcher) | Bahasa Indonesia (default) dan English masuk F0. Struktur harus siap menambah bahasa tanpa mengubah kode komponen. Preferensi per pengguna disimpan di server. Sebelum login atau bila belum diatur: Bahasa Indonesia | Katalog per bahasa, preferensi di profil pengguna, endpoint baru |
| Scope terjemahan (SHAA-214) | Diterjemahkan: semua teks antarmuka (menu, label, validasi, error yang tampil di UI, email/notifikasi bila ada di F0) serta format tanggal, waktu, dan angka. Zona waktu tetap Asia/Jakarta | Tidak boleh ada teks UI di luar katalog. Format lewat `Intl` |
| Non-scope (SHAA-214) | Data input pengguna, body respons API untuk client dan provider upstream, log, dokumentasi teknis (SHAA-176), terjemahan otomatis | Backend tidak dilokalisasi. API downstream tidak berubah |
| ADR-0003 rev 5 §5 | `src/i18n/id.ts` sebagai pusat teks; "tanpa pustaka i18n di F0, karena hanya satu bahasa" | Diganti oleh §4 addendum ini |
| ADR-0003 rev 5 §3.4 | Skema Zod dengan "pesan Bahasa Indonesia" | Pesan Zod diganti key katalog (§6.2) |
| ADR-0003 rev 5 §7.6 | `lang="id"`, locale `id-ID`, contoh "3 Nov 2026, 14.30 WIB" | `lang` mengikuti bahasa aktif (§5) |
| ADR-0001 rev 11 §5a.7 | Error RFC 9457 dengan `kode` stabil; 422 memuat `errors` berisi nama field tanpa nilai | `kode` dipakai sebagai key terjemahan; `errors` perlu `kode` per field (§6.1) |
| PRD v0.4.8 | F0 tidak punya email atau notifikasi ke manusia. Webhook adalah kanal mesin | Butir "email/notifikasi" tidak berlaku di F0 (§6.4) |
| Fakta React Aria (dicek 2026-11-03, react-aria.adobe.com/quality) | String bawaan React Aria tersedia untuk 36 locale. **`id-ID` tidak termasuk**; untuk locale itu RAC memakai string English | Celah yang sudah ada sejak rev 2: pengumuman pembaca layar bawaan RAC berbahasa English saat UI Bahasa Indonesia. Ditangani di §5.4 |
| Arahan owner tambahan (komentar Fitcher di SHAA-214, 2026-11-03) | Wajib pustaka open-source yang mature; mekanisme i18n buatan sendiri dilarang. Kriteria: dipelihara aktif (rilis 12 bulan terakhir, sudah melewati 1.x stabil); adopsi luas (unduhan mingguan dan dependent), lisensi permisif (MIT/Apache-2.0), tanpa CVE terbuka; mendukung React 19 + TypeScript, plural/ICU atau sejenisnya, lazy-load katalog, dan key type-safe; ada tooling ekstraksi atau cek key hilang untuk CI. Pembanding minimal react-i18next dan react-intl | Opsi katalog buatan sendiri dikeluarkan. Bukti per kriteria di §3.2 |

## 2. Ringkasan keputusan

| ID | Area | Keputusan | Versi minimum |
|---|---|---|---|
| I-1 | Pustaka | `i18next` + `react-i18next` (`I18nextProvider`, `useTranslation`, `<Trans>`). Tanpa plugin detektor bahasa, backend HTTP, atau ICU | `i18next` 26.5, `react-i18next` 17.1 |
| I-2 | Format pesan | Format JSON v4 i18next: interpolasi bernama `{{nama}}`, plural lewat sufiks `_one`/`_other` yang dipilih `Intl.PluralRules`, formatter bawaan `{{nilai, number}}`. ICU tidak dipakai di F0 (§3.3) | — |
| I-3 | Katalog | JSON bersarang per bahasa: `frontend/src/i18n/messages/id.json` (sumber) dan `en.json`. Satu namespace `translation`. Key eksplisit dan bertipe lewat `i18next-cli types` | — |
| I-4 | Tidak ada string hardcode | `i18next-cli lint` (teks JSX dan atribut yang tidak lewat `t()`/`<Trans>` = error), ditambah API komponen yang hanya menerima key bertipe atau hasil `t()` | `i18next-cli` 1.82 |
| I-5 | Kelengkapan | `make -C frontend i18n-check`: `i18next-cli status` (terjemahan hilang), `i18next-cli types --ci` (tipe basi), `i18next-cli lint`, dan `tsc` atas tipe key dan pemetaan `kode`. Gagal = CI gagal | `i18next-cli` 1.82 |
| I-6 | Format | Tanggal, waktu, dan angka lewat `Intl` (dibungkus helper `format.ts` yang memakai locale format bahasa aktif). Zona selalu `Asia/Jakarta`. Locale format: `id` → `id-ID`, `en` → `en-GB`. Label "WIB" dari katalog | — |
| I-7 | Preferensi | `admin_user.bahasa_ui` (`id`/`en`/NULL), dikirim di `GET /admin/v1/auth/me`, diubah lewat `PATCH /admin/v1/auth/me/preferensi`. Sebelum login: Bahasa Indonesia, tanpa penyimpanan browser | — |
| I-8 | Error backend | Backend mengirim `kode` (dan `kode` per field untuk 422). UI tidak pernah menampilkan `title` atau `detail` dari backend. Teks diambil dari katalog `error.kode.<kode>` | — |
| I-9 | Menambah bahasa | Tambah `messages/<tag>.json`, satu entri di `src/i18n/locales.ts`, dan satu nilai enum `BahasaUi` di OpenAPI dan `CHECK` database. Kode komponen tidak berubah | — |

Baris I-1, I-4, dan I-5 masuk tabel §2 ADR-0003 sebagai elemen stack yang diikat. Penggantiannya mengikuti ADR-0003 §6 butir 2.

## 3. Pustaka i18n dan format katalog

### 3.1 Kriteria

Kriteria K1–K4 berasal dari arahan owner tambahan (SHAA-214). K5–K7 dari batasan ADR-0003 yang sudah mengikat.

| ID | Kriteria |
|---|---|
| K1 | Dipelihara aktif: rilis dalam 12 bulan terakhir, sudah melewati versi stabil 1.x |
| K2 | Adopsi luas (unduhan mingguan npm, dependent), lisensi permisif MIT/Apache-2.0, tanpa CVE terbuka yang belum ditambal |
| K3 | Mendukung React 19 + TypeScript, plural/ICU atau sejenisnya, lazy-load katalog, key type-safe |
| K4 | Tooling ekstraksi atau cek key hilang untuk CI |
| K5 | Tidak melanggar CSP `script-src 'self'` (tanpa `eval` atau `new Function`) |
| K6 | Format tanggal dan angka memakai `Intl` bawaan browser |
| K7 | Tidak menambah transformasi build (macro Babel atau plugin compiler di Vite) |

Mekanisme i18n buatan sendiri (opsi A rev 1) dilarang arahan owner dan tidak dibandingkan lagi.

### 3.2 Bukti per kandidat (dicek 2026-11-03)

Sumber: npm registry (`/latest`), npm downloads API (minggu 2026-10-26 s.d. 2026-11-01), halaman rilis GitHub, OSV.dev, dan dokumentasi resmi masing-masing proyek.

| Kriteria | **B. i18next + react-i18next + i18next-cli** | C. react-intl (FormatJS) + `@formatjs/cli` + `eslint-plugin-formatjs` |
|---|---|---|
| K1 versi dan rilis | `i18next` 26.7.1, dirilis 2026-11-01 (v26.7.0 2026-11-01, v26.6.0 2026-10-31). `react-i18next` 17.1.11. `i18next-cli` 1.82.1 (pemelihara inti i18next, repo `github.com/i18next/i18next-cli`). Semua jauh di atas 1.x | `react-intl` 12.1.6. Monorepo formatjs merilis paket pada 2026-10-31 dan 2026-11-01. `@formatjs/cli` 6.18.0, `eslint-plugin-formatjs` 8.1.3 |
| K2 unduhan mingguan | `i18next` 33,8 juta; `react-i18next` 24,8 juta; `i18next-cli` 1,15 juta | `react-intl` 4,4 juta; `intl-messageformat` 29,9 juta (sebagian besar lewat dependensi transitif) |
| K2 lisensi | **MIT** untuk ketiga paket. `i18next` tanpa dependensi runtime | **BSD-3-Clause** untuk `react-intl` dan `intl-messageformat` (inti runtime). MIT untuk `@formatjs/intl`, CLI, dan plugin ESLint. BSD-3-Clause permisif, tetapi tidak termasuk MIT/Apache-2.0 yang disebut kriteria |
| K2 CVE | OSV.dev: tidak ada advisory untuk `i18next`, `react-i18next`, atau `i18next-cli`. Advisory 2026 hanya untuk plugin yang **tidak dipakai** (`i18next-http-backend`, `i18next-fs-backend`, `i18next-http-middleware`, `i18next-locize-backend`); semuanya sudah punya perbaikan. Plugin itu dilarang di repo ini (§3.3) | OSV.dev: tidak ada advisory untuk `react-intl`, `intl-messageformat`, atau `@formatjs/*` |
| K3 React 19 + TS | Peer `react >= 16.8`; tipe resmi. Key type-safe lewat augmentasi `CustomTypeOptions` dengan tipe hasil `i18next-cli types`; argumen interpolasi ikut bertipe (`parseInterpolation`) | Peer `react >= 18`; key type-safe lewat augmentasi `FormatjsIntl.Message` |
| K3 plural | Bawaan: sufiks `_zero`/`_one`/`_two`/`_few`/`_many`/`_other` dipilih `Intl.PluralRules` | ICU MessageFormat bawaan |
| K3 lazy-load | `addResourceBundle` setelah `import()` katalog, lalu `changeLanguage` | Ganti prop `messages` setelah `import()` |
| K4 CI | `i18next-cli status` (terjemahan hilang di locale sekunder, key di kode yang tidak ada di bahasa sumber, opsi `checkIdentical` untuk nilai yang sama dengan bahasa sumber), `types --ci` (tipe basi = gagal), `extract --ci`, `lint` (teks JSX tanpa `t()` = error, ketidakcocokan placeholder antarlocale, konkatenasi string) | `formatjs verify --missing-keys --extra-keys --structural-equality`, `eslint-plugin-formatjs` `no-literal-string-in-jsx` |
| K5 CSP | Tanpa `eval` | Tanpa `eval` |
| K6 `Intl` | Formatter bawaan membungkus `Intl.NumberFormat`/`DateTimeFormat`; F0 memakai helper `format.ts` (§5) | `formatDate`/`formatNumber` membungkus `Intl` |
| K7 build | Tanpa plugin Vite. `i18next-cli` hanya alat CLI dev (berbasis SWC, berjalan di runner container ADR-0003 §3.2) | Tanpa plugin Vite |

Lingui dan Paraglide (rev 1, opsi D dan E) tetap tidak dipilih: keduanya butuh transformasi build (gagal K7), dan unduhannya jauh di bawah dua kandidat di atas.

### 3.3 Keputusan: B (i18next + react-i18next + i18next-cli)

**Alasan.** B memenuhi K1–K7. C memenuhi semua kriteria kecuali K2 lisensi: inti runtime-nya BSD-3-Clause, sedangkan arahan owner menyebut MIT/Apache-2.0. B juga unggul di adopsi (5–6× unduhan) dan tooling. `status` dengan `checkIdentical` menutup risiko rev 1 "terjemahan English tidak dicek mesin", dan `lint` memeriksa ketidakcocokan placeholder antarlocale. Kelemahan B dibanding C adalah format pesan non-ICU. Arahan owner menerima "plural/ICU atau sejenisnya", dan plural i18next memakai kategori CLDR yang sama lewat `Intl.PluralRules`. Rev 1 memilih C dengan alasan format ICU. Alasan itu kalah oleh kriteria owner, dan belum ada kode yang terbangun (SHAA-197 masih `backlog`), jadi penggantian ini tanpa biaya.

**Aturan pemakaian:**
- **ICU tidak dipakai.** Plugin `i18next-icu` bergantung pada `intl-messageformat` (BSD-3-Clause), jadi tidak dipasang. Pesan memakai format JSON v4 i18next (§3.4).
- **Plugin yang dilarang:** detektor bahasa browser (bertentangan dengan default Indonesia sebelum login, §7.3), backend HTTP, filesystem, dan locize (permukaan CVE 2026 di atas, dan tidak dibutuhkan karena katalog dibundel). Penambahan plugin i18next apa pun mengikuti ADR-0003 §6 butir 3.
- **Konfigurasi wajib:** `supportedLngs: ['id','en']`, `fallbackLng: 'id'`, `load: 'languageOnly'`, `interpolation.escapeValue: false` (React sudah meng-escape), `returnNull: false`, `saveMissing: false`. Di test, `missingKeyHandler` melempar error.

Dependensi baru (masuk tabel §2 ADR-0003; perubahan `package.json` direview Ridley, ADR-0003 §6 butir 1):

| Paket | Jenis | Versi minimum | Lisensi |
|---|---|---|---|
| `i18next` | runtime | 26.5 (peer `react-i18next` 17.1 menuntut ≥ 26.2; 26.5 dipilih agar fitur tipe `i18next-cli types` terbaru tersedia) | MIT |
| `react-i18next` | runtime | 17.1 | MIT |
| `i18next-cli` | dev | 1.82 | MIT |

### 3.4 Aturan format pesan

- **Bahasa sumber: `id`.** Nanami menulis teks Indonesia lebih dulu. `en.json` harus punya key yang sama.
- **Interpolasi:** argumen bernama `snake_case`, misalnya `{{nama_client}}` atau `{{jam}}`. Tidak ada argumen posisional.
- **Kalimat utuh:** satu key berisi satu kalimat atau frasa utuh dengan placeholder. Teks tidak boleh digabung dari beberapa key atau dengan `+` di kode, karena urutan kata berbeda antarbahasa. `i18next-cli lint` menandai konkatenasi.
- **Plural:** pesan yang bergantung jumlah dipanggil dengan `t(key, { count })` dan ditulis dengan sufiks kategori CLDR. Bahasa Indonesia hanya punya kategori `other`, jadi `id.json` cukup `_other`. `en.json` wajib `_one` dan `_other`. Contoh:
  - `id`: `"pending_count_other": "{{count, number}} permintaan menunggu"`
  - `en`: `"pending_count_one": "{{count, number}} request pending"`, `"pending_count_other": "{{count, number}} requests pending"`
- **Angka di dalam pesan** selalu memakai formatter `{{count, number}}` atau `{{jumlah, number}}`, sehingga pemisah ribuan mengikuti locale. `{{count}}` tanpa formatter menampilkan angka mentah dan dilarang untuk angka yang dibaca pengguna.
- **Variasi kalimat per nilai enum** memakai fitur context i18next (`t(key, { context: peran })`, key `..._auditor`). Nilai enum yang ditampilkan sebagai label memakai key enum (§6.3).
- **Rich text:** hanya lewat `<Trans>` dengan peta komponen untuk `<b>`, `<code>`, dan `<link>`. HTML mentah di pesan dan `dangerouslySetInnerHTML` dilarang.
- **NIK, secret, dan nilai input tidak pernah menjadi argumen pesan.** Aturan ADR-0003 §3.6 dan §7.6 tetap berlaku untuk semua bahasa.

## 4. Struktur katalog di `frontend/`

```
frontend/src/i18n/
├── locales.ts          # registri bahasa: BahasaUi → { tag, formatLocale, endonym, load() }
├── messages/
│   ├── id.json         # sumber, dimuat eager (default dan sebelum login)
│   └── en.json         # dimuat lewat dynamic import() saat dipilih
├── i18n.ts             # inisialisasi i18next (konfigurasi wajib §3.3), tanpa plugin terlarang
├── provider.tsx        # I18nextProvider + I18nProvider RAC + sinkronisasi <html lang> dan document.title
├── format.ts           # helper format tanggal, waktu, angka, dan rentang (§5)
├── errors.ts           # Record<ProblemKode, MessageId> dan pemetaan 422 ke field (§6)
├── enums.ts            # Record<nilai enum server, MessageId> untuk label status (§6.3)
├── i18next.d.ts        # dibangkitkan `i18next-cli types` (di-commit; dicek `types --ci`); augmentasi CustomTypeOptions
├── types.ts            # alias MessageId = kunci bertipe dari i18next (ParseKeys)
├── README.md           # aturan key, cara menambah bahasa, alur perubahan teks
└── (akar frontend) i18next.config.ts  # konfigurasi i18next-cli: locales, sumber id, lint, status
```

Folder ini menggantikan `src/i18n/id.ts` di ADR-0003 rev 5 §5.

**Aturan key:**
- JSON bersarang; path key ditulis dengan titik di kode. Pola `<ruang>.<layar atau komponen>.<elemen>[.<keadaan>]`, segmen `[a-z0-9_]+`, identifier English (bahasa kode, ADR-0003 §5). Contoh: `common.action.save`, `provider.form.secret.label`, `log.search.empty.title`, `session.idle.dialog.body`.
- Ruang khusus: `common.*` untuk teks yang memang sama maknanya di semua layar; `error.kode.<kode>` untuk error backend (nilai `kode` apa adanya); `error.http.<kelas>` untuk fallback; `validation.*` untuk validasi form; `enum.<nama_enum>.<nilai>` untuk label nilai enum server; `title.<layar>` untuk `document.title`.
- Satu key untuk satu konteks. Dua teks yang kebetulan sama di dua konteks berbeda tetap memakai dua key, karena terjemahannya bisa berbeda.
- Key tidak dihapus atau diganti nama tanpa memperbarui kedua katalog di commit yang sama (`i18next-cli status --unused` membantu menemukan key yatim; hasilnya informasi, bukan gate, karena key yang dirujuk lewat peta `Record` tidak terdeteksi sebagai pemakaian).

**Tipe:**
- `i18next-cli types` membangkitkan `i18next.d.ts` dari `id.json`, dengan `CustomTypeOptions.resources` dan pengetikan argumen interpolasi. Akibatnya `t('key.tidak.ada')` dan argumen yang hilang gagal di `tsc`. File ini di-commit, dan CI menjalankan `types --ci` agar tidak basi.
- `MessageId` adalah alias tipe key i18next dan dipakai di `errors.ts`, `enums.ts`, serta prop komponen yang menerima key.
- Teks tidak ditulis sebagai nilai default di kode (`t('key', 'teks')` dilarang). Kode hanya merujuk key.

**Pemuatan:**
- `id.json` dibundel di chunk utama karena dipakai sebelum login.
- `en.json` dimuat dengan `import()` (chunk same-origin, aman untuk CSP), lalu `addResourceBundle('en', 'translation', ...)` dan `changeLanguage('en')`. Aplikasi menunggu katalog termuat sebelum merender shell, sehingga tidak ada kilasan teks bahasa lain. `react-i18next` dipakai tanpa Suspense (`useSuspense: false`), dan state loading memakai state shell yang sudah ada.
- `fallbackLng: 'id'`: key yang lolos dari CI tetap tampil dalam Bahasa Indonesia, bukan sebagai key mentah.
- Key hilang saat runtime: di test, `missingKeyHandler` melempar error sehingga test komponen gagal. Di build produksi hanya `console.warn` dengan key, tanpa nilai argumen.

**Alur perubahan teks:** teks dimiliki Nanami (UX copy) dan disetujui Fitcher. Tabel key → ID → EN dari SHAA-216 menjadi isi awal katalog. Viktor memindahkannya ke JSON. Perubahan teks berikutnya mengikuti alur commit biasa (K-34/K-35): Viktor meng-commit, Ridley mereview. Perubahan teks saja tidak memerlukan revisi ADR.

## 5. Tanggal, waktu, angka, dan zona waktu lewat `Intl`

### 5.1 Locale format

| Bahasa UI | `lang` di `<html>` | Locale format (`Intl`, RAC `I18nProvider`) | Alasan |
|---|---|---|---|
| `id` | `id` | `id-ID` | Sama dengan rev 5 §7.6 |
| `en` | `en` | `en-GB` | Urutan tanggal `dd/mm/yyyy`, jam 24 jam, dan minggu mulai Senin sama dengan `id-ID`. Dengan begitu input tanggal manual (handoff spec v0.3.2, `dd/mm/yyyy`) dan kebiasaan pengguna BGN tidak berubah saat ganti bahasa. `en-US` (`mm/dd/yyyy`, 12 jam) berisiko salah baca tanggal di data audit |

Locale format adalah properti registri (`locales.ts`), bukan bagian kode komponen. Tag bahasa i18next tetap `id`/`en`; locale format hanya dipakai `format.ts` dan `I18nProvider` RAC. Ejaan teks English di katalog ditentukan Nanami dan dipakai konsisten.

### 5.2 Aturan

- **Zona waktu:** `format.ts` memakai `timeZone: 'Asia/Jakarta'` di setiap `Intl.DateTimeFormat`. Zona browser pengguna tidak pernah dipakai. Nilai dari API selalu ISO 8601 UTC dan dikonversi hanya di `format.ts`. Tanggal untuk RAC memakai `@internationalized/date` dengan zona yang sama (rev 5 §3.5).
- **Label zona** "WIB" diambil dari katalog lewat pesan gabungan, misalnya `common.datetime.wib`: `"{{tanggal}}, {{jam}} WIB"`. `timeZoneName` dari `Intl` tidak dipakai, karena keluarannya berbeda antarversi ICU browser (misalnya "WIB" atau "GMT+7").
- **Tanggal dan jam diformat terpisah, lalu digabung oleh pesan katalog.** Format gabungan `Intl` di `id-ID` bisa berbeda antarbrowser (misalnya penghubung "pukul"). Hasil yang diharapkan:
  - `id`: `3 Nov 2026, 14.30 WIB`
  - `en`: `3 Nov 2026, 14:30 WIB`
- **Format bernama** di `format.ts`, tidak dibuat ad hoc di komponen: `tanggal` (`day: 'numeric', month: 'short', year: 'numeric'`), `jam` (`hour: '2-digit', minute: '2-digit', hourCycle: 'h23'`), `tanggalWaktu` (gabungan lewat katalog), `angka`, `persen`, dan `durasi` (`{{m}}:{{ss}}`, dari `Intl.NumberFormat` dengan `minimumIntegerDigits: 2`). `Intl.DurationFormat` tidak dipakai karena belum tersedia di semua target browser ADR-0003 §3.2.
- **Dialog sesi (rev 5 §3.6f):** jam berakhir sesi ("pukul {{jam}} WIB") memakai format `jam` dari `waktu_server`, dan penghitung sisa waktu memakai `durasi`. Aturan pengumuman pembaca layar §3.6f tidak berubah oleh bahasa.
- **Angka:** `Intl.NumberFormat` (lewat `format.ts` atau formatter `{{x, number}}`) mengikuti locale (`id`: `1.240`; `en`: `1,240`). Kode transaksi, `queue_id`, ID, dan nilai mentah lain bukan angka untuk diformat. Nilai itu ditampilkan apa adanya dengan `tabular-nums`.
- **ISO 8601 mentah** tetap tersedia di `Tooltip` atau detail (rev 5 §7.6), tanpa diterjemahkan.
- **Waktu relatif** ("2 menit lalu"), bila dipakai desain, memakai `Intl.RelativeTimeFormat` lewat `format.ts`.
- **Input tanggal manual** diparse dengan format tetap `dd/mm/yyyy` untuk kedua bahasa (§5.1). Supporting text contoh format ("Format: 03/11/2026") diambil dari katalog.
- **Uji:** unit test `format.ts` untuk kedua bahasa dijalankan di Node 24 (ICU penuh). E2E tidak mencocokkan string tanggal gabungan secara persis, cukup pola dan zona.

### 5.3 Elemen dokumen

- `<html lang>` diperbarui setiap kali bahasa berubah. Ini dibutuhkan WCAG 3.1.1 dan pengucapan pembaca layar.
- `document.title` mengikuti pola "{Layar} · PoolRouter" (handoff spec v0.3.2) dari key `title.*`.
- Rute dan path URL tidak diterjemahkan dan sama untuk semua bahasa, agar bookmark dan tautan tetap berlaku.

### 5.4 String bawaan React Aria (celah `id-ID`)

RAC tidak menyertakan string `id-ID`. Untuk locale itu, label dan pengumuman bawaannya berbahasa English (misalnya label tombol tutup tersembunyi, pengumuman sortir tabel, dan jumlah opsi ComboBox). Celah ini sudah ada sejak rev 2, terlepas dari keputusan ganti bahasa. Keputusan:
1. Setiap prop label yang disediakan RAC (`aria-label`, label tombol navigasi kalender, label region Toast, dan sejenisnya) wajib diisi dari katalog di komponen `src/components/m3/`. Komponen tidak bergantung pada string bawaan RAC bila ada prop pengganti.
2. Di SHAA-197, Viktor menginventaris string bawaan RAC yang tetap muncul di komponen katalog §7.1 dan tidak bisa diganti lewat prop. Daftar itu ditulis di `src/components/m3/README.md`.
3. Sisa string English di UI Bahasa Indonesia diterima untuk F0 sebagai keterbatasan yang tercatat. Audit pembaca layar Quality (ADR-0003 §3.8) memeriksa kedua bahasa dan mencatat sisa ini di laporan aksesibilitas. Penutupan permanen, dengan menyumbang string `id-ID` ke upstream React Aria atau memakai mekanisme lokal bila tersedia, dinilai di F1 lewat ADR-0003 §6.

## 6. Pesan error, validasi, dan label enum

### 6.1 Error dari backend

| Aspek | Aturan |
|---|---|
| Sumber teks | Backend mengirim Problem Details RFC 9457 dengan `kode` stabil (ADR-0001 §5a.7). UI menampilkan `error.kode.<kode>` dari katalog. `title` dan `detail` dari backend **tidak pernah dirender** di UI, karena bahasanya tetap dan bukan teks untuk pengguna. Keduanya tetap ada untuk client API dan diagnosis |
| Kontrak `kode` | Semua nilai `kode` admin API didaftarkan sebagai enum `ProblemKode` di `backend/openapi/admin-api.yaml`, sehingga ikut ke `schema.gen.ts`. Menambah `kode` baru di backend berarti menambah nilai enum |
| Kelengkapan saat compile | `errors.ts` memuat `Record<ProblemKode, MessageId>`. `kode` baru tanpa key katalog membuat `tsc` gagal, sehingga lint gagal di commit yang sama (alur kontrak ADR-0003 §4.2) |
| 422 per field | Item `errors` diperluas dari "nama field tanpa nilai" menjadi `{ "field": "<json pointer atau nama field>", "kode": "<kode validasi>", "batas": { "min": 10, "max": 500 } }`. `batas` opsional dan hanya memuat parameter aturan berupa angka. Nilai input, NIK, dan secret tidak pernah dikirim. UI menampilkan `validation.<kode>` dengan `batas` sebagai argumen pesan di field terkait (helper rev 5 §3.4) |
| `kode` tidak dikenal | Terjadi bila frontend lebih lama dari backend di luar alur monorepo. UI menampilkan `error.http.4xx` atau `error.http.5xx`, ditambah nilai `kode` dalam teks mono untuk dukungan. `kode` adalah identifier ASCII tanpa data pengguna |
| Error jaringan dan timeout | Key `error.network.*` dari katalog |
| API downstream | Tidak berubah. Body respons untuk client dan provider upstream di luar scope |

### 6.2 Validasi form (Zod)

- Pesan Zod tidak ditulis sebagai teks. Setiap aturan memakai key `validation.*` sebagai `error`/`message`, misalnya `z.string().min(10, { error: 'validation.too_short' })`, dan `FieldError` menerjemahkannya. Ini menggantikan "pesan Bahasa Indonesia" di ADR-0003 rev 5 §3.4.
- Satu helper bersama membungkus aturan umum (wajib, panjang, format URL, CIDR), sehingga key dan argumen `batas` seragam dengan 422 dari server. Kode validasi klien dan server memakai himpunan yang sama sebisa mungkin.
- Unit test memastikan setiap skema form hanya menghasilkan pesan yang merupakan `MessageId`. Pesan bawaan Zod tidak boleh sampai ke UI.

### 6.3 Label nilai enum dari server

Nilai enum yang ditampilkan sebagai label, misalnya `status_akses`, `error_class`, status request, status webhook, peran, dan `aksi` audit, diterjemahkan lewat `enums.ts` (`Record<EnumServer, MessageId>` dari `schema.gen.ts`). Enum baru di OpenAPI tanpa key membuat `tsc` gagal. Nilai mentah tetap tersedia di detail atau `Tooltip` untuk penelusuran.

### 6.4 Yang tidak diterjemahkan

- Data yang diinput pengguna: nama provider, pool, client, URL, catatan, dan alasan auditor.
- Body respons API downstream dan upstream, log aplikasi, metrik, dan pesan CLI backend (§3.6c rev 5), yang merupakan alat teknis.
- Berkas ekspor CSV auditor (§3.6d rev 5): nama kolom dan nama berkas tetap, karena berkas ini artefak bukti yang dibaca mesin.
- Dokumentasi teknis (SHAA-176).
- Email dan notifikasi: F0 tidak punya kanal ke manusia (PRD v0.4.8). Bila nanti ada, teksnya memakai `bahasa_ui` penerima dan diputuskan di ADR saat kanal itu dirancang.

## 7. Penyimpanan preferensi bahasa

### 7.1 Opsi

| Opsi | Kelebihan | Kekurangan |
|---|---|---|
| **A. Kolom di `admin_user` + field di `auth/me` + endpoint preferensi** | Ikut ke perangkat lain (syarat scope); satu sumber; tanpa storage browser | Satu migrasi dan satu endpoint |
| B. `localStorage` saja | Tanpa backend | Tidak ikut ke perangkat lain; melanggar scope |
| C. Tabel preferensi generik (key-value) | Siap untuk preferensi lain | Skema longgar tanpa `CHECK` bertipe; tidak ada preferensi lain di F0 |

**Keputusan: A.** C ditunda sampai ada preferensi kedua. Saat itu kolom dipindahkan lewat migrasi.

### 7.2 Kontrak backend (path final di OpenAPI admin)

| Aspek | Aturan |
|---|---|
| Skema | `admin_user.bahasa_ui TEXT NULL CHECK (bahasa_ui IN ('id','en'))`. NULL berarti belum diatur, dan UI memakai `id`. Tanpa default `'id'` di database, supaya "belum memilih" tetap bisa dibedakan dari "memilih Indonesia" |
| OpenAPI | Skema `BahasaUi` = enum `[id, en]` |
| Baca | `GET /admin/v1/auth/me` menambah `bahasa_ui: BahasaUi \| null` |
| Ubah | `PATCH /admin/v1/auth/me/preferensi` dengan body `{ "bahasa_ui": "en" }` → 200 dengan `{ "bahasa_ui": "en" }`. Hanya untuk pengguna sesi itu sendiri: tidak ada ID pengguna di path atau body, sehingga tidak ada IDOR. Semua peran boleh. Nilai di luar enum → 422 `kode` validasi. Tanpa sesi → 401. Aturan CSRF `Origin`/`Sec-Fetch-Site` (rev 5 §3.6) dan `Cache-Control: no-store` berlaku |
| Audit | Bukan akses data. Tidak menulis `audit_akses`. Cukup log aplikasi terstruktur tanpa PII |
| Sesi | Mengubah bahasa tidak mencabut atau memperbarui sesi |
| CLI pengguna (§3.6c) | Tidak perlu opsi bahasa. Pengguna baru mulai dengan NULL |
| Seed `dev`/`ci` | Satu pengguna sintetis dengan `bahasa_ui = 'en'` untuk uji e2e |

### 7.3 Perilaku frontend

- **Sebelum login** (`/login` dan halaman sesi berakhir): selalu `id`. Tidak ada deteksi `navigator.language` dan tidak ada storage browser. Ini sesuai scope ("sebelum login: Bahasa Indonesia").
  - Bila addendum UX SHAA-216 menetapkan pemilih bahasa di halaman login, mekanisme yang diizinkan hanya satu nilai enum di `localStorage` dengan key `poolrouter.bahasa_ui`. Nilai itu hanya dipakai di halaman sebelum login dan ditimpa nilai server setelah login. Ini bukan data sensitif, jadi tidak melanggar rev 5 §3.6. Tanpa keputusan UX itu, mekanisme ini tidak dibangun.
- **Setelah login:** bahasa = `auth/me.bahasa_ui ?? 'id'`. Shell aplikasi dirender setelah `auth/me` dan katalog bahasa itu termuat (state loading yang sudah ada).
- **Mengganti bahasa di Pengaturan:** satu mutation `PATCH`. UI langsung beralih setelah 200, lalu cache `auth/me` diperbarui dari respons. Selama request berjalan, kontrol nonaktif. Bila gagal, bahasa tetap dan `InlineAlert` tampil. Data server di cache Query tidak perlu dimuat ulang, karena hanya label yang berubah.
- **Tab lain dan perangkat lain:** bahasa ikut berubah saat `auth/me` di-refetch (fokus jendela atau navigasi). F0 tidak memakai BroadcastChannel.
- **Logout:** kembali ke `id` untuk halaman login.

## 8. Cek otomatis (CI)

Target baru `make -C frontend i18n-check`, dijalankan sebagai bagian `make -C frontend lint`. Gate repo `make setup test lint` (ADR-0003 §5) otomatis mencakupnya.

| Cek | Alat | Gagal bila |
|---|---|---|
| Terjemahan hilang | `i18next-cli status` (bahasa sumber `id`) | Ada key di `id.json` yang tidak punya terjemahan di `en.json`, termasuk kategori plural yang wajib untuk `en`, atau kode memakai key yang tidak ada di `id.json` |
| Nilai belum diterjemahkan | `i18next-cli status` dengan `checkIdentical`, dengan daftar pengecualian `status.ignoreKeys` (mis. `common.product_name`, `common.datetime.wib`) | Nilai `en` sama persis dengan `id` di luar daftar pengecualian. Bila opsi ini hanya melaporkan tanpa exit code non-zero, Viktor mengubahnya menjadi gate di target `make` dan mencatatnya di permintaan review |
| Tipe basi | `i18next-cli types --ci` | `i18next.d.ts` tidak cocok dengan `id.json` |
| Key dan argumen di kode | `tsc --noEmit` (tipe hasil `types`) | Kode merujuk key yang tidak ada, atau argumen interpolasi hilang |
| `kode` dan enum server | `tsc --noEmit` (`Record<ProblemKode, MessageId>`, `Record<EnumServer, MessageId>`) | `kode` atau nilai enum baru di OpenAPI belum punya key |
| String hardcode dan placeholder | `i18next-cli lint`, error untuk teks JSX dan atribut (`aria-label`, `title`, `alt`, `placeholder`, `label`, `description`, `supportingText`, `errorMessage`, `tooltip`) yang tidak lewat `t()`/`<Trans>`, ketidakcocokan placeholder antarlocale, dan konkatenasi. Diterapkan di `src/**` kecuali `src/i18n/messages/`, `schema.gen.ts`, dan file test | Ada teks UI di luar katalog, atau placeholder `en` berbeda dari `id` |
| API pemberi teks | TypeScript: `Snackbar`, dialog konfirmasi, dan `document.title` hanya menerima `MessageId` (dengan argumennya) atau hasil `t()`. Konfigurasi navigasi dan objek serupa menyimpan `labelKey: MessageId`, bukan teks | String mentah lewat jalur di luar JSX |
| Pesan runtime | Test komponen dengan `missingKeyHandler` yang melempar error | Key hilang saat render |

Pengecualian lint hanya lewat mekanisme pengecualian `i18next-cli lint` (konfigurasi atau baseline `--update-baseline`) dengan alasan tertulis di `i18next.config.ts`, satu entri per kasus. Contohnya simbol teknis yang bukan bahasa. Ridley menolak pengecualian tanpa alasan. Nama produk "PoolRouter" tetap disimpan di katalog (`common.product_name`). Bila `lint` ternyata tidak menangkap suatu pola (mis. string di luar JSX), pola itu ditangani lewat tipe (`MessageId`) atau lewat penambahan aturan sesuai ADR-0003 §6 butir 3, bukan lewat skrip buatan sendiri.

Yang tetap **tidak** dicek mesin: ketepatan makna terjemahan. Itu dijaga lewat tabel copy Nanami yang disetujui Fitcher (SHAA-216).

**Uji teks panjang:** rev 1 memakai pseudo-locale FormatJS. Rev 2 tidak memakainya, karena i18next tidak menyediakan pseudo-locale bawaan dan membuatnya sendiri dilarang. Ketahanan tata letak terhadap teks panjang diuji lewat katalog `en` dan `id` asli di layar berisiko (§10.2), di audit tata letak Quality.

## 9. Pengujian (tambahan ADR-0003 §3.7)

| Lapis | Tambahan |
|---|---|
| Unit | `format.ts` untuk `id` dan `en` (tanggal, jam, gabungan WIB, angka, persen, durasi, rentang); pemetaan 422 → `validation.*` dengan `batas`; fallback `kode` tidak dikenal |
| Komponen | Helper render menerima parameter bahasa. Setiap layar diuji di `id` (default). Satu smoke test per layar di `en` memastikan tidak ada key hilang (`missingKeyHandler`). Halaman Pengaturan: pilih bahasa, request pending, sukses, gagal |
| E2E (alur 10, baru) | (a) Login sebagai pengguna seed `bahasa_ui = NULL` → UI Indonesia, `<html lang="id">`. (b) Pengaturan → English → UI English, `<html lang="en">`, format tanggal `en-GB`. (c) Reload → tetap English. (d) Context browser baru (perangkat lain), pengguna sama → English. (e) Logout → halaman login Indonesia. (f) Pengguna seed `bahasa_ui = 'en'` → langsung English setelah login. (g) Axe dijalankan di Pengaturan dan satu layar tabel dalam kedua bahasa |
| Backend (integrasi) | `PATCH` preferensi: sukses, nilai tidak valid → 422 dengan `kode`, tanpa sesi → 401, tanpa `Origin` yang benar → 403, tidak bisa mengubah pengguna lain; `auth/me` memuat `bahasa_ui` |
| Audit aksesibilitas (Quality) | Audit pembaca layar dilakukan di kedua bahasa. Sisa string RAC English dicatat (§5.4) |

## 10. Batasan desain untuk Jude (UI) dan Nanami (UX)

Bagian ini melengkapi ADR-0003 rev 5 §7. Penyimpangan mengikuti aturan §7 yang sama.

### 10.1 Komponen pemilih bahasa (katalog M3 §7.1, tanpa komponen baru)

- **`RadioGroup`** di halaman Pengaturan, selama jumlah bahasa ≤ 5. Semua opsi terlihat, satu langkah, dan sesuai pedoman M3 untuk pilihan eksklusif yang sedikit. Bila bahasa > 5, kontrol diganti `Select` (exposed dropdown) tanpa mengubah kontrak data.
- Label opsi memakai **endonym** dan tidak diterjemahkan: "Bahasa Indonesia" dan "English", masing-masing dengan atribut `lang` sendiri (WCAG 3.1.2). Bendera tidak dipakai, karena bendera menandai negara, bukan bahasa.
- Label grup ("Bahasa tampilan" / "Display language") dan supporting text yang menjelaskan bahwa pilihan berlaku di semua perangkat berasal dari katalog.
- Perilaku simpan: pilihan langsung disimpan (satu `PATCH`, §7.3), tanpa tombol Simpan terpisah. Selama menyimpan, `RadioGroup` nonaktif dengan `CircularProgress`. Konfirmasi memakai `Snackbar` dalam bahasa baru. Kegagalan memakai `InlineAlert` dan pilihan kembali ke nilai lama. Bila UX SHAA-216 memilih pola tombol Simpan, kontrak teknisnya sama.
- **Halaman Pengaturan** adalah tujuan navigasi baru untuk semua peran, sehingga drawer menjadi 13–15 tujuan (rev 5 §7.4 tetap memakai navigation drawer). Letaknya ditentukan Nanami di SHAA-216. Isi F0 hanya pemilih bahasa.

### 10.2 Panjang teks EN vs ID

Teks Indonesia umumnya lebih panjang daripada English, tetapi tidak selalu (misalnya label pendek dan istilah teknis). Desain wajib tahan terhadap keduanya dan terhadap bahasa berikutnya:
- Desain diukur dengan teks **terpanjang** dari kedua bahasa, ditambah cadangan: +100% untuk teks ≤ 10 karakter (label tombol, chip, tab, header tabel) dan +30% untuk teks yang lebih panjang.
- Tidak ada lebar tetap untuk elemen yang berisi teks UI. Tombol, chip, tab, dan `StatusLabel` tumbuh mengikuti isi. Batasnya ditentukan lebar kontainer, bukan lebar teks contoh.
- Label UI tidak pernah dipotong dengan elipsis. Teks boleh membungkus: item navigation drawer dan header `DataTable` maksimal 2 baris, label tombol 1 baris. Bila tidak muat, desain diubah (ikon + `Tooltip`, atau susunan vertikal). Elipsis hanya untuk data pengguna, dengan nilai penuh di `Tooltip` atau detail.
- Judul `TopAppBar` dan `Dialog` boleh dua baris di window compact.
- Layar dan komponen berisiko dari catatan Nanami (SHAA-216) diperiksa di frame 375 px dan reflow 320 px dalam kedua bahasa.
- Teks di desain tidak dibuat sebagai gambar atau path vektor.

### 10.3 Handoff spec

- Setiap teks di handoff spec menyebut **key katalog** (§4), selaras dengan tabel key → ID → EN SHAA-216. Teks tanpa key tidak bisa diimplementasikan karena lint akan menolaknya.
- Contoh format tanggal dan angka ditulis untuk kedua bahasa: `3 Nov 2026, 14.30 WIB` / `3 Nov 2026, 14:30 WIB`; `1–25 dari 1.240` / `1–25 of 1,240`.
- Input tanggal manual tetap `dd/mm/yyyy` di kedua bahasa (§5.1).
- Typeface Roboto mencakup semua karakter kedua bahasa. Token tipografi tidak berubah.
- Mockup Figma English cukup dibuat untuk layar berisiko dan halaman Pengaturan. Layar lain cukup versi Indonesia dengan key.

## 11. Konsekuensi

- **Lebih mudah:** menambah bahasa cukup lewat katalog dan registri (I-9). Teks error konsisten di semua layar karena berasal dari satu pemetaan `kode`. Key yang hilang tertangkap di CI, bukan oleh pengguna. Teks UI yang sudah terpusat di rev 5 (`id.ts`) membuat biaya i18n sejak awal kecil.
- **Lebih sulit atau perlu dikerjakan:**
  - Setiap teks UI butuh key dan dua terjemahan, termasuk teks error dan validasi.
  - Kontrak admin API bertambah: enum `ProblemKode` yang lengkap dan `kode` per field di 422. Ini memperluas ADR-0001 §5a.7 (kompatibel, karena `errors` memperoleh field tambahan). ADR-0001 diselaraskan di revisi berikutnya.
  - Celah string RAC `id-ID` sekarang tercatat (§5.4) dan diterima untuk F0.
  - Ketepatan makna terjemahan English tidak dicek mesin. Kelengkapan dan nilai yang belum diterjemahkan sudah dicek `i18next-cli status` (§8).
  - Format pesan non-ICU: Nanami menulis placeholder `{{nama}}` dan key plural bersufiks. Aturannya ada di §3.4 dan `src/i18n/README.md`.
- **Biaya:** tanpa lisensi berbayar. Bundle bertambah sekitar 15–20 kB gzip (perkiraan untuk `i18next` dan `react-i18next`; ukuran pasti dicatat di permintaan review SHAA-197) ditambah satu chunk `en.json` yang dimuat sesuai kebutuhan. Perkiraan usaha di atas rencana F0: **frontend 3–4 hari kerja** (infrastruktur i18n dan cek CI 1,5 hari; halaman Pengaturan 0,5 hari; pemindahan katalog EN dan smoke test per layar 1–2 hari); **backend 1 hari kerja** (migrasi kolom, field `auth/me`, endpoint preferensi, enum `ProblemKode`, `kode` per field 422, test integrasi). Terjemahan English dari Nanami ada di SHAA-216.
- **Pekerjaan lanjutan (AC tambahan di sub-issue SHAA-175):**

| Sub-issue | Pelaksana | Tambahan |
|---|---|---|
| SHAA-192 OpenAPI | Flik | Skema `BahasaUi`; `bahasa_ui` di `auth/me`; `PATCH /auth/me/preferensi`; enum `ProblemKode` lengkap; item `errors` 422 `{field, kode, batas?}` |
| SHAA-193 Skema PostgreSQL | Flik (desain data direview Klaus per K-35) | Kolom `admin_user.bahasa_ui` dengan `CHECK`, NULL = belum diatur |
| SHAA-196 Sesi admin dan CLI | Flik | Endpoint preferensi (self-only, CSRF, `no-store`); seed `dev`/`ci` dengan satu pengguna `bahasa_ui = 'en'`; test integrasi §9 |
| SHAA-197 Kerangka frontend | Viktor | `src/i18n/` §4, dependensi §3.3, provider dan `<html lang>`, `format.ts`, `make i18n-check` dan aturan lint §8, inventaris string RAC §5.4 |
| SHAA-202 Web UI login, konfigurasi | Viktor | Halaman Pengaturan dan pemilih bahasa §10.1; perilaku sebelum login §7.3; e2e alur 10 |
| SHAA-203 Web UI log, auditor | Viktor | Label enum §6.3, format tanggal dan angka §5, smoke test `en` per layar |
| SHAA-204 Eksekusi uji (K2 Quality) | squad Quality | Audit pembaca layar dua bahasa; catat sisa string RAC English |

## 12. Pertanyaan terbuka

| ID | Pertanyaan | Untuk | Default bila belum dijawab |
|---|---|---|---|
| W-A1 | Pemilih bahasa di halaman login (sebelum login)? | Nanami (SHAA-216), disetujui Fitcher | Tidak ada. Halaman sebelum login selalu Indonesia (§7.3) |
| W-A2 | Nomor keputusan owner: PRD v0.4.8 §12 sudah memakai **K-39** untuk kepatuhan PDP dan C-04, sedangkan SHAA-214 memakai K-39 untuk ganti bahasa | Fitcher | Addendum ini merujuk "keputusan owner ganti bahasa (SHAA-214)". Nomornya mengikuti penomoran ulang dari Fitcher |

## Rujukan

- SHAA-214 (keputusan owner ganti bahasa dan scope Fitcher); SHAA-215 (user story, Richmond); SHAA-216 (addendum UX dan copy ID/EN, Nanami).
- ADR-0003 rev 5 (lampiran SHAA-196): §2, §3.2, §3.4, §3.5, §3.6, §3.6c, §3.6d, §3.7, §3.8, §4.2, §5, §6, §7.
- ADR-0001 rev 11 (lampiran SHAA-189): §5a.7 Problem Details dan `kode`.
- Handoff spec UI F0 v0.3.2 (SHAA-212): §12.5 copy, tabel semantik dan `<title>`, input tanggal `dd/mm/yyyy`.
- PRD v0.4.8 (lampiran SHAA-190): tanpa email atau notifikasi ke manusia di F0; §12 K-39 (PDP dan C-04).
- i18next: dokumentasi TypeScript (`CustomTypeOptions`, `parseInterpolation`), README `i18next-cli` (`status`, `types --ci`, `extract --ci`, `lint`, `checkIdentical`), halaman rilis `github.com/i18next/i18next` (v26.7.1, 2026-11-01), dicek 2026-11-03.
- OSV.dev, pencarian `i18next` dan `react-intl` (ekosistem npm), dicek 2026-11-03.
- npm downloads API, minggu 2026-10-26 s.d. 2026-11-01: `i18next` 33.791.221, `react-i18next` 24.802.996, `i18next-cli` 1.147.901, `react-intl` 4.410.036, `intl-messageformat` 29.878.712.
- FormatJS (pembanding): dokumentasi CLI dan `eslint-plugin-formatjs`, dicek 2026-11-03.
- Arahan owner tambahan (komentar Fitcher 2026-11-03 di SHAA-214).
- React Aria, halaman Quality bagian Internationalization (36 locale, tanpa `id-ID`), dicek 2026-11-03.
- npm registry, dicek 2026-11-03: `i18next` 26.7.1 (MIT, tanpa dependensi runtime), `react-i18next` 17.1.11 (MIT), `i18next-cli` 1.82.1 (MIT); pembanding `react-intl` 12.1.6 dan `intl-messageformat` 12.1.4 (BSD-3-Clause), `@formatjs/intl` 6.1.4, `@formatjs/cli` 6.18.0, `eslint-plugin-formatjs` 8.1.3 (MIT), `@lingui/core` 6.11.0, `@inlang/paraglide-js` 3.0.0.
- Unicode CLDR plural rules (`id`: hanya `other`); W3C WCAG 2.2 SC 3.1.1 dan 3.1.2.
- Commit: belum ada (repo belum memuat kerangka frontend).

## Riwayat revisi

| Rev | Tanggal | Perubahan |
|---|---|---|
| 1 | 2026-11-03 | Versi pertama (react-intl, ICU), final oleh Shu sebagai lead Engineering (SHAA-214) |
| 2 | 2026-11-03 | Arahan owner tambahan: pustaka open-source mature wajib, mekanisme sendiri dilarang. §3 ditulis ulang dengan kriteria K1–K7 dan bukti per kandidat. Pustaka diganti ke i18next + react-i18next + i18next-cli (MIT; react-intl gagal kriteria lisensi MIT/Apache-2.0). §2 I-1 sampai I-5, §3.4 format JSON v4 i18next (tanpa ICU), §4 tipe dan pemuatan, §5 contoh placeholder, §8 cek CI (`status`, `checkIdentical`, `types --ci`, `lint`), §9, §11 diselaraskan. Pseudo-locale dikeluarkan. Bagian lain tidak berubah. Final oleh Shu sebagai lead Engineering |
