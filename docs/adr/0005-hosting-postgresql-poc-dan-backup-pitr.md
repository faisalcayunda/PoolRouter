# ADR-0005: Hosting PostgreSQL PoC dan backup/PITR (R-08)

**Ringkasan:** PoC PoolRouter butuh PostgreSQL 17 dengan PITR 7 hari, backup terenkripsi, RTO ≤ 4 jam, dan RPO ≤ 300 detik [asumsi] (R-08). **Keputusan: opsi B, PostgreSQL 17 di kontainer dengan pgBackRest**, repo backup terenkripsi di storage yang terpisah dari host DB. Opsi A (Huawei RDS for PostgreSQL 17) tidak dipilih karena tidak satu pun syarat 1–4 terkonfirmasi, dan aturan §4.1 tidak mengizinkan opsi A dipilih berdasarkan dugaan. Keputusan diambil atas arahan owner 2026-11-04 untuk melanjutkan tanpa menunggu jawaban BGN, bukan atas jawaban BGN; pertanyaan 1–8 menjadi asumsi terbuka dengan pemicu tinjau ulang di §6. Pekerjaan lanjutan: issue implementasi backup dan alat drill, lalu issue alert rule, keduanya di squad Platform & SRE.

- **Pemilik:** Shu
- **Tanggal:** 2026-11-04
- **Status:** final (Accepted, revisi 4).
- **Issue:** SHAA-211 (induk SHAA-175). Desain acuan: SHAA-206.
- **Difinalkan oleh:** Shu, sebagai leader squad Engineering (SHAA-180)

## 1. Konteks

Kebutuhan R-08 berasal dari ADR-0001 rev 15 §5a.10, data contract v0.7.11 §8.4, dan desain SHAA-206 revisi 2 (final, approve Huan dan Shu 2026-11-03):

| Kebutuhan | Nilai | Sumber |
|---|---|---|
| Objek backup | PostgreSQL saja. Redis dan RabbitMQ dibangun ulang dari PostgreSQL | ADR-0001 §5a.10 |
| PITR | Ke timestamp sembarang dalam rentang ≥ 7 hari | ADR-0001 §5a.10, SHAA-206 §2 |
| Enkripsi | At rest (KMS atau secret manager BGN) dan TLS saat transit | ADR-0001 §5a.10, SHAA-206 §4 |
| RTO | ≤ 4 jam, dari T0 (keputusan pulih) sampai T1 (health check PoolRouter dan query lewat DB hasil restore lolos) | SHAA-206 §2 |
| RPO | ≤ 300 detik **[asumsi]** | SHAA-206 §2 |
| Retensi backup | Tidak boleh melebihi retensi online (90 hari, usulan Q-02) | Contract §8.4 |
| Kelas backup | Rahasia. Agent Shaal tidak punya akses | Contract §8.4 B-02 |
| Restore drill | Satu kali sebelum NIK asli dipakai (gerbang PoC K-40). Lulus hanya jika B-03, B-04, DQ-05, dan DQ-06 terbukti | ADR-0001 §5a.10, contract §8.4 |

Keputusan yang sudah diambil di SHAA-206 (catatan keputusan Shu) dan berlaku untuk opsi B:
- Tool: **pgBackRest**.
- `archive_timeout = 60`, `archive-async=y`, `compress-type=zst`. Jika volume repo melebihi anggaran, nilai `archive_timeout` boleh dinaikkan ke 120 detik, tidak lebih.
- Retensi berbasis waktu: `repo1-retention-full-type=time`, `repo1-retention-full=7`.
- Jadwal: full harian 01.00 WIB, diferensial tiap 6 jam, arsip WAL terus-menerus.

Fakta vendor yang sudah dicek (dokumentasi publik Huawei Cloud, belum diverifikasi ke lingkungan BGN):
- Backup otomatis RDS for PostgreSQL disimpan di OBS dan tidak memakai storage instance. Retensinya mengikuti kebijakan backup otomatis yang dikonfigurasi.
- RDS menyediakan ruang backup gratis kira-kira sebesar storage yang dibeli. Pemakaian di atas itu ditagih bertingkat per GB.
- PITR tersedia di tingkat instance. Halaman resmi tidak menyebut interval backup log (WAL), batas rentang PITR, atau rentang retensi yang diizinkan. Nilai ini ditanyakan ke BGN (pertanyaan 2a–2d).

