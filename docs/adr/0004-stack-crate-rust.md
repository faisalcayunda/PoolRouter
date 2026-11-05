# ADR-0004: Stack crate Rust dan toolchain backend PoolRouter F0

**Ringkasan:** Keputusan pemilik produk K-32 (PRD v0.4.5; acuan terkini v0.4.6) menetapkan bahwa seluruh backend PoolRouter ditulis dalam Rust. Pemilihan framework dan crate diserahkan ke ADR. ADR-0001 sudah memilih axum (§8.1) dan pola port antrean (§6, §8.2). Crate pendukungnya masih berstatus "usulan; pelaksana boleh mengganti". ADR ini mengubahnya menjadi keputusan yang mengikat, berisi: daftar crate F0 dengan versi minimum dan alasannya, crate F2 yang ditandai terpisah, toolchain (edisi 2024, toolchain dan MSRV 1.99), satu penyedia kripto (`aws-lc-rs`), serta aturan menambah atau mengganti crate lewat PR yang direview Ridley. Rev 2 mengisi baris antrean dari hasil spike SHAA-177 (Lulus bersyarat): apalis + apalis-amqp tetap dipakai untuk konsumsi, publish lewat `lapin` langsung, `apalis-amqp` di-pin persis `=1.0.0`, dengan empat workaround adapter yang wajib, W1–W4 (§4.3; W5 ditangani reconciler ADR-0001). Rev 2 juga mengganti nomor dokumen ini dari ADR-0003 menjadi ADR-0004, karena nomor 0003 sudah dipakai ADR stack Web UI (SHAA-178/SHAA-183) yang dirujuk issue desain.

- **Pemilik:** Shu
- **Tanggal:** 2026-11-03
- **Status:** **final, rev 4** (oleh Shu sebagai lead, arahan owner 2026-11-03: Luca tidak ikut review project PoolRouter). Rev 4 menyelaraskan dengan ADR-0001 rev 7 §5a (riset SHAA-184) dan merapikan rujukan (Lampiran A SHAA-184); tidak ada crate baru. Rev 3 menyelaraskan dengan ADR-0003 rev 3. §4.3 antrean tetap seperti rev 2.
- **Nomor:** dokumen ini bernama ADR-0003 di rev 1. Nomor 0003 kini milik ADR stack Web UI (SHAA-178, rev 2 di SHAA-183). Rujukan "ADR-0003 rev 1 (crate)" dibaca sebagai dokumen ini.
- **Issue:** SHAA-165 (induk SHAA-162). Permintaan: Fitcher, SHAA-165, 2026-11-03.
- **Reviewer:** Shu (lead). Implementasi, `Cargo.toml`, `deny.toml`, dan CI direview Ridley.
- **Lokasi akhir:** `docs/adr/0004-stack-crate-rust.md` di akar repo `github.com/faisalcayunda/PoolRouter` (folder `docs/adr/` sesuai ADR-0003 §5; rev 2 keliru menulis `backend/docs/adr/`), di commit kerangka repo pertama. Sampai saat itu, lampiran rev 4 di SHAA-184 adalah salinan resmi (menggantikan lampiran rev 3 di SHAA-165).
- **Menggantikan:** paragraf "Default crate pendukung (usulan; pelaksana boleh mengganti …)" di ADR-0001 §8.1. ADR-0001 rev 5–6 §8.1 sudah merujuk dokumen ini.

## 1. Konteks

| Sumber | Isi yang mengikat |
|---|---|
| PRD v0.4.6 K-32 | Rust untuk webserver/API, controller/dispatcher, worker, dan adapter provider. Python hanya di function runner F2 (K-09). |
| K-16, K-17 | apalis 1.0 + RabbitMQ lewat `apalis-amqp` (klien `lapin`). Pesan hanya `request_id`. |
| K-25 | Kode transaksi UUIDv7 dibuat di Rust (`Uuid::now_v7()`). |
| K-31 | Hook `pre_upstream`, WASM, dan function runner masuk F2. Tidak ada kode atau stub di F0. |
| ADR-0001 | axum, port `JobPublisher`/`JobConsumer`, PostgreSQL sebagai sumber status, Redis fail-closed, envelope encryption AES-256-GCM, HMAC-SHA256 webhook, argon2id, JWT berumur pendek, webhook tanpa redirect dan dengan IP di-pin (W-06, W-07 contract v0.7.4). Rev 7 §5a: kuota efektif dan `pr:siap` di skrip Lua, full jitter, timeout per lapisan, aturan JWT, Standard Webhooks, RFC 9457, `no-store`, katalog metrik. |
| ADR-0002 D1, D5 | GET dengan body JSON. Trust TLS per provider `system`/`ca_kustom`/`pin_spki`, TLS ≥ 1.2 lewat rustls, satu HTTP client per provider, tanpa mode yang mematikan verifikasi. |
| ADR-0003 rev 4 (Web UI, monorepo) | Spesifikasi OpenAPI di-commit di `backend/openapi/admin-api.yaml` dan `downstream-api.yaml`, lalu dicek CI (§4.2). Login Web UI memakai sesi opaque di cookie `HttpOnly` `__Host-pr_session`: token 256 bit, server hanya menyimpan SHA-256-nya, password argon2id, CSRF lewat `Origin`/`Sec-Fetch-Site` (§3.6) |
| `shaal-eng-standards` | Kontrak antar-komponen, build dan test di container Podman, review kode oleh Ridley. |