Batasan: kedua opsi sama-sama bergantung pada BGN: opsi A pada ketersediaan dan kemampuan RDS, opsi B pada storage terpisah dan secret manager. Rev 1–3 menahan keputusan sampai jawaban BGN masuk. Pada 2026-11-04 06.07 UTC owner menyatakan surat sudah dikirim dan meminta pekerjaan dilanjutkan tanpa menunggu BGN (SHAA-211). Arahan itu bukan jawaban per nomor: tidak ada fakta baru dari BGN tentang RDS PG17, PITR, interval arsip, enkripsi, atau storage. Keputusan di bagian 4 karena itu diambil dengan aturan §4.1 apa adanya, dan pertanyaan 1–8 dicatat sebagai asumsi terbuka (§4.4).

## 2. Opsi

| Aspek | Opsi A: Huawei RDS for PostgreSQL 17 | Opsi B: PostgreSQL 17 di kontainer + pgBackRest |
|---|---|---|
| Backup dan PITR | Fitur layanan (backup otomatis dan PITR instance) | pgBackRest: full harian, diferensial tiap 6 jam, arsip WAL async |
| RPO | Bergantung pada interval backup log RDS, **belum diketahui** | `archive_timeout = 60` memberi ruang sekitar 4 menit terhadap batas 300 detik |
| Bukti RPO saat drill | Skenario (b) "kehilangan host" tidak bisa memakai `kill -9`. Jika vendor tidak punya prosedur pengganti, RPO tidak bisa dinyatakan lulus (SHAA-206 §8) | Skenario (b) bisa diuji penuh (SHAA-206 §7.3) |
| Enkripsi | KMS BGN untuk backup; TLS dari layanan (2d, 4) | AES-256-CBC di repo; passphrase dari secret manager BGN lewat `${PGBACKREST_REPO1_CIPHER_PASS}`; TLS `ssl = on` |
| Restore ke instance baru | Lewat console atau API (2d, 7) | Kontainer baru di volume kosong (SHAA-206 §6) |
| Beban operasional | Rendah: patch, storage, dan backup dikelola layanan | Lebih tinggi: patch image, kapasitas repo, `pgbackrest verify`, alert |
| Ketergantungan | Vendor dan kebijakan akun BGN | Storage terpisah dan secret manager dari BGN |
| Kesesuaian dengan ADR-0001 | Sesuai default §5a.10 dan PRD §10 | Menggantikan butir default §5a.10 (konsekuensi 5.3) |

## 3. Estimasi biaya

### 3.1 Asumsi ukuran

Angka ini perkiraan Shu untuk perencanaan, bukan target. Semua angka bertanda [asumsi] diukur saat implementasi.

| Parameter | S1: PoC awal | S2: batas atas |
|---|---|---|
| Trafik | 100.000 request/hari (10% K-22) [asumsi] | 1.000.000 request/hari (K-22 penuh) |
| Data log per hari (`full_payload`, ±3 KB per baris) | ±0,3 GB | ±3 GB (ADR-0001 §1) |
| Umur data online | 30 hari | 90 hari (retensi online, contract §8.4) |
| Ukuran DB (log × 1,3 untuk indeks dan tabel lain) [asumsi] | ±12 GB | ±350 GB |
| WAL mentah per hari (±3× tulis data) [asumsi] | ±1 GB | ±9 GB |
| Rasio kompresi zstd [asumsi] | 0,35 | 0,35 |

Kedua skenario memakai `full_payload`. Dengan `metadata_only`, ukurannya jauh lebih kecil.

Catatan `archive_timeout = 60`: saat trafik sepi, PostgreSQL tetap menutup paling banyak 1.440 segmen WAL 16 MB per hari. Sisa segmen yang ditutup paksa berisi nol, jadi setelah kompresi zstd ukurannya kecil. Perkiraannya ≤ 1 GB/hari [asumsi]. Volume repo per hari wajib diukur saat implementasi (butir non-blocking SHAA-206).

### 3.2 Storage repo 7 hari, opsi B

Retensi berbasis waktu 7 hari dengan full harian menyimpan sampai **8 backup full** beserta WAL 8 hari. Alasannya, satu full yang lebih tua dari 7 hari harus tetap ada agar titik pulih tertua tetap ≤ sekarang − 7 hari.

| Komponen | S1 | S2 (jadwal SHAA-206) | S2 (jadwal hemat, bagian 5.2) |
|---|---|---|---|
| Backup full | 8 × 12 × 0,35 ≈ 34 GB | 8 × 350 × 0,35 ≈ 980 GB | 2 × 350 × 0,35 ≈ 245 GB |
| Diferensial | ≈ 2 GB | ≈ 16 GB | ≤ 77 GB (8 diferensial terakhir) |
| WAL | 8 × ≤ 1 GB ≈ 8 GB | 8 × 3,2 ≈ 26 GB | 14 × 3,2 ≈ 45 GB |
| **Total** | **≈ 45 GB** | **≈ 1,0 TB** | **≈ 330–370 GB** |
| Disediakan (ruang cadangan 50%) | 70 GB | 1,5 TB | 550 GB |