Versi di bawah diperiksa di crates.io pada 2026-11-03. Rust stable terbaru per 2026-11-03 adalah 1.99.1 (2026-10-13).

## 2. Toolchain

| Butir | Keputusan | Alasan |
|---|---|---|
| Edisi | **2024** untuk semua crate di workspace | Edisi terbaru. Resolver dependensi v3 (sadar MSRV) aktif secara bawaan. |
| Toolchain | **1.99.1**, dipin di `rust-toolchain.toml` (komponen `rustfmt`, `clippy`) | Satu versi untuk dev, CI, dan image build. Build reprodusibel. |
| MSRV | **1.99** (`rust-version = "1.99"` di `[workspace.package]`) | Ini aplikasi, bukan library, jadi MSRV = toolchain yang dipin. Batas bawah nyata dari dependensi: sqlx 0.9 butuh 1.94; wasmtime 50 (F2) butuh 1.97. |
| Image build | `rust` bertag **1.99.1** (varian distro ditetapkan Platform & SRE). Tidak boleh `latest`. | Sama dengan `rust-toolchain.toml`. Sesuai `shaal-eng-standards` §8. |
| Kenaikan toolchain | Lewat PR yang mengubah `rust-toolchain.toml`, `rust-version`, dan tag image sekaligus, direview Ridley. Dilakukan paling lambat sekali per kuartal, atau lebih cepat bila ada advisori keamanan atau dependensi wajib yang menuntut. | Mencegah toolchain tertinggal jauh, tanpa kejutan di tengah sprint. |
| Lint | `cargo fmt --check` dan `cargo clippy --all-targets -- -D warnings` wajib di CI | Standar minimum. |

## 3. Aturan dependensi

1. **Satu tempat deklarasi.** Semua dependensi pihak ketiga dideklarasikan di `[workspace.dependencies]` root. Crate anggota memakai `workspace = true`. `Cargo.lock` di-commit.
2. **Versi minimum** di tabel §4 ditulis sebagai batas bawah caret di `Cargo.toml` (mis. `"0.8.9"` berarti ≥ 0.8.9 dan < 0.9). Kenaikan patch dan minor lewat PR otomatis (Dependabot/Renovate), cukup lolos CI dan review biasa, tanpa mengubah ADR.
3. **Menambah dependensi langsung baru, mengganti crate di §4, atau menaikkan major/0.x-minor crate di §4** wajib lewat PR yang:
   - memperbarui tabel di dokumen ini (`docs/adr/0004-stack-crate-rust.md`) di PR yang sama;
   - menulis alasannya: kebutuhan, alternatif yang ditimbang, rilis terakhir, pemelihara, lisensi, advisori RustSec, dan MSRV;
   - direview **Ridley**. Pelaksana tidak memilih atau mengganti crate secara bebas.
   - Perubahan di kategori kripto/secret, TLS, atau antrean juga perlu persetujuan Shu di PR itu, karena menyentuh keputusan keamanan ADR-0001 §7, ADR-0002 D5, atau K-16. Penggantian apalis tetap keputusan owner (§4.3).
4. **Supply chain di CI:** `cargo deny check` (advisories, licenses, bans, sources) wajib lulus.
   - Lisensi yang diizinkan: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, serta OpenSSL dan ISC dari `aws-lc-rs`/`aws-lc-sys`. Lisensi lain lewat aturan 3.
   - Dilarang: `openssl-sys` dan `native-tls`, karena TLS hanya lewat rustls (ADR-0002 D5). Sumber hanya crates.io, tanpa dependensi git.
   - Duplikasi major `rustls`, `tokio`, `hyper`, atau `opentelemetry` dilarang.
5. **Larangan di kode lewat clippy** (`clippy.toml` `disallowed-methods`): `reqwest::ClientBuilder::tls_danger_accept_invalid_certs`, `…::danger_accept_invalid_certs`, dan `…::tls_danger_accept_invalid_hostnames`. Tidak ada jalur untuk mematikan verifikasi sertifikat, termasuk di dev (ADR-0002 D5 opsi A).
6. **Tanpa `unsafe`** di crate PoolRouter (`#![forbid(unsafe_code)]`), kecuali lewat PR dengan alasan dan review Ridley.

## 4. Daftar crate F0 (mengikat)

### 4.1 Inti layanan

| Kategori | Crate | Versi minimum | Fitur | Alasan |
|---|---|---|---|---|
| Runtime async | `tokio` | 1.54 | `rt-multi-thread`, `macros`, `time`, `signal`, `net` | Runtime de facto. Dipakai axum, sqlx, redis, reqwest, dan apalis, jadi satu runtime untuk semua. |
| | `tokio-util` | 0.7.20 | — | `CancellationToken` untuk graceful shutdown worker dan controller (spike S6). |
| Framework HTTP | `axum` | 0.8.9 | `json`, `macros` | Keputusan ADR-0001 §8.1. Satu stack tower dengan apalis dan tonic. |
| | `tower` | 0.5.3 | `timeout`, `limit` | Middleware bersama untuk HTTP dan antrean. |
| | `tower-http` | 0.7.1 | `trace`, `timeout`, `request-id`, `limit`, `sensitive-headers`, `set-header` | Request id, batas ukuran body, timeout (ADR-0001 §5a.4), penyamaran header `Authorization` di trace, dan `Cache-Control: no-store` di akar router (ADR-0001 §5a.8, rev 4). |
| OpenAPI | `utoipa` | 6.0 | `axum_extras`, `uuid`, `chrono`, `yaml` | **Code-first.** Spesifikasi dibangkitkan dari handler, jadi tidak pernah menyimpang dari implementasi. Hasilnya di-commit sebagai `backend/openapi/admin-api.yaml` dan `backend/openapi/downstream-api.yaml` (rev 3, ADR-0003 §4.2). `make -C backend openapi-check` membangkitkan ulang lalu menjalankan `git diff --exit-code`. Frontend membangkitkan tipe dari `admin-api.yaml` di commit yang sama. Keputusan ini menutup pilihan "code-first atau spec-first" yang ADR-0003 §4.2 serahkan ke pelaksana backend. |
| | `utoipa-axum` | 0.3 | — | Router yang mendaftarkan rute dan dokumentasi sekaligus. |
| Serialisasi | `serde` | 1.0.229 | `derive` | Standar. `#[serde(deny_unknown_fields)]` dipakai untuk input client (ADR-0002 D6). |
| | `serde_json` | 1.0.154 | — | Body JSON dan payload log. Deteksi kunci JSON ganda (ADR-0002 D6) dilakukan dengan deserializer yang menolak duplikat, bukan parser lain. |
| Validasi | `regex` | 1.13 | — | Aturan validasi regex per provider (US-03). Waktu linear, aman terhadap ReDoS. |
| Waktu | `chrono` | 0.4.45 | `serde`, `clock` | Didukung sqlx dan utoipa. Batas hari kuota Asia/Jakarta memakai `FixedOffset` +07:00 (tanpa DST), jadi tidak butuh `chrono-tz` (T-3). |
| UUIDv7 | `uuid` | 1.29 | `v7`, `serde` | `Uuid::now_v7()` untuk `kode_transaksi` per percobaan (K-25), `request_id`, dan `webhook_delivery_id`. Fitur `uuid` di sqlx memetakannya ke kolom UUID. |
| Cookie sesi admin (rev 3) | `axum-extra` | 0.12.6 | `default-features = false`; `cookie` | `CookieJar` untuk cookie sesi `__Host-pr_session` (ADR-0003 §3.6). Token sesi dibuat dan di-hash dengan `aws-lc-rs` (§4.4), dan disimpan di PostgreSQL tanpa crate sesi pihak ketiga. |
| Error | `thiserror` | 2.0 | — | Tipe error di crate library. |
| | `anyhow` | 1.0 | — | Hanya di `main` binary, tidak di crate domain. |

### 4.2 Data, Redis, dan HTTP keluar