Volume DB: S1 ≈ 20 GB, S2 ≈ 525 GB (ukuran DB × 1,5).

### 3.3 Komponen biaya per opsi

| Komponen | Opsi A | Opsi B |
|---|---|---|
| Compute DB | Instance RDS PG17. Untuk PoC, single instance 2 vCPU/8 GB [asumsi]: ±70 tulis/s pada S2 ringan untuk PG17 (ADR-0001 §1). Primary/standby tidak diperlukan untuk PoC karena RTO 4 jam ditutup restore. HA masuk produksi (F1, R-22) | Bagian dari lingkungan PoC BGN yang juga menjalankan PoolRouter. Tambahan sekitar 2 vCPU/8 GB [asumsi] |
| Storage DB | Storage RDS yang dibeli: S1 ≥ 20 GB, S2 ≥ 525 GB | Volume BGN: S1 20 GB, S2 525 GB |
| Storage backup | Gratis sampai ±100% storage yang dibeli, kelebihannya ditagih bertingkat. S1: kemungkinan 0 GB tertagih. S2: kemungkinan ±500 GB tertagih, jika RDS menyimpan full harian (belum diketahui) | Repo di volume atau OBS BGN yang terpisah: S1 70 GB, S2 550 GB–1,5 TB |
| Waktu implementasi (Platform & SRE) | ±1,5 hari kerja (kebijakan backup, verifikasi enkripsi dan TLS, varian runbook SHAA-206 §8) + alert ±0,5 hari (Shiro) | ±4 hari kerja (pgBackRest di image, konfigurasi, secret, stanza, job terjadwal, `verify`, exporter) + alert ±1 hari (Shiro) |
| Alat drill (berlaku untuk kedua opsi) | ±1 hari kerja: penulis sentinel, checksum, hitung RPO | Sama |
| Pemeliharaan | ±0,5 jam/minggu | ±2 jam/minggu (patch image, kapasitas repo, hasil `verify`) |

Rumusnya:

- Opsi A = harga instance × jam + harga storage × GB storage + harga backup × max(0, GB backup − GB storage).
- Opsi B = harga volume × GB volume DB + harga OBS × GB repo + compute tambahan, jika ditagih terpisah.

### 3.4 Estimasi biaya indikatif (batas atas, rev 3)

Harga region BGN belum terverifikasi, dan penanggung biaya baru diketahui dari jawaban pertanyaan 5 dan 8. Karena itu angka di bawah ini **batas atas indikatif**, bukan penawaran. Tujuannya satu: plafon bisa disetujui Luca lebih dulu (bagian 3.5), sehingga syarat 5 di bagian 4.1 langsung bisa dinilai saat jawaban BGN masuk, tanpa satu putaran lagi.

Unit harga yang dipakai, semuanya **[asumsi, belum terverifikasi untuk region Jakarta]** dan sengaja dibulatkan ke atas:

| Unit | Nilai dipakai | Dasar |
|---|---|---|
| Instance RDS PG17 single 2 vCPU/8 GB | USD 0,25/jam (±USD 183/bulan, 730 jam) | Contoh tagihan resmi CN-Hong Kong: 4 vCPU/32 GB primary/standby ±USD 0,95/jam untuk dua node. Kelas 2 vCPU/8 GB single diperkirakan ≤ seperempatnya; nilai dipakai dibulatkan ke atas |
| Storage SSD (RDS atau volume) | USD 0,146/GB-bulan (USD 0,0002/GB-jam) | Unit pay-per-use di dokumentasi harga RDS Huawei Cloud, tanpa nama region |
| Storage backup RDS di atas kuota gratis | USD 0,146/GB-bulan | Diasumsikan sama dengan storage SSD (pesimistis) |
| OBS Standard (repo pgBackRest) | USD 0,025/GB-bulan | Contoh AP-Singapore multi-AZ; angka umum USD 0,023 |
| Compute tambahan opsi B (2 vCPU/8 GB) | USD 120/bulan | Hanya jika BGN menagih terpisah; batas atas |
| Kurs | Rp16.500/USD [asumsi] | |

Biaya per bulan (dibulatkan ke atas):