| Kategori | Crate | Versi minimum | Fitur | Alasan |
|---|---|---|---|---|
| PostgreSQL | `sqlx` | 0.9.0 | `default-features = false`; `postgres`, `runtime-tokio`, `tls-rustls-aws-lc-rs`, `macros`, `migrate`, `uuid`, `chrono`, `json` | Async, kueri dicek saat kompilasi (`query!`). Compare-and-set ADR-0001 §3 ditulis sebagai SQL eksplisit, tanpa ORM yang menyembunyikan predikat. Mode offline (`.sqlx/` di-commit, `SQLX_OFFLINE=true` di CI). Migrasi dijalankan job deploy dengan `poolrouter_owner` (ADR-0001 §7.2), tidak pernah oleh aplikasi saat start. Satu pool per role database. |
| Redis | `redis` | 1.8 | `tokio-rustls-comp`, `connection-manager`, `script`, `aio` | Klien resmi. `ConnectionManager` menangani reconnect. `Script` menjalankan Lua token bucket GCRA, kuota, dan semaphore secara atomik (ADR-0001 §5). Error dipetakan ke fail-closed, bukan ditelan. |
| HTTP client ke provider dan webhook | `reqwest` | 0.13.5 | `default-features = false`; `rustls`, `json`, `http2` | Mendukung body pada GET (ADR-0002 D1), timeout per request, `redirect(Policy::none())` untuk webhook (W-06), `dns_resolver` kustom yang hanya mengembalikan IP yang lolos pemeriksaan (pin IP W-07), dan `tls_version_min(TLS_1_2)`. |
| TLS | `rustls` | 0.23.46 | `default-features = false`; `aws_lc_rs`, `std`, `tls12`, `logging` | Satu-satunya stack TLS (ADR-0002 D5). Per provider: `RootCertStore` sistem (`system`), CA kustom (`ca_kustom`), atau `ServerCertVerifier` pin SPKI SHA-256 (`pin_spki`), dirakit menjadi `ClientConfig` dan dipasang ke reqwest lewat `tls_backend_preconfigured`. |
| | `rustls-pki-types` | 1.15 | — | Tipe sertifikat dan SPKI untuk verifier pin. |

**Catatan risiko `tls_backend_preconfigured`.** Dokumentasi reqwest 0.13 menyebut API ini tidak dijamin stabil secara semver. Versi `rustls` di workspace harus sama dengan versi yang dipakai reqwest. Kalau tidak sama, konfigurasi ditolak saat runtime. Mitigasinya:
- pemakaiannya hanya di satu fungsi (`build_provider_client(tls_trust)`) di crate adapter provider;
- uji ADR-0002 nomor 6 dan 7 (contract U-42) wajib di CI, jadi setiap kenaikan reqwest atau rustls, termasuk minor, hanya lolos bila uji `system`/`ca_kustom`/`pin_spki` tetap lulus;
- `cargo deny` melarang dua major rustls (§3.4).

Kalau API ini dihapus, penggantinya adalah `hyper-util` + `hyper-rustls` di fungsi yang sama, lewat aturan §3.3. Tidak ada alternatif yang mematikan verifikasi.

**Catatan arsip (tambahan ARC-146, ADR-0006 §8 butir 3).** Tiga crate arsip masuk di sini lewat aturan §3.3:

| Kategori | Crate | Versi minimum | Fitur | Alasan |
|---|---|---|---|---|
| Format arsip | `parquet` | 60.1.0 | `default-features = false`; `arrow`, `zstd`, `snap`, `brotli`, `lz4`, `flate2-zlib-rs`, `simdutf8` | Format objek arsip (ADR-0006 §8 butir 2): zstd, row group 128.000 baris, bloom filter pada `nik_hmac`, dan rentang baca per row group lewat `get_range`. Fitur `encryption` sengaja mati: DA10 membungkus `nik` dan kolom payload di aplikasi dengan AES-256-GCM, jadi tata letak kolom tidak bergantung pada versi Parquet modular encryption yang didukung crate ini. Fitur kompresi selain zstd dipertahankan karena berkas arsip lama harus tetap terbaca; `flate2-zlib-rs` dipilih supaya tidak ada `libz` sistem di image. Lisensi Apache-2.0 (sudah di allow list `deny.toml`). |
| Kolom dalam memori | `arrow` | 60.1.0 | `default-features = false`; `prettyprint` | Tipe kolom yang dipakai `parquet` versi yang sama; versi harus sejalan dengan `parquet` supaya `ArrowWriter`/`ArrowReader` tidak menarik dua versi `arrow-*`. Lisensi Apache-2.0. |
| Object storage | `object_store` | 0.14.2 | `aws` | S3-compatible untuk lane arsip, termasuk OBS Huawei lewat API S3 (ADR-0006 §8 butir 3). Dipakai hanya di implementasi `PenyimpanArsip`; port tidak membocorkan tipe crate ini. Lisensi MIT/Apache-2.0. |

Risiko versi `rustls` (§3.4) sudah diperiksa: `object_store` 0.14.2 menarik `reqwest` 0.13.5 yang sudah ada di workspace, sehingga `rustls` tetap satu versi, **0.23.46**. Karena itu tidak ada penghentian kerja ke Shu untuk butir ini.

### 4.3 Antrean (final, spike SHAA-177: Lulus bersyarat)

| Kategori | Crate | Versi | Fitur | Alasan |
|---|---|---|---|---|
| Antrean (adapter `queue-apalis`), sisi consume | `apalis` | 1.0.0 | `default-features = false`; `tracing`. Tanpa layer `retry` dan `timeout`. | K-16. Retry bawaan apalis hanya aktif bila layer `.retry()` dipasang, jadi layer itu dilarang (spike S5). Retry dikendalikan PoolRouter lewat PostgreSQL (ADR-0001 §5). |
| | `apalis-amqp` | **`=1.0.0` (pin persis)** | — | K-16. Pin persis karena workaround W2 bergantung pada perilaku internal `poll_ready`/`poll_next` versi ini. Naik versi hanya lewat PR yang menjalankan ulang suite kontrak antrean dan S1, S3, S7. |
| Antrean, sisi publish dan topologi | `lapin` | sama dengan yang dipakai `apalis-amqp` (^4.12, spike: 4.12.3) | `default-features = false`, `tokio`; TLS ke broker lewat rustls dengan `aws-lc-rs` (§4.4), diverifikasi di PR pertama | Publish native `apalis-amqp` 1.0.0 tidak durable (W1). Satu versi `lapin` di dependency tree. Tipe apalis dan lapin tidak keluar dari crate adapter (ADR-0001 §6). |
| Lisensi turunan | `amq-protocol` | mengikuti `lapin` (spike: 10.6.3) | — | BSD-2-Clause, sudah termasuk daftar lisensi yang diizinkan (§3 aturan 4). Tidak perlu pengecualian. |

**Workaround wajib di adapter `queue-apalis`** (bukti: laporan spike SHAA-177, `report.md` W1–W4):
1. **W1, publish lewat `lapin` langsung:** `confirm_select` per channel publish, `delivery_mode = 2`, `mandatory = true`. `JobPublisher::publish` sukses hanya untuk `Confirmation::Ack(None)`. `Ack(Some(_))` (unroutable), `Nack`, dan `NotRequested` adalah error. Jalur `push()`/`Sink` apalis dilarang untuk publish.
2. **W2, panggil `Sink::poll_ready` sekali** sebelum worker consume-only dijalankan. Tanpa ini `poll_next` panic (`apalis-amqp` 1.0.0 `lib.rs:325`).
3. **W3, loop reconnect di luar `Worker::run`** dengan backoff, yang membangun ulang koneksi, topologi, backend, dan worker sampai sinyal shutdown. Probe liveness worker bernilai gagal selama loop belum tersambung.
4. **W4, `nack_options.requeue = true`** untuk `HandlerError::Transient`, dan DLX/DLQ dideklarasikan adapter sendiri dengan argumen queue yang sama di semua deklarasi. `Config::max_retries` tidak dipakai (tidak dibaca `apalis-amqp` 1.0.0). `Config::quorum(false)` tidak dipakai; tipe queue diatur eksplisit lewat `declare_arguments`.

Workaround W5 (redelivery ke baris `in_flight`) bukan urusan adapter; sudah ditangani reconciler ADR-0001 §5 (lihat ADR-0001 §6).

**Pemicu peninjauan ulang** (Shu membuka ADR ini kembali bila salah satu terjadi): (a) cacat baru di `apalis-amqp` yang membuat suite kontrak gagal dan butuh workaround kelima di adapter; (b) `apalis-amqp` tanpa rilis atau commit selama 90 hari, padahal W1 atau W2 belum diperbaiki upstream; (c) upgrade `lapin` atau `tokio` terhalang pin `apalis-amqp`. Pada pemicu itu opsi pengganti adalah adapter `queue-lapin` (`lapin` langsung di balik port yang sama, perkiraan spike 2–4 hari kerja). Karena membalik K-16, usulannya diajukan Shu ke Fitcher (lead project) untuk keputusan owner, sesuai rev 1.

### 4.4 Kripto dan secret