| Skenario | Opsi A | Opsi B (tanpa compute tambahan) | Opsi B (dengan compute tambahan) |
|---|---|---|---|
| S1 PoC awal | Instance 183 + storage 20 GB ≈ 3 + backup ≈ 0 → **±USD 186 (±Rp3,1 jt)** | Volume 20 GB ≈ 3 + repo 70 GB OBS ≈ 2 → **±USD 5 (±Rp0,1 jt)** | **±USD 125 (±Rp2,1 jt)** |
| S2 batas atas, jadwal SHAA-206 | 183 + 525 GB ≈ 77 + backup ±500 GB ≈ 73 → **±USD 333 (±Rp5,5 jt)** | Volume 525 GB ≈ 77 + repo 1,5 TB ≈ 38 → **±USD 115 (±Rp1,9 jt)** | **±USD 235 (±Rp3,9 jt)** |
| S2, jadwal hemat (bagian 5.2) | Tidak berlaku | 77 + repo 550 GB ≈ 14 → **±USD 91 (±Rp1,5 jt)** | **±USD 211 (±Rp3,5 jt)** |

Waktu kerja (bagian 3.3) tidak dirupiahkan: dikerjakan agent Platform & SRE dan masuk estimasi token F0, bukan biaya infra.

Bacaan: selisih biaya kedua opsi kecil dibanding risiko. Opsi A lebih mahal di infra (±Rp3–5,5 jt/bulan) tetapi lebih murah di operasional. Opsi B hampir gratis di infra bila compute dan storage memakai kuota lingkungan PoC BGN. Biaya tidak mengubah aturan keputusan; yang menentukan tetap syarat 1–4.

### 3.5 Pengajuan plafon bersyarat ke Luca

Biaya ini baru menjadi biaya Shaal jika BGN menjawab pertanyaan 8 (atau 5) bahwa biaya ditanggung anggaran uji coba. Agar syarat 5 tidak menahan ADR satu putaran lagi, plafon diajukan sekarang secara bersyarat:

- **Plafon:** Rp6 jt per bulan untuk PostgreSQL PoC beserta storage backup, opsi mana pun, selama masa PoC.
- **Berlaku hanya jika** BGN menyatakan biaya masuk anggaran uji coba. Jika BGN yang menanggung, tidak ada biaya baru bagi Shaal dan pengajuan ini gugur.
- **Pemicu pengajuan ulang:** harga terverifikasi dari kalkulator Huawei Cloud atau dari BGN melebihi plafon, atau ukuran trafik PoC melampaui S2.
- Jika Luca tidak menyetujui plafon, syarat 5 dinilai tidak terpenuhi untuk opsi A, dan opsi B dipilih sesuai bagian 4.1.

**Keputusan Luca (2026-11-04, SHAA-211):** plafon Rp6 jt/bulan disetujui, hanya jika jawaban BGN atas pertanyaan 8 menyatakan biaya masuk anggaran uji coba BGN. Jika biaya ditanggung Shaal atau jawabannya tidak jelas, plafon tidak berlaku dan keputusan biaya kembali ke Luca. Plafon tidak boleh dinaikkan diam-diam.

## 4. Keputusan

**Opsi B: PostgreSQL 17 di kontainer dengan pgBackRest.** Konfigurasi mengikuti keputusan SHAA-206 yang dirangkum di §1, ditambah §4.5.

| Syarat opsi A (§4.1) | Status per 2026-11-04 | Penilaian |
|---|---|---|
| 1. BGN menyediakan RDS PG17 | Tidak ada jawaban | Tidak terpenuhi |
| 2. PITR timestamp sembarang ≥ 7 hari | Tidak ada jawaban; dokumentasi publik tidak menyebut batas rentang | Tidak terpenuhi |
| 3. RPO ≤ 300 detik (tafsir §4.2) | Tidak ada angka tertulis interval log | Tidak terpenuhi |
| 4. Enkripsi, TLS, restore ke instance baru | Tidak ada jawaban | Tidak terpenuhi |
| 5. Biaya disetujui Luca | Plafon bersyarat Rp6 jt/bulan disetujui Luca 2026-11-04, berlaku hanya bila jawaban 8 menyatakan biaya masuk anggaran uji coba BGN | Bersyarat, tidak relevan karena 1–4 gagal |

Satu syarat yang tidak terpenuhi sudah cukup untuk memilih opsi B. Aturan §4.3 ("keputusan menunggu") ditulis untuk kasus pertanyaan 1 dijawab "ya" dengan jawaban 2a–2d tidak jelas. Kasus itu tidak terjadi, dan arahan owner menggantikan kewajiban menunggu.

Alasan selain aturan:
- Opsi B satu-satunya opsi yang RPO-nya bisa dibuktikan penuh lewat skenario (b) SHAA-206 §7.3. Untuk opsi A, bukti RPO bergantung pada prosedur vendor yang belum diketahui.
- Opsi B bisa dibangun dan diuji sekarang di Compose lokal (profile `bundled-infra`, ADR-0001 Addendum A) tanpa akses ke lingkungan BGN. Hasilnya dibawa ke PoC sebagai image dan konfigurasi yang sama.
- Biaya infra opsi B paling rendah (§3.4). Biaya tambahannya ada di operasional Platform & SRE (±2 jam/minggu).