| Kategori | Crate | Versi minimum | Fitur | Alasan |
|---|---|---|---|---|
| Penyedia kripto | `aws-lc-rs` | 1.19 | — | **Satu penyedia kripto untuk seluruh backend.** Penyedia ini sudah menjadi default rustls 0.23 dan dipakai sqlx (`tls-rustls-aws-lc-rs`), reqwest (`rustls`), dan jsonwebtoken (`aws_lc_rs`). Dipakai langsung untuk AES-256-GCM envelope encryption secret provider dan secret webhook (ADR-0001 §7, contract W-03), HMAC-SHA256 tanda tangan webhook, SHA-256 hash API key dan pin SPKI, serta RNG untuk data key dan nonce. Lebih sedikit kode kripto di tree berarti lebih sedikit permukaan audit. |
| Hash client secret | `argon2` | 0.6 | `password-hash`, `zeroize` | argon2id (ADR-0001 §7). Parameter ditetapkan di konfigurasi, dengan uji waktu verifikasi. |
| Token OAuth2 | `jsonwebtoken` | 11.1 | `aws_lc_rs`, `use_pem` | JWT bertanda tangan berumur pendek untuk client credentials (ADR-0001 §7). Penyedia kripto sama. Algoritma diset eksplisit (`EdDSA` atau `ES256`), dan `none`/HS dengan kunci publik ditolak. Signing key dari secret manager (`${POOLROUTER_JWT_SIGNING_KEY}`). Rev 4: `Validation` diisi sesuai ADR-0001 §5a.5 (`iss`, `aud`, klaim wajib, leeway 60 s); kunci dipilih dari `kid`, dan `Validation` dibuat per kunci dengan algoritma kunci itu. |
| Penanganan secret di memori | `secrecy` | 0.10.3 | `serde` | `SecretBox`/`SecretString` tanpa `Debug`/`Display` yang menampilkan nilai (ADR-0001 §7, contract W-04). |
| | `zeroize` | 1.9 | `derive` | Plaintext secret, data key, dan HMAC di-zeroize setelah dipakai. |

Rev 4: angka acak untuk full jitter retry (ADR-0001 §5a.3) dan secret yang dibangkitkan server memakai `aws_lc_rs::rand::SystemRandom`, tanpa crate `rand`. Tanda tangan Standard Webhooks (ADR-0001 §5a.6) memakai HMAC-SHA256 `aws-lc-rs` yang sama dan `base64` (ditambah lewat §3.3 bila belum ada sebagai dependensi langsung). RFC 9457 cukup tipe `serde`, tanpa crate.

KEK produksi lewat KMS (T-5) masih terbuka. Kalau KMS dipanggil lewat HTTP API, panggilannya memakai `reqwest` yang sama. SDK KMS hanya ditambah lewat aturan §3.3 setelah T-5 dijawab. Dev memakai `${POOLROUTER_KEK}` dari Podman secret.

### 4.5 Observability

| Kategori | Crate | Versi minimum | Fitur | Alasan |
|---|---|---|---|---|
| Log dan span | `tracing` | 0.1.44 | — | Standar ekosistem. Dipakai axum, sqlx, apalis, dan reqwest. |
| | `tracing-subscriber` | 0.3.23 | `json`, `env-filter`, `fmt` | Log JSON terstruktur. Aturan NIK di ADR-0001 §7.1 berlaku: tanpa body, tanpa URL hasil render, dan header sensitif disamarkan. |
| Trace | `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp` | 0.33 | `opentelemetry-otlp`: `grpc-tonic` atau `http-proto` (dipilih bersama Platform & SRE sesuai collector) | Ekspor OTLP ke collector. **Keempat crate OTel dinaikkan bersamaan (lockstep)**, karena 0.x minor saling tidak kompatibel. |
| | `tracing-opentelemetry` | 0.34 | — | Jembatan `tracing` ke OTel. Versi ini cocok dengan opentelemetry 0.33. |
| Metrik | `metrics` | 0.24 | — | Facade metrik dengan label terkontrol (tanpa NIK, `request_id`, atau IP; contract L-06 versi final terakhir). |
| | `metrics-exporter-prometheus` | 0.18 | `http-listener` | Endpoint `/metrics` Prometheus di port admin terpisah. |

### 4.6 Uji (dev-dependencies)

| Crate | Versi minimum | Alasan |
|---|---|---|
| `proptest` | 1.12 | Property test skrip Lua rate limit dan algoritma pemilihan dispatcher (ADR-0001 §9). |
| `wiremock` | 0.6.5 | Stub provider dan penerima webhook di uji integrasi (hitung panggilan, 3xx, timeout). Uji end-to-end tetap memakai mock Dukcapil di Compose (ADR-0002 §5). |

PostgreSQL, Redis, dan RabbitMQ untuk uji integrasi dijalankan sebagai container Podman dengan tag tetap, sesuai `shaal-eng-standards` §8. Crate pengelola container untuk uji ditambah lewat aturan §3.3 bila dibutuhkan.

## 5. Crate F2 (ditandai terpisah, tidak masuk F0)

Tidak ada kode, stub, atau dependensi crate di bawah ini di F0 (K-31). Versinya ditetapkan ulang saat F2 dimulai, lewat aturan §3.3.

| Kategori | Crate | Versi saat ini (2026-11-03) | Catatan |
|---|---|---|---|
| gRPC ke function runner Python (K-09) | `tonic`, `tonic-prost`, `prost` | 0.14 | Satu stack tower dengan axum. MSRV 1.88. |
| Runtime plugin WASM (K-09) | `wasmtime` | 50.0 | Rilis major bulanan. Pilih versi LTS yang berlaku saat F2. Butuh MSRV 1.97, masih di bawah toolchain 1.99. |
| Cache respons (K-01, F2) | tanpa crate baru | — | Memakai `redis` §4.2. |

## 6. Opsi yang ditimbang

| Keputusan | Dipilih | Ditolak | Alasan penolakan |
|---|---|---|---|
| Akses PostgreSQL | sqlx | diesel, sea-orm, tokio-postgres | Diesel dan sea-orm menyembunyikan SQL. Predikat compare-and-set dan grant per kolom (ADR-0001 §3, §7.2) harus terlihat dan dicek terhadap skema nyata. tokio-postgres tidak punya pengecekan kueri saat kompilasi, migrasi, maupun pool bawaan. |
| HTTP client | reqwest + rustls | hyper-util + hyper-rustls langsung | reqwest sudah memberi timeout, JSON, redirect policy, dan resolver kustom. Hyper langsung butuh lebih banyak kode untuk hal yang sama. Hyper tetap menjadi jalur cadangan (§4.2). |
| Stack TLS | rustls | native-tls/OpenSSL | rustls memudahkan verifier pin SPKI per provider, tanpa dependensi OpenSSL sistem di image, dan sudah menjadi keputusan ADR-0002 D5. |
| Penyedia kripto | aws-lc-rs | ring, RustCrypto (`aes-gcm`, `hmac`, `sha2`) | Penyedia ini sudah default di rustls, sqlx, reqwest, dan jsonwebtoken. Memakai yang lain berarti dua penyedia di tree. Biayanya: build butuh compiler C (dan cmake di sebagian target), yang dipasang di image build Platform & SRE. |
| OpenAPI | utoipa (code-first) | spec-first + generator | Spec-first butuh generator server yang matang untuk axum dan rawan selisih manual. Code-first dengan cek diff di CI memberi satu sumber kebenaran. |
| Waktu | chrono | time, jiff | Didukung langsung oleh sqlx, utoipa, dan jsonwebtoken. Kebutuhan zona waktu cukup offset tetap. |
| Metrik | `metrics` + exporter Prometheus | OTel metrics | Lebih sederhana dan stabil untuk scrape Prometheus. Trace tetap lewat OTel. |

## 7. Konsekuensi

- **Lebih mudah:** hand-off SHAA-175 punya satu daftar crate yang mengikat. Review dependensi oleh Ridley punya acuan. Satu penyedia kripto dan satu stack TLS mempersempit audit keamanan.
- **Lebih sulit atau perlu dikerjakan:**
  - toolchain 1.99 dan sqlx 0.9 (MSRV 1.94) menuntut image build yang baru;
  - aws-lc-rs butuh compiler C di image build;
  - API TLS reqwest yang tidak dijamin stabil wajib dijaga uji U-42;
  - crate OTel harus dinaikkan lockstep;
  - `cargo deny` dan clippy `disallowed-methods` harus disiapkan di PR pertama oleh pelaksana, sebagai pekerjaan Platform & SRE atau Flik sesuai pembagian SHAA-175.
- **Ketergantungan:** §4.3 berdasarkan spike SHAA-177. Penggantian apalis tetap keputusan owner (K-16). Spike memakai Rust 1.90, sqlx 0.8.6, dan reqwest 0.12; kompatibilitas satu binary (S9) dengan toolchain 1.99 dan sqlx 0.9 diverifikasi ulang di PR pertama (§8).

## 8. Cara verifikasi