### 4.1 Aturan keputusan (ditetapkan di SHAA-206, tidak berubah)

Opsi A dipilih hanya jika kelima syarat ini terpenuhi:
1. BGN menyediakan RDS PG17 untuk PoC (pertanyaan 1).
2. PITR bisa ke timestamp sembarang dalam rentang ≥ 7 hari (2a, 2b).
3. Interval arsip log menjamin RPO ≤ 300 detik (2c).
4. Backup terenkripsi at rest, koneksi memakai TLS, dan restore ke instance baru tersedia (2d, 4, 7).
5. Biayanya disetujui Luca (8, bagian 3).

Jika satu syarat saja tidak terpenuhi, opsi B yang dipilih.

### 4.2 Tafsir syarat 3

Batas 300 detik mencakup interval arsip ditambah waktu unggah. Karena itu, interval backup log 5 menit **tidak memenuhi** syarat 3. Alasannya sama dengan penolakan `archive_timeout = 300` di SHAA-206: kasus terburuknya melewati batas. Syarat 3 terpenuhi jika interval ≤ 4 menit, atau jika BGN atau vendor menyatakan RPO ≤ 300 detik secara tertulis.

### 4.3 Jawaban parsial

- Jika pertanyaan 1 dijawab "tidak", opsi A gugur saat itu juga. Opsi B lalu hanya butuh jawaban 3, 4, dan 5 untuk bisa diputuskan. Jawaban 6 dan 7 baru dibutuhkan untuk jadwal drill, bukan untuk ADR.
- Jika pertanyaan 1 dijawab "ya" tetapi salah satu dari 2a–2d tidak jelas, keputusan menunggu. Opsi A tidak dipilih berdasarkan dugaan.
- Jika jawaban 2c berupa "sesuai dokumentasi vendor" tanpa angka, ADR menunggu angka tertulis. Angka itu tidak ditebak.

### 4.4 Asumsi terbuka (dari pertanyaan BGN yang belum dijawab)

| No. | Asumsi yang dipakai | Jika ternyata salah |
|---|---|---|
| 1, 2a–2d | Tidak dipakai, karena opsi A tidak dipilih | Jika BGN **mewajibkan** RDS dan melarang DB di kontainer: ADR baru menggantikan ADR ini (§6) |
| 3, 5 | BGN menyediakan volume atau OBS yang terpisah dari host DB untuk repo, dan mengizinkan backup disimpan di sana | Tanpa storage terpisah, syarat "kehilangan host" tidak terpenuhi. Gerbang drill R-08 tertahan sampai storage tersedia; tidak ada repo di host yang sama |
| 4 | AES-256-CBC pgBackRest dengan passphrase dari secret manager BGN, ditambah TLS `ssl = on`, memenuhi kebijakan enkripsi BGN | Jika BGN mensyaratkan KMS tertentu atau algoritma lain: konfigurasi disesuaikan di issue implementasi (enkripsi di sisi OBS/KMS sebagai lapisan tambahan). Tidak butuh ADR baru kecuali tool berganti |
| 6, 7 | Jadwal dan saksi drill ditetapkan saat PoC | Tidak memengaruhi ADR; hanya jadwal gerbang drill |
| 8 | Lingkungan PoC (compute, volume, OBS) disediakan BGN tanpa tagihan ke Shaal | Jika ditagihkan ke Shaal: batas atas ±Rp3,9 jt/bulan (§3.4) diajukan ke Luca **sebelum** resource ditagih. Plafon bersyarat Luca 2026-11-04 tidak otomatis berlaku untuk kasus ini |

Asumsi ini dikonfirmasi ke BGN saat persiapan PoC di lingkungan BGN, bersama informasi B-01 (§5.1).

### 4.5 Konfigurasi yang ditetapkan untuk implementasi

- Image PostgreSQL 17 dengan pgBackRest, tag dan digest dipin. `archive_command` memakai pgBackRest dengan `archive-async=y`; `archive_timeout = 60`; `compress-type=zst`.
- Repo: `repo1-type` dari variabel. `posix` di volume terpisah untuk dev; `s3` untuk OBS di lingkungan PoC. Repo tidak pernah berada di volume data DB.
- Enkripsi repo: `repo1-cipher-type=aes-256-cbc`, passphrase `${PGBACKREST_REPO1_CIPHER_PASS}`. Kredensial S3 lewat `${PGBACKREST_REPO1_S3_KEY}` dan `${PGBACKREST_REPO1_S3_KEY_SECRET}`. Tidak ada nilai rahasia di repo.
- Retensi: `repo1-retention-full-type=time`, `repo1-retention-full=7`, dibaca dari variabel dengan default 7. Lingkungan dev boleh menurunkannya untuk menghemat disk, tetapi nilai itu dicatat di bukti dan tidak berlaku untuk PoC.
- Jadwal: full harian 01.00 WIB, diferensial tiap 6 jam, `pgbackrest verify` minimal harian. Jadwal hemat §5.2 mengikuti ambangnya.
- Metrik untuk alert: status arsip WAL (`pg_stat_archiver`) dan status backup pgBackRest diekspos ke Prometheus lewat exporter dengan image dipin. Pilihan exporter ditetapkan pelaksana dan dicatat di issue implementasi.
- ADR ini di-commit sebagai `docs/adr/0005-hosting-postgresql-poc-dan-backup-pitr.md` oleh pelaksana issue implementasi.