| Butir | Bukti di PR pertama repo backend |
|---|---|
| Toolchain | `rust-toolchain.toml` = 1.99.1; `rust-version = "1.99"`; edisi 2024; tag image `rust:1.99.1-…` |
| Daftar crate | `[workspace.dependencies]` sama dengan §4. `cargo tree -d` tanpa duplikasi major yang dilarang |
| Supply chain | `cargo deny check` lulus di CI; `openssl-sys` dan `native-tls` tidak ada di `cargo tree` |
| Larangan verifikasi TLS | `clippy.toml` `disallowed-methods` aktif; uji ADR-0002 nomor 6–7 (U-42) lulus |
| OpenAPI | `backend/openapi/*.yaml` di-commit; `make -C backend openapi-check` gagal bila berbeda (ADR-0003 §4.2) |
| sqlx offline | `.sqlx/` di-commit dan CI build dengan `SQLX_OFFLINE=true` |
| Antrean (§4.3) | `apalis-amqp = "=1.0.0"`; `cargo tree -d` hanya satu `lapin`, satu `tokio`, satu `hyper`; suite kontrak antrean dari spike (ok → ack sekali, transient → kirim ulang, permanent → tepat satu di DLQ) lulus di CI terhadap RabbitMQ container; uji publish dengan broker restart memberi 0 pesan hilang; tidak ada pemanggilan `push()` apalis di kode |

## Rujukan

- PRD PoolRouter v0.4.6 final (lampiran SHAA-162): K-09, K-16, K-17, K-25, K-31, K-32, §10, risiko 6.
- ADR-0001 rev 7 (SHAA-165): §3, §5, §5a, §6, §7, §7.2, §8.1, §8.2, §8.3, §9. ADR-0003 stack Web UI rev 4 (SHAA-184; rev 3 di SHAA-183): §5 struktur monorepo.
- ADR-0002 rev 5 (SHAA-165): D1, D5, D6; uji 6–7. Data contract SHAA-166 v0.7.4 final: W-03, W-04, W-06, W-07, U-42, L-06.
- SHAA-177: spike apalis-amqp, laporan `report.md` dan arsip `spike-shaa-177.tar.gz` (Flik, 2026-11-03). SHAA-175: hand-off implementasi F0.
- Versi crate: crates.io, diperiksa 2026-11-03. Rust stable 1.99.1: channel `stable` rust-lang.org, diperiksa 2026-11-03.
- `shaal-eng-standards` §8 (container, tag tetap).

## Riwayat revisi

| Rev | Tanggal | Perubahan |
|---|---|---|
| 1 | 2026-11-03 | Dokumen pertama, atas permintaan Fitcher (SHAA-165) untuk K-32. Final oleh Shu sebagai lead, kecuali §4.3 (antrean) yang bersyarat pada SHAA-177. Menggantikan paragraf crate "usulan" di ADR-0001 §8.1 |
| 2 | 2026-11-03 | §4.3 antrean final dari spike SHAA-177 (Lulus bersyarat): consume lewat apalis + `apalis-amqp` `=1.0.0`, publish lewat `lapin` langsung, workaround W1–W4 wajib, pemicu peninjauan ulang, `amq-protocol` BSD-2-Clause tercakup §3. Verifikasi antrean di §8. Nomor diganti ADR-0003 → ADR-0004 karena bentrok dengan ADR stack Web UI (SHAA-178); lokasi akhir mengikuti monorepo. Baris lain tidak berubah |
| 3 | 2026-11-03 | Penyelarasan dengan ADR-0003 rev 3: OpenAPI code-first di-commit sebagai `backend/openapi/*.yaml` (fitur `yaml` utoipa) dengan `openapi-check`, menggantikan `openapi.json`; `axum-extra` (`cookie`) untuk sesi admin §3.6; lokasi dokumen dikoreksi ke `docs/adr/` di akar repo. Spike memakai reqwest 0.12; kenaikan ke reqwest 0.13 ikut diverifikasi ulang bersama cek S9 di PR pertama. §4.3 tidak berubah |
| 4 | 2026-11-03 | Penyelarasan dengan ADR-0001 rev 7 §5a (riset SHAA-184): fitur `set-header` `tower-http` untuk `no-store`; aturan `Validation` JWT per `kid`; angka acak jitter dan secret lewat `aws-lc-rs` tanpa crate baru. Lampiran A SHAA-184: rujukan ke ADR-0002 rev 5, PRD v0.4.6, contract v0.7.4, dan ADR-0003 rev 4; §3 aturan 3 menunjuk dokumen ini, bukan `0003`; ringkasan menyebut empat workaround adapter W1–W4. §4.3 tidak berubah |