## 5. Konsekuensi

### 5.1 Berlaku untuk kedua opsi

- **Retensi efektif: paling lama 7 hari ditambah satu interval backup full, bukan tepat 7 hari.** Agar PITR 7 hari tetap terjamin, satu full yang lebih tua dari 7 hari harus disimpan. Akibatnya, data yang sudah di-purge bisa bertahan di backup lebih lama dari 7 hari. Menurunkan retensi ke 6 hari akan merusak syarat PITR 7 hari, jadi itu bukan solusi. Rumusan contract sudah dikoreksi di **v0.7.11 §8.4 B-01** (Klaus, 2026-11-03), dan ADR ini mengikuti rinciannya:
  - (a) Jadwal SHAA-206 (full harian): ±8 hari.
  - (b) Jadwal hemat (full mingguan, bagian 5.2): ±14 hari.
  - (c) Interval dihitung antar-full yang **sukses**. Full yang gagal memperpanjang batas; kondisi ini dijaga alert "full terakhir yang sukses lebih tua dari 26 jam" (8 hari pada jadwal hemat).
  - (d) pgBackRest menjalankan expire setelah backup berikutnya sukses, jadi ada jeda tambahan paling lama satu interval backup (6 jam). Jeda ini diukur saat implementasi.
  - (e) Opsi A: tidak berlaku sejak rev 4, karena opsi B dipilih.
  - Semua batas di atas tetap di bawah retensi online 90 hari.
- Butir non-blocking masuk ke issue implementasi backup dan alat drill:
  - Checksum skenario (a) hanya membandingkan baris dengan `ts ≤ T_target` (SHAA-206).
  - Timestamp RPO memakai waktu commit server (`RETURNING clock_timestamp()`) yang dicatat klien, bukan jam klien (SHAA-206).
  - Interval sentinel skenario (b) 1 detik (SHAA-206).
  - Volume repo per hari diukur dengan `archive_timeout = 60` (SHAA-206).
  - Setelah backup berjalan lebih dari 8 hari, umur full tertua di repo diukur dengan `pgbackrest info` lalu dibandingkan dengan B-01 (a) dan (d). Jika melebihi ±8 hari + 6 jam, penyebabnya dicatat di issue implementasi dan Klaus diberi tahu untuk koreksi contract (contract v0.7.11 butir 38c).
- Informasi B-01 v0.7.11 disampaikan ke BGN saat persiapan PoC di lingkungan BGN, bersama keputusan ADR ini dan konfirmasi asumsi §4.4, lewat owner. Owner menyatakan urusan BGN tidak perlu dipikirkan tim sekarang (2026-11-04), jadi tidak ada surat baru yang disiapkan di F0. Surat pertanyaan v0.2 tidak diubah.
- Alat drill (penulis sentinel, checksum, hitung RPO) tidak bergantung pada pilihan opsi. Alat ini tidak dibuat lebih awal karena tidak ada di jalur kritis: drill penuh menunggu lingkungan PoC dan backup yang sudah berjalan 7 hari (SHAA-206 §5). Alat ini dibuat bersama issue implementasi.
- ADR ini difinalkan sebagai file `docs/adr/0005-hosting-postgresql-poc-dan-backup-pitr.md` di repo PoolRouter oleh pelaksana issue implementasi. Shu tidak meng-commit.

### 5.2 Opsi B (dipilih)

- Lebih mudah: RPO bisa diuji penuh lewat skenario (b), dan tidak ada ketergantungan pada fitur vendor yang belum terverifikasi.
- Lebih sulit: operasional ditanggung Platform & SRE, termasuk patch, kapasitas, `verify`, dan alert. Repo wajib berada di storage yang terpisah dari host DB.
- **Ambang jadwal hemat:** jika ukuran backup full terkompresi melebihi 50 GB (DB ±140 GB), atau repo melewati anggaran yang disetujui, jadwal diganti menjadi full mingguan + diferensial tiap 6 jam dengan `repo1-retention-diff=8`. Storage S2 turun dari ±1,0 TB menjadi ±350 GB. Dampaknya, restore memutar ulang diferensial dan WAL paling banyak 6 jam. RTO tetap diukur saat drill. **Konsekuensi retensi:** peralihan ke jadwal hemat menaikkan retensi efektif dari ±8 hari menjadi **±14 hari** (contract v0.7.11 B-01 (b)), sehingga informasi yang disampaikan ke BGN ikut berubah. Peralihan ini tidak butuh ADR baru, tetapi tidak boleh dilakukan diam-diam. Pelaksana wajib mencatatnya di issue implementasi sebelum jadwal berubah, dengan isi: tanggal berlaku, pemicu (ukuran full terkompresi atau anggaran repo), dan retensi efektif baru. Catatan itu me-mention Shu dan Klaus. Informasi B-01 untuk BGN diperbarui lewat jalur yang sama dengan penyampaian keputusan ADR ini.
- Pemantauan, dibuat Shiro sebagai alert rule: arsip WAL gagal atau tertunda > 5 menit; full terakhir yang sukses lebih tua dari 26 jam (atau 8 hari pada jadwal hemat); titik pulih tertua < 7 hari; `pgbackrest verify` gagal; repo terisi > 80%.
- Butir default "backup terkelola RDS" di ADR-0001 §5a.10 digantikan ADR ini. Revisi ADR-0001 berikutnya cukup merujuk ADR-0005.

### 5.3 Opsi A (tidak dipilih, disimpan sebagai rujukan bila ADR ini digantikan)

- Lebih mudah: beban operasional rendah, dan pilihan ini sesuai default ADR-0001 §5a.10.
- Lebih sulit: bukti RPO. Jika vendor tidak punya prosedur pengganti `kill -9`, RPO hanya dibuktikan dengan dua hal: pernyataan tertulis interval log (syarat 3), dan pemantauan selisih "waktu terbaru yang bisa dipulihkan" terhadap waktu sekarang, jika RDS menampilkannya lewat API. RPO tidak dinyatakan lulus hanya dari dokumen (SHAA-206 §8).
- Pemantauan, dibuat Shiro sebagai alert rule: backup otomatis gagal; selisih waktu terbaru yang bisa dipulihkan > 300 detik (jika tersedia); storage backup melewati ruang gratis.
- Retensi efektif backup RDS (contract B-01 (e)) ditetapkan dari jawaban pertanyaan 2. Jika lebih dari ±8 hari, angkanya dicatat di revisi Accepted dan diteruskan ke Klaus untuk contract.
- Pekerjaan opsi B (image pgBackRest, job terjadwal) tidak dibuat.

### 5.4 Pekerjaan lanjutan

- Issue implementasi backup sesuai opsi terpilih, termasuk alat drill dan butir non-blocking SHAA-206. Sub-issue K2 untuk squad Platform & SRE; reviewer runbook Huan; kode, Containerfile, dan konfigurasi direview Ridley.
- Issue alert rule untuk Shiro (squad Platform & SRE), direview Huan.
- Restore drill R-08 di lingkungan BGN tetap gerbang PoC (K-40), di luar hand-off F0. Issue implementasi menjalankan drill lokal (skenario a dan b dengan data sintetis) sebagai bukti awal; drill di lingkungan BGN tetap dibutuhkan sebelum NIK asli dipakai.
- Biaya: tidak ada biaya baru bagi Shaal saat ini. Dev berjalan di Compose lokal, dan lingkungan BGN di luar lingkup F0. Pengajuan ke Luca dipicu asumsi 8 (§4.4).

## 6. Risiko terbuka

- **Keputusan diambil tanpa jawaban BGN (arahan owner 2026-11-04).** Risiko terbesar: BGN ternyata mewajibkan RDS atau melarang DB di kontainer. *Pemicu tinjau ulang:* jawaban BGN atas pertanyaan 1, 3, 4, atau 5 bertentangan dengan asumsi §4.4. *Biaya balik arah:* pekerjaan khusus opsi B (image pgBackRest, job terjadwal, exporter) ±4 hari kerja menjadi tidak terpakai; alat drill, butir non-blocking SHAA-206, dan alert umum tetap terpakai. Biaya ini diterima karena menunggu juga menahan gerbang PoC tanpa batas waktu.
- **Storage terpisah dari BGN belum pasti (asumsi 3 dan 5).** Tanpa storage itu, skenario kehilangan host tidak bisa lulus dan drill R-08 tertahan.
- Angka ukuran S1 dan S2 adalah asumsi. Jika trafik PoC atau rasio kompresi jauh berbeda, §3 dihitung ulang dari ukuran repo yang diukur di issue implementasi.
- Harga di §3.4 memakai batas atas dari contoh region lain; diganti harga terverifikasi hanya jika asumsi 8 gagal.
- Jeda expire B-01 (d) masih perkiraan. Angka pastinya dari pengukuran `pgbackrest info` (§5.1).
- Disk lingkungan build terbatas (`/data` di instabox, SHAA-373). Drill lokal dan repo backup dev bersaing dengan gate F0; issue implementasi memakai cek ruang kosong yang sama dengan runbook SHAA-373.

## 7. Rujukan

- SHAA-206: desain backup dan PITR revisi 2 (`SHAA-206-desain-backup-pitr.md`), catatan keputusan Shu 2026-11-03.
- SHAA-211: surat pertanyaan BGN v0.2 (`SHAA-211-pertanyaan-BGN-backup-basis-data-v0.2.md`).
- ADR-0001 rev 15 §1 (angka ukuran), §5a.10 (lampiran SHAA-218). Data contract v0.7.11 §8.4 B-01–B-04 dan §11 butir 38a–38d (lampiran SHAA-211, Klaus 2026-11-03).
- Huawei Cloud, "How Long Does RDS for PostgreSQL Store Backup Data For?": https://support.huaweicloud.com/intl/en-us/rds-pg_faq/rds_faq_0043.html
- Huawei Cloud, "How Is RDS for PostgreSQL Backup Data Billed?": https://support.huaweicloud.com/intl/en-us/rds-pg_faq/rds_faq_0074.html
- Huawei Cloud, RDS for PostgreSQL "Billing Examples" (CN-Hong Kong): https://support.huaweicloud.com/intl/en-us/price-rds-pg/rds_00_0007.html
- Huawei Cloud, RDS "Pay-per-Use Billing": https://support.huaweicloud.com/intl/en-us/price-rds/rds_00_0005.html
- Huawei Cloud, OBS "Storage Space" dan "Billing Examples": https://support.huaweicloud.com/intl/en-us/price-obs/obs_42_0003.html, https://support.huaweicloud.com/intl/en-us/price-obs/obs_42_0031.html
- pgBackRest User Guide (retensi berbasis waktu, `archive-async`, `compress-type`): https://pgbackrest.org/user-guide.html

## Riwayat

| Rev | Tanggal | Perubahan |
|---|---|---|
| 1 | 2026-11-03 | Draf Proposed (K-40): konteks, opsi A dan B, aturan keputusan beserta tafsir syarat 3 dan aturan jawaban parsial, estimasi storage dan komponen biaya, konsekuensi, dan temuan B-01 (retensi efektif ±8 hari) |
| 2 | 2026-11-03 | Atas contract v0.7.11 (Klaus, SHAA-211): rujukan §1, §5.1, dan §7 ke v0.7.11; §5.1 mengikuti B-01 (a)–(e) dan menambah butir non-blocking pengukuran umur full tertua dengan `pgbackrest info` setelah > 8 hari; §5.1 menetapkan informasi B-01 disampaikan ke BGN bersama keputusan ADR; §5.2 mencatat retensi efektif ±14 hari pada jadwal hemat dan kewajiban mencatat peralihan jadwal di issue implementasi beserta tanggal berlakunya; §5.3 menambah penetapan B-01 (e) untuk opsi A; §6 risiko B-01 diperbarui. Aturan keputusan, opsi, dan estimasi biaya tidak berubah |
| 3 | 2026-11-04 | Jawaban BGN belum masuk. §3.3 rumus dipisah dari catatan; §3.4 baru: estimasi biaya indikatif batas atas per bulan untuk opsi A dan B (S1, S2, jadwal hemat) dengan unit harga [asumsi] dari dokumentasi Huawei Cloud region lain; §3.5 baru: pengajuan plafon bersyarat Rp6 jt/bulan ke Luca agar syarat 5 bisa dinilai langsung; §6 risiko status pengiriman surat dan harga. Aturan keputusan, opsi, dan konsekuensi tidak berubah |
| 4 | 2026-11-04 | **Final (Accepted).** Atas arahan owner 06.07 UTC untuk melanjutkan tanpa jawaban BGN: §4 diisi, opsi B dipilih dengan penilaian syarat 1–5; §4.4 asumsi terbuka per pertanyaan beserta akibatnya; §4.5 konfigurasi implementasi; §1 batasan diperbarui; §5.3 opsi A sebagai rujukan; §5.4 drill lokal dan pemicu biaya; §6 risiko keputusan tanpa jawaban BGN beserta pemicu tinjau ulang dan biaya balik arah. Plafon Luca (§3.5) dicatat sebagai keputusan bersyarat |
