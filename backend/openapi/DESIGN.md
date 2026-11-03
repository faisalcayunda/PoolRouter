# Desain kontrak OpenAPI PoolRouter F0 (downstream dan admin)

Pemilik: Flik. Issue: SHAA-175 (induk SHAA-162). Status: **draf untuk review desain**, belum dikompilasi.
Dokumen ini inventaris operasi sebelum handler `utoipa` ditulis. Setelah di-approve, `downstream-api.yaml` dan `admin-api.yaml` dibangkitkan dari kode (`openapi-gen`) dan harus cocok dengan dokumen ini.

## 0. Acuan yang dipakai

Body issue SHAA-175 masih menyebut data contract v0.7.7 dan ADR-0001 rev 11. Dokumen ini memakai versi yang lebih baru, berdasarkan perubahan acuan yang tertulis di body issue yang sama (bagian "Perubahan acuan: data contract v0.7.10 dan ADR-0001 rev 15", Fitcher, 2026-11-03, SHAA-218):

| Acuan | Versi dipakai | Sumber yang membuatnya berlaku |
|---|---|---|
| ADR-0001 | **rev 15** | Final Shu, lampiran SHAA-218 komentar `01a19ffd…`; sudah ada di repo (`docs/adr/0001-arsitektur-poolrouter.md`, commit `8904199`). Menggantikan rev 11 sampai 14 dan melipat errata E-1…E-4 serta keputusan SHAA-200 (502 `retry_habis` untuk sync) |
| Data contract | **v0.7.10** | Final Klaus, lampiran SHAA-218 komentar `01a1a000-8767…`. Menggantikan v0.7.7 sampai v0.7.9. Menurut body issue, v0.7.10 menyatukan cabang v0.7.8 tanpa kode HTTP, nilai status, `cancel_reason`, `failure_reason`, atau kolom baru dibanding v0.7.9. Perubahan v0.7.8→v0.7.10 yang menyentuh kontrak ini: sync `retry_habis` (§3.2.5) dan K-20 (§3.2.6). Saya tidak membandingkan v0.7.7 dengan v0.7.10 baris per baris. Rujukan "v0.7.7/v0.7.8/v0.7.9 §…" tetap sah. v0.8.0 ditarik |
| ADR-0003 | **rev 5** (§3.6 dan §3.6f) + **Addendum A rev 2** (i18n) | Body issue: "Perubahan scope A-01" (SHAA-196) dan "Tambahan i18n Web UI" (SHAA-214). **Catatan:** `docs/adr/0003-stack-web-ui.md` di repo masih rev 4; rev 5 dan Addendum A ada di lampiran SHAA-196 dan SHAA-214, belum masuk repo |
| ADR-0002 | rev 5 (D6) | Body issue; ADR-0002 di repo belum diperiksa ulang untuk dokumen ini |
| ADR-0004 | rev 4 | Metode `utoipa` 6.0 + `utoipa-axum` 0.3 (code-first), disetujui Shu |
| BRD/US | **v0.17** (§7) | Body issue (SHAA-188) |
| UX v0.3, handoff spec UI v0.2/v0.3 | tidak dibaca ulang untuk dokumen ini | Kesesuaian dengan layar UI dicek Fitcher saat review |

Aturan benturan dari body issue: bila PRD v0.4.7 dan ADR-0001 rev 11 berbeda soal nilai teknis, ADR-0001 berlaku; di dalam ADR-0001 rev 15, bagian 9 mengalahkan bila contract dan ADR berbeda.

Penanda kolom **Sumber** di bawah: **S** = tertulis di acuan (dengan rujukan), **U** = usulan saya yang tidak ditetapkan acuan manapun dan perlu keputusan di review (daftar di §7).

## 1. Aturan lintas operasi

| Aturan | Isi | Sumber |
|---|---|---|
| Versi dan prefix | Downstream `/v1/…`, admin `/admin/v1/…`. Setelah SHAA-175 selesai, perubahan breaking di `downstream-api.yaml` dilarang tanpa versi API baru dan keputusan Shu | Body issue |
| Format error | Semua 4xx/5xx: `application/problem+json` RFC 9457 dengan `type`, `title`, `status`, `kode`, opsional `detail`. `type` = `https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#<kode>`. `detail` tidak pernah memuat nilai input, NIK, atau secret | R-10, ADR-0001 §5a.7 |
| Error 422 | `errors: [{field, kode, batas?}]`; `batas` hanya parameter angka aturan. Tidak ada nilai input | Addendum A §6.1; ADR-0002 D6 |
| Respons 429 | Selalu `Retry-After` (detik bulat, naik, minimal 1) dan `kode` dari `RateLimitKode`. `penolakan_berlebih` hanya memuat `type`, `title`, `status`, `kode` | ADR-0001 §5a.7; contract L-05 |
| `Cache-Control: no-store` | Pada **semua** respons kedua API, termasuk error dan 204. Dipasang satu layer di akar router, jadi spec mendeklarasikannya sebagai header respons komponen yang dirujuk semua operasi | R-16, ADR-0001 §5a.8 |
| `Idempotency-Key` | Hanya `enqueueAsync`. Opsional, ≤128 karakter ASCII, tanpa data pribadi. Dipakai sync → 400 | ADR-0001 §4.1 |
| Tipe | `request_id` = `queue_id` = UUIDv7; waktu `date-time` UTC | ADR-0001 §4.1 |
| Input ketat | Body request `additionalProperties: false` di level amplop. Isi `input` dicocokkan runtime dengan kontrak input endpoint (OpenAPI statis tidak bisa mengekspresikannya): nama persis, field tak dikenal → 422, kunci JSON ganda dan JSON tidak valid → 400 | ADR-0002 rev 5 D6; US-03 |
| Skema keamanan downstream | `oauth2` clientCredentials (`tokenUrl: /v1/oauth/token`, TTL 15 menit, tanpa refresh token) dan `apiKey` di header. **Nama header API key tidak ditetapkan acuan**; usulan `X-API-Key` (U-2) | R-05, ADR-0001 §5a.5 |
| Skema keamanan admin | Cookie `__Host-pr_session` (`HttpOnly; Secure; SameSite=Strict`). Method selain GET/HEAD memeriksa `Origin` = `${POOLROUTER_UI_ORIGIN}` dan `Sec-Fetch-Site`; gagal → 403 | ADR-0003 rev 5 §3.6 |

### 1.1 Daftar `kode` (enum `ProblemKode`)

Downstream, dari acuan: `validasi_gagal`, `metadata_tidak_valid`, `idempotency_key_konflik`, `kuota_habis`, `porsi_sync_habis`, `penolakan_berlebih`, `objek_nonaktif` (+ `jenis_objek` ∈ `endpoint|downstream_pool|upstream_pool|provider`), `tidak_bisa_dibatalkan`, `layanan_tidak_tersedia`, `retry_habis`, `provider_error_permanen`, `timeout_sync`. Usulan (U-1): `tidak_terautentikasi` (semua 401, supaya penyebab tidak bocor; rincian `alasan` ada di metrik MT-03), `tidak_ditemukan` (404), `idempotency_key_tidak_didukung` (sync + header, 400), `token_dibatasi` (429 endpoint token).

Admin: daftar lengkap menjadi enum `ProblemKode` di `admin-api.yaml` (Addendum A §6.1). Dari acuan: `ekspor_terlalu_besar` (ADR-0003 §3.6d). Usulan: `kredensial_salah`, `login_dibatasi`, `origin_tidak_valid`, `akses_ditolak`, `pencarian_nik_dilarang`, `validasi_gagal`, `secret_dipakai`, `kredensial_dicabut`, `batas_kredensial`, `webhook_domain_tidak_diizinkan`, `webhook_tidak_https`, `metadata_wajib_hilang`, `tidak_ditemukan`. Enum final ditutup saat compile pertama karena `Problem.kode` di kode masih `String`.

## 2. Downstream API (`downstream-api.yaml`)

Semua operasi (kecuali `issueToken`) memakai `oauth2` atau `apiKey`. Respons umum di setiap operasi bertanda auth: 401 `tidak_terautentikasi` (tidak disimpan, tanpa baris `request`, tanpa kuota) dan 503 `layanan_tidak_tersedia` + `Retry-After: 5` (Redis fail-closed). Keduanya tidak diulang di tabel.

| operationId | Method dan path | Auth | Request | Respons per status | Sumber |
|---|---|---|---|---|---|
| `issueToken` | `POST /v1/oauth/token` | HTTP Basic atau body `client_id`/`client_secret` (U-3) | `application/x-www-form-urlencoded`: `grant_type=client_credentials` | 200 `TokenResponse{access_token, token_type: "Bearer", expires_in: 900}`; 400 `validasi_gagal` (`grant_tidak_didukung`, `permintaan_tidak_valid`); 401 `tidak_terautentikasi`; 429 `token_dibatasi` + `Retry-After` (GCRA 60/menit per `client_id`, burst 10; 120/menit per IP) | S: R-05. Format error problem+json menyimpang dari RFC 6749 §5.2; lihat U-3 |
| `getEndpointKontrak` | `GET /v1/endpoints/{endpoint_id}/kontrak` | token/API key | Path `endpoint_id` | 200 `EndpointKontrak{endpoint_id, mode_diizinkan[], field_input[{nama, tipe, wajib}], field_metadata[{nama_field, tipe, wajib, nilai_diizinkan?, deskripsi, versi}], endpoint_config_version}`; 404 `tidak_ditemukan` (endpoint bukan milik client) | S: US-15 (isi dari contract §4); U: path dan bentuk (U-4) |
| `submitSync` | `POST /v1/endpoints/{endpoint_id}/sync` | token/API key | `SubmitRequest{input: object, metadata: object}` (`additionalProperties: false` di amplop). Header `Idempotency-Key` **tidak boleh** ada | 200 `SyncResult{request_id, status: "done", hasil}`; 400 `validasi_gagal` (JSON tidak valid / kunci ganda) atau `idempotency_key_tidak_didukung`; 403 `objek_nonaktif`+`jenis_objek=endpoint`; 404 `tidak_ditemukan`; 422 `validasi_gagal` (field tak dikenal, termasuk `TRESHOLD`, `USER_ID`, `PASSWORD`, `IP_USER`, `transactionId`) atau `metadata_tidak_valid`; 429 `kuota_habis` / `porsi_sync_habis` / `penolakan_berlebih`, semua dengan `Retry-After`; 502 `retry_habis` atau `provider_error_permanen` (+`request_id`, tanpa `Retry-After`); 503 `objek_nonaktif`+`jenis_objek` (pool/provider, tanpa `Retry-After`); 504 `timeout_sync` (+`request_id`, tanpa `Retry-After`) | S: ADR-0001 §4.1, contract §3.2.2, §3.2.5; US-03, 07, 08, 09 |
| `enqueueAsync` | `POST /v1/endpoints/{endpoint_id}/async` | token/API key | `SubmitRequest`; header opsional `Idempotency-Key` | 202 `EnqueueResponse{queue_id, status: "queued"}` (request baru); **200** `EnqueueResponse` dengan `queue_id` dan status terkini (kunci dan sidik sama, tidak ada request baru); 400 `validasi_gagal`; 403/503 `objek_nonaktif`; 404; 422 `validasi_gagal`, `metadata_tidak_valid`, atau `idempotency_key_konflik` (kunci sama, sidik beda); 429 `kuota_habis` / `penolakan_berlebih` | S: ADR-0001 §4.1; US-07. Kode 202 untuk request baru adalah U-5 (acuan hanya menyebut "respons berisi `queue_id`"). **Path menggantikan stub `/v1/async` di `4e7ea9e`** (U-4) |
| `getRequest` | `GET /v1/requests/{queue_id}` | token/API key | Path `queue_id` (UUID) | 200 `RequestStatus{queue_id, status: ApiStatus, hasil?, failure_reason?, cancel_reason?}`; 404 `tidak_ditemukan` (milik client lain atau tidak dikenal, tidak dibedakan, penolakan tercatat) | S: US-17, contract §3.2.4 S-01…S-05, ADR-0001 §4.1 |
| `cancelRequest` | `POST /v1/requests/{queue_id}/cancel` | token/API key | Tanpa body | 200 `RequestStatus{status: "cancelled", cancel_reason: "dibatalkan_client"}`; 404 `tidak_ditemukan`; 409 `tidak_bisa_dibatalkan` dengan status API saat ini di body (`status`) | S: US-19, ADR-0001 §4.1, contract T-02. Path dan method: U-4 |

Catatan bentuk:
- `status` memakai enum `ApiStatus` (enam nilai, tidak pernah mundur). `attempt_count > 0` → `processing` (ADR-0001 §4.1). `failure_reason` hanya bila `failed`, `cancel_reason` hanya bila `cancelled`; `expired` tidak punya kode alasan dalam acuan.
- `hasil` hanya pada `done`: objek `{ "<elemen>": {"status": "sesuai"|"tidak_sesuai", "skor": 0..100|null} }`, hanya elemen yang dikirim client, tanpa `NIK` (K-32, contract O-01/O-02). Nama properti `hasil` adalah U-6; contract memakai nama kolom `output_payload`.
- Hasil `done` bisa dipolling minimal selama TTL downstream pool sejak `completed_at` (S-05). Polling tidak memakai kuota.

### 2.1 Blok `webhooks:` (OpenAPI 3.1)

| Nama | Method | Dikirim ke | Isi |
|---|---|---|---|
| `requestTerminal` | `POST` ke `webhook_url` client (HTTPS) | Sekali per request async yang mencapai `done`, `failed`, `expired`, atau `cancelled`, bila client punya webhook aktif. Tanpa retry di F0 | Body `WebhookBody` = bentuk yang sama dengan `RequestStatus` (fungsi pemetaan yang sama, ADR-0001 §3a.6): `queue_id`, `status`, `hasil?`, `failure_reason?`, `cancel_reason?` |

Header Standard Webhooks pada setiap pengiriman (ADR-0001 §5a.6; contract W-08):

| Header | Nilai | Ditandatangani |
|---|---|---|
| `webhook-id` | `request_id` (= `queue_id`), tetap sama saat retry F1 | Ya |
| `webhook-timestamp` | Detik Unix | Ya |
| `webhook-signature` | `v1,<base64(HMAC-SHA256(kunci, "{webhook-id}.{webhook-timestamp}.{body}"))>`; kunci = byte hasil decode secret `whsec_…` | — |
| `poolrouter-secret-versi` | Integer `secret_versi` yang dipakai (contract §3.7) | **Tidak** (informatif) |

Respons client: 2xx = `terkirim`; selain itu (termasuk 3xx, redirect tidak diikuti) = `gagal` dan tidak ada retry. Panduan client di spec: bandingkan tanda tangan dengan waktu-konstan, tolak selisih timestamp >5 menit, dedupe per `webhook-id`.

### 2.2 Contoh yang wajib ada di spec (butir R)

| Butir | Contoh di spec | Tempat |
|---|---|---|
| R-05 | Respons `issueToken` (`expires_in: 900`, tanpa `refresh_token`); 429 `token_dibatasi` | `issueToken` |
| R-09 | Webhook `done` berikut keempat header, dan contoh `cancelled` dengan `cancel_reason` | `webhooks.requestTerminal` |
| R-10 | Satu contoh problem+json per `kode` pada tabel §2, termasuk 422 dengan `errors[{field, kode, batas?}]` dan 429 `penolakan_berlebih` dengan hanya empat field | Komponen `responses` |
| R-16 | Header `Cache-Control: no-store` di komponen `headers`, dirujuk semua respons | Komponen |
| R-18 | Ekspor auditor → admin API (§3) | `exportLogAuditor` |

## 3. Admin API (`admin-api.yaml`)

Auth: sesi cookie. Kolom **Peran** memakai nama dari contract §8.1: `platform_admin` (PA), `auditor` (AU), `operator` (OP), `client_owner` (CO). Peran di luar kolom → 403 `akses_ditolak` dan baris `audit_akses` `akses_ditolak`. Semua respons `no-store`. Semua operasi tulis: 401 tanpa sesi, 403 `origin_tidak_valid` bila `Origin` gagal, 422 `validasi_gagal`. Pengelolaan pengguna dan peran **tidak** ada di API (CLI, §3.6c).

### 3.1 Sesi dan identitas (ADR-0003 rev 5 §3.6, §3.6f; Addendum A §7.2)

| operationId | Method dan path | Peran | Request | Respons | Sumber |
|---|---|---|---|---|---|
| `login` | `POST /admin/v1/auth/login` | publik | `LoginRequest{username, password}` | 204 + `Set-Cookie: __Host-pr_session=…; HttpOnly; Secure; SameSite=Strict; Path=/`; 401 `kredensial_salah` (tidak membedakan username/password); 429 `login_dibatasi` + `Retry-After` (5 gagal/15 menit per username); 422 | S §3.6; kode: U-1 |
| `logout` | `POST /admin/v1/auth/logout` | semua | — | 204 + cookie kedaluwarsa; 401 | S |
| `getMe` | `GET /admin/v1/auth/me` | semua | — | 200 `Me{pengguna_id, username, peran[], client_id?, bahasa_ui: BahasaUi\|null}`; 401 | S (field dasar: "identitas, peran, `client_id`"; nama field U) |
| `updatePreferensi` | `PATCH /admin/v1/auth/me/preferensi` | semua | `{bahasa_ui: BahasaUi}`, tanpa ID pengguna | 200 `{bahasa_ui}`; 401; 403; 422 | S Addendum A §7.2 |
| `getSesi` | `GET /admin/v1/auth/sesi` | semua | — | 200 `SesiStatus`; 401. **Tidak menggeser idle** | S rev 5 §3.6f (final) |
| `perpanjangSesi` | `POST /admin/v1/auth/sesi/perpanjang` | semua | tanpa body | 200 `SesiStatus`; 401; 403. Reset idle, tidak menggeser batas absolut, tanpa `Set-Cookie` | S rev 5 §3.6f (final) |

`SesiStatus` (`additionalProperties: false`, semua wajib): `idle_berakhir_pada`, `absolut_berakhir_pada`, `waktu_server`, semuanya `date-time` UTC.

### 3.2 Konfigurasi (US-01, 02, 04, 05, 06, 15)

Sumber umum: peran PA (hak ubah), audit konfigurasi di setiap tulis (siapa, kapan, lama → baru). Nama path adalah usulan (ADR-0003 menyerahkan path ke OpenAPI). Detail per field mengikuti contract; di sini hanya batas yang menentukan respons.

| operationId | Method dan path | Peran | Request (field kunci) | Respons |
|---|---|---|---|---|
| `listProviders` / `getProvider` | `GET /admin/v1/providers`, `…/{id}` | PA | — | 200; 404 |
| `createProvider` | `POST /admin/v1/providers` | PA | `ProviderConfig`: endpoint/base URL, template body, parameter + rule validasi, skema respons, `tls_trust`, `log_mode`, `config_schema_version: 1` | 201 `Provider` (menaikkan `provider_config_version`); 422 `validasi_gagal` untuk template tidak seimbang (lokasi kesalahan di `errors`), `{{secret.X}}` yang tidak ada (nama secret yang hilang di `errors`), `config_schema_version` salah atau field tak dikenal; kunci template tersimpan persis (`USER_ID`, `TRESHOLD`, `NAMA_LGKP`) |
| `updateProvider` | `PUT /admin/v1/providers/{id}` | PA | `ProviderConfig` + `alasan` | 200 versi baru; versi lama tetap utuh (V-03); 404; 422 |
| `listSecrets` | `GET /admin/v1/secrets` | PA | — | 200 daftar `{nama, tersamar, dipakai_oleh_provider_count}`; nilai tidak pernah dikembalikan |
| `putSecret` | `PUT /admin/v1/secrets/{nama}` | PA | `{nilai}` (write-only) | 204; 422 |
| `deleteSecret` | `DELETE /admin/v1/secrets/{nama}` | PA | — | 204; 404; 409 `secret_dipakai` dengan daftar provider aktif yang memakainya (US-02 AC4) |
| `listUpstreamPools` / `getUpstreamPool` | `GET /admin/v1/upstream-pools`, `…/{id}` | PA | — | 200; 404 |
| `createUpstreamPool` / `updateUpstreamPool` | `POST /admin/v1/upstream-pools`, `PUT …/{id}` | PA | `provider_id`, `kuota_harian`, `rate_limit_per_jam`, `porsi_sync`, `margin_kuota` (0,001–0,200), `batas_percobaan` (1–5), `retry_backoff_base_detik`, `retry_backoff_cap_detik`, `retry_rasio`, `retry_min_per_jam`, `jendela_kuota` (`bergulir_24j`\|`kalender`), `reset_kuota_jam`, `reset_kuota_zona` (IANA), `jeda_min`, `jeda_maks`, `ambang_berdampingan`, `alasan` (wajib saat ubah) | 201/200; 422 di luar rentang KP-01, offset tetap pada zona, `kalender` tanpa sumber kepastian di `alasan`; `reset_kuota_*` berlaku di akhir periode (KP-02) |
| `listDownstreamPools` / `getDownstreamPool` | `GET /admin/v1/downstream-pools`, `…/{id}` | PA | — | 200; 404 |
| `createDownstreamPool` / `updateDownstreamPool` | `POST …`, `PUT …/{id}` | PA | `mode` (`sync`\|`async`), `ttl_antrean` (default 24 jam), `upstream_pool_id` (alokasi tunggal; tanpa reserved/shareable/prioritas) | 201/200; 422 |
| `listClients` / `getClient` | `GET /admin/v1/clients`, `…/{id}` | PA; CO hanya clientnya | — | 200; 404 |
| `createClient` / `updateClient` | `POST /admin/v1/clients`, `PUT …/{id}` | PA | `nama`, `limiter_penolakan_burst` (1–1000, default 20), `limiter_penolakan_isi_ulang_per_detik` (0,10–100,00, default 1,00), `alasan` wajib saat ubah limiter | 201/200; 422 di luar rentang (L-04) |
| `setClientEndpoints` | `PUT /admin/v1/clients/{id}/endpoints` | PA | `{endpoint_ids[]}` | 200; 422. Client tanpa penugasan → 404 saat memanggil downstream. **Bentuk penugasan client↔endpoint tidak ditetapkan acuan (U-7)** |
| `listCredentials` | `GET /admin/v1/clients/{id}/credentials` | PA; CO clientnya | — | 200 `[{credential_id, jenis, status, kedaluwarsa_at, dicabut_at?}]`; tanpa hash |
| `createCredential` | `POST /admin/v1/clients/{id}/credentials` | PA; CO clientnya | `{jenis: "oauth2"\|"api_key", kedaluwarsa_at?}` (default dan maksimum 365 hari) | 201 `CredentialSecret{credential_id, client_id, secret, kedaluwarsa_at}` dengan `secret` **sekali** (256 bit, base64url), `no-store`; 409 `batas_kredensial` (>2 aktif); 422 |
| `getWebhookConfig` | `GET /admin/v1/clients/{id}/webhook` | PA; CO clientnya | — | 200 `{webhook_url, aktif, secret_versi}`; tanpa secret; 404 |
| `putWebhookConfig` | `PUT /admin/v1/clients/{id}/webhook` | PA; CO clientnya | `{webhook_url, aktif}` | 200; 422 `webhook_tidak_https`, 422 `webhook_domain_tidak_diizinkan` (allowlist dicek saat simpan dan saat kirim) |
| `rotateWebhookSecret` | `POST /admin/v1/clients/{id}/webhook/rotasi-secret` | PA; CO clientnya | tanpa body | 200 `WebhookSecret{secret_versi, secret}`; `secret` = `whsec_` + base64 32 byte, **sekali**, `no-store`; berlaku untuk pengiriman berikutnya |
| `listWebhookLog` | `GET /admin/v1/clients/{id}/webhook/log` | CO clientnya; PA | Query: rentang waktu, `hasil`, kursor | 200 `[{request_id, sent_at, hasil, http_status?, error_class?, secret_versi?}]`; kegagalan pengiriman milik client terlihat (US-06) |
| `listWebhookAllowlist` / `putWebhookAllowlistEntry` / `deleteWebhookAllowlistEntry` | `GET`, `PUT`, `DELETE /admin/v1/webhook-allowlist[/{domain}]` | PA | `{aktif}` | 200/204; 422 |
| `listEndpoints` / `getEndpoint` | `GET /admin/v1/endpoints`, `…/{id}` | PA | — | 200; 404 |
| `createEndpoint` / `updateEndpoint` | `POST /admin/v1/endpoints`, `PUT …/{id}` | PA | `provider_id`, `downstream_pool_id`, `parameter_literal` (termasuk `TRESHOLD`), `log_mode`, `metadata_fields[]` (`nama_field`, `tipe`, `wajib`, `nilai_diizinkan?`, `aturan_validasi?`, `klasifikasi`, `kirim_ke_data_balikan`, `deskripsi`), `config_schema_version: 1`, `alasan` | 201/200 (menaikkan `endpoint_config_version`); 422: rujukan `{{endpoint.*}}` tanpa literal, endpoint Dukcapil tanpa `status_penerima_manfaat` (`metadata_wajib_hilang`, M-R1), field tak dikenal |

### 3.3 Penonaktifan dan reaktivasi K-20 (ADR-0001 §3a; contract EN-01…EN-09)

Pola sama untuk enam entitas. Pratinjau memakai `GET` tanpa body dan hanya mengembalikan angka.

| Entitas | Pratinjau | Nonaktifkan | Aktifkan |
|---|---|---|---|
| Client | `previewDeactivateClient` `GET /admin/v1/clients/{id}/pratinjau-nonaktif` | `deactivateClient` `POST …/nonaktifkan` | `activateClient` `POST …/aktifkan` |
| Kredensial | `previewRevokeCredential` `GET /admin/v1/credentials/{id}/pratinjau-cabut` | `revokeCredential` `POST …/cabut` | `activateCredential` `POST …/aktifkan`: **selalu 409 `kredensial_dicabut`** untuk kredensial `dicabut` (U-54b); penggantinya kredensial baru |
| Endpoint | `previewDeactivateEndpoint` | `deactivateEndpoint` | `activateEndpoint` |
| Downstream pool | `previewDeactivateDownstreamPool` | `deactivateDownstreamPool` | `activateDownstreamPool` |
| Upstream pool | `previewDeactivateUpstreamPool` | `deactivateUpstreamPool` | `activateUpstreamPool` |
| Provider | `previewDeactivateProvider` | `deactivateProvider` | `activateProvider` |

- Peran: PA. Body nonaktifkan/cabut: `{alasan}`. 404 untuk id tidak dikenal.
- Pratinjau 200 `PratinjauK20{akan_cancelled, akan_failed, dihitung_pada}`, tanpa `request_id`, NIK, atau payload, dan tidak disimpan. **Pratinjau upstream pool** menambah `upstream_aktif_terakhir_untuk[]` (`downstream_pool_id`, `nama`) bila pool itu upstream pool aktif terakhir bagi minimal satu downstream pool (ADR-0001 §3a.10); dua angka tidak berubah.
- Nonaktifkan 200 `HasilK20{jumlah_cancelled, jumlah_failed, dilakukan_pada}`. Angka yang sama masuk audit konfigurasi. `in_flight` tidak dihitung.
- Aktifkan 200 entitas; hanya berlaku untuk request yang masuk sesudahnya. Tanpa dialog K-20.
- Cache entitas ≤30 s: copy dan deskripsi operasi tidak boleh menulis "seketika" (§3a.4).

### 3.4 Quota tracker (US-12)

| operationId | Method dan path | Peran | Respons |
|---|---|---|---|
| `getQuotaTracker` | `GET /admin/v1/kuota?upstream_pool_id=` | PA | 200 `[{upstream_pool_id, periode_mulai, periode_selesai, kuota_harian, kuota_efektif, terpakai_periode, sisa_periode, terpakai_jam_berjalan, rate_limit_per_jam}]`; tanpa NIK; 404 |

Sumber angka: `upstream_attempt_daily` / `transaction_log` (bukan counter Redis; KP-07). Nama field U.

### 3.5 Log, audit akses, ekspor (US-14, 16; ADR-0003 §3.6b, §3.6d, §3.6e)

| operationId | Method dan path | Peran | Request | Respons |
|---|---|---|---|---|
| `searchLog` | `POST /admin/v1/log/cari` | OP, AU | `{dari, sampai, kode_transaksi?, queue_id?, client_id?, status_akses?, kursor?}` (rentang waktu wajib; `POST` agar filter tidak masuk access log) | 200 `LogPage{baris[{nik, requested_at, status_akses, kode_transaksi, queue_id, client_id}], kursor?, di_luar_retensi_online}`; `nik` tersamar (`************1234`) untuk OP, dibentuk server. Parameter yang memuat NIK atau potongannya → 403 `pencarian_nik_dilarang`, tercatat `akses_ditolak`. 422. Satu baris `audit_akses` per panggilan (`cari_log`) |
| `getLogEntry` | `GET /admin/v1/log/{kode_transaksi}` | OP, AU | — | 200 `LogEntry` (tersamar untuk OP); 404; baris `lihat_log` |
| `searchLogByNik` | `POST /admin/v1/auditor/log/cari-nik` | AU | `{nik: "^[0-9]{16}$", dari, sampai, kursor?, alasan: 10..500}` | 200 `LogPage` (NIK utuh; tanpa payload/metadata Rahasia); 403 peran lain (`akses_ditolak`); 422 tanpa NIK utuh, wildcard, atau prefiks. Tepat satu baris `audit_akses` `cari_nik` per panggilan, termasuk pindah halaman dan hasil kosong |
| `openPayload` | `POST /admin/v1/auditor/payload` | AU | `{request_id \| kode_transaksi, alasan: 10..500}` (salah satu) | 200 `PayloadView`; 403; 404; 422. Satu baris `lihat_payload` per pembukaan; tanpa cache |
| `exportLogAuditor` | `POST /admin/v1/auditor/log/ekspor` | AU | `{nik, dari, sampai, alasan: 10..500}` | 200 `text/csv; charset=utf-8`, `Content-Disposition: attachment; filename="ekspor-log-<audit_id>.csv"`, `Cache-Control: no-store`, `Repr-Digest: sha-256=:<base64>:`; kolom: `nik`, `requested_at`, `status_akses`, `kode_transaksi`, `queue_id`, `client_id`; 403; 422 `ekspor_terlalu_besar` (>10.000 baris, tanpa berkas, baris audit `ditolak`) atau validasi |
| `searchAuditAkses` | `POST /admin/v1/auditor/audit-akses/cari` | AU | `{dari, sampai (≤31 hari), pengguna_id?, aksi?, hasil?, kursor?}` | 200 kolom `audit_akses` tanpa `nik_hmac`; 403; 422. Baris `lihat_audit` per panggilan. Tidak ada endpoint ubah/hapus |

Contoh R-18 di spec: respons `exportLogAuditor` dengan keempat header dan 422 `ekspor_terlalu_besar`.

## 4. Enum bersama

Ditranskripsi ke `crates/webserver/src/api/enums.rs` dari contract v0.7.10 (tanpa TODO tersisa):

| Enum | Nilai | Rujukan |
|---|---|---|
| `ApiStatus` | `queued`, `processing`, `done`, `failed`, `expired`, `cancelled` | ADR-0001 §4.1; contract §3.2.4 |
| `FailureReason` | `retry_habis`, `provider_error_permanen`, `timeout_sync`, `objek_nonaktif` | §3.2.5 |
| `CancelReason` | `dibatalkan_client`, `client_nonaktif`, `kredensial_nonaktif`, `endpoint_nonaktif`, `pool_nonaktif`, `provider_nonaktif` | §3.2.3 |
| `RateLimitKode` | `kuota_habis`, `porsi_sync_habis`, `penolakan_berlebih` | ADR-0001 §5a.7 |
| `BahasaUi` | `id`, `en` | Addendum A §7.2 |

Nilai `reject_reason` (`validasi_gagal`, `metadata_tidak_valid`, `kuota_habis`, `porsi_sync_habis`, `objek_nonaktif`) tidak menjadi enum sendiri; mereka muncul sebagai `kode` di `ProblemKode`.

## 5. Tabel story → operationId (BRD v0.17 §7)

Semua story F0 yang punya permukaan API. Story tanpa permukaan API tercantum di bagian bawah.

| Story | Operasi |
|---|---|
| US-01 Daftarkan provider | `listProviders`, `getProvider`, `createProvider`, `updateProvider`, `createEndpoint`, `updateEndpoint` (nilai `TRESHOLD` di `parameter_literal`) |
| US-02 Secret storage | `listSecrets`, `putSecret`, `deleteSecret` |
| US-03 Rule validasi bawaan | `createProvider`, `updateProvider` (rule); perilaku tolak: 400/422 pada `submitSync`, `enqueueAsync`; limiter penolakan: 429 `penolakan_berlebih` pada keduanya; ubah limiter: `createClient`, `updateClient` |
| US-04 Upstream pool | `listUpstreamPools`, `getUpstreamPool`, `createUpstreamPool`, `updateUpstreamPool`; nonaktif lewat US-20 |
| US-05 Downstream pool | `listDownstreamPools`, `getDownstreamPool`, `createDownstreamPool`, `updateDownstreamPool` |
| US-06 Webhook dan kredensial | `createClient`, `setClientEndpoints`, `createCredential`, `listCredentials`, `getWebhookConfig`, `putWebhookConfig`, `rotateWebhookSecret`, `listWebhookLog`, `listWebhookAllowlist`, `putWebhookAllowlistEntry`, `deleteWebhookAllowlistEntry`; akses ke pool yang tidak ditugaskan: 404 pada `submitSync`/`enqueueAsync` |
| US-07 Kirim massal async | `enqueueAsync` (+ `Idempotency-Key`), `getEndpointKontrak`; hasil lewat US-17 |
| US-08 Validasi sync | `submitSync` |
| US-09 429 kuota habis | 429 pada `submitSync` dan `enqueueAsync` (`kuota_habis`, `porsi_sync_habis`, `penolakan_berlebih` + `Retry-After`) |
| US-12 Quota tracker | `getQuotaTracker` |
| US-14 Pencarian log online | `searchLogByNik`, `searchLog` (`di_luar_retensi_online`) |
| US-15 Kontrak metadata | `createEndpoint`, `updateEndpoint` (`metadata_fields`), `getEndpointKontrak`; tolak `metadata_tidak_valid` pada `submitSync`/`enqueueAsync` |
| US-16 Kontrol akses log NIK dan audit | `searchLog`, `getLogEntry`, `searchLogByNik`, `openPayload`, `exportLogAuditor`, `searchAuditAkses` |
| US-17 Polling dan webhook | `getRequest`, `webhooks.requestTerminal` |
| US-18 Autentikasi client | `issueToken`; skema `oauth2`/`apiKey` pada semua operasi downstream; 401 |
| US-19 Pembatalan oleh client | `cancelRequest` |
| US-20 Pembatalan massal | `previewDeactivate*`, `deactivate*`, `previewRevokeCredential`, `revokeCredential`, `activate*` (enam entitas, §3.3); webhook `cancelled` lewat `webhooks.requestTerminal` |
| US-21 TTL dan `expired` | `getRequest` (`status: expired`), `webhooks.requestTerminal`; TTL di `createDownstreamPool`/`updateDownstreamPool` |
| US-22 Anti-pola NIK | `createUpstreamPool`, `updateUpstreamPool` (`jeda_min`, `jeda_maks`, `ambang_berdampingan`); tanpa permukaan API client |
| US-23 Log transaksi | Tanpa operasi sendiri; dibaca lewat `searchLog`, `getLogEntry`, `searchLogByNik` |
| A-01 WCAG 2.2.1 (SHAA-200) | `getSesi`, `perpanjangSesi` |
| i18n (Addendum A, SHAA-214) | `getMe`, `updatePreferensi` |

Bukan story F0 di §7 v0.17, jadi tidak dipetakan: US-10 (F2), US-11 dan US-13 (F1), lihat §9 dan §14 BRD. Autentikasi UI (`login`, `logout`) tidak ditautkan ke story F0; keduanya diturunkan dari ADR-0003 §3.6.

## 6. Yang dikerjakan dan belum

Dikerjakan tanpa kompilasi di branch ini: dokumen ini, dan transkripsi enum `FailureReason`/`CancelReason`. **Belum pernah dikompilasi**: seluruh kode Rust di branch (generator, tipe `Problem`, stub `enqueueAsync`), `Cargo.lock` belum diperbarui. Handler `utoipa` untuk operasi di atas **tidak** ditulis sebelum ada compile pertama (arahan Shu).

Stub `enqueueAsync` di `downstream.rs` masih memakai path `/v1/async`; akan diubah menjadi `/v1/endpoints/{endpoint_id}/async` setelah U-4 diputuskan.

## 7. Keputusan yang diminta di review

| # | Pertanyaan | Rekomendasi saya |
|---|---|---|
| U-1 | Daftar `kode` tambahan (§1.1), khususnya satu `tidak_terautentikasi` untuk semua 401 | Setuju satu kode; rincian penyebab hanya di metrik MT-03 |
| U-2 | Nama header API key | `X-API-Key` |
| U-3 | `issueToken` memakai problem+json (R-10) bukan `{"error": …}` RFC 6749 §5.2. Pustaka OAuth2 client standar membaca `error` | Problem+json plus field `error` kompatibel RFC 6749 di body. Butuh keputusan Shu karena menyentuh R-10 dan R-05 |
| U-4 | Endpoint dipilih lewat path (`/v1/endpoints/{endpoint_id}/sync\|async`) bukan body. Sidik idempotensi memakai `downstream_endpoint_id`, dan kontrak input berbeda per endpoint | Path. Mengganti stub `/v1/async` di `4e7ea9e`; belum dipakai siapa pun. Pembatalan/polling di `/v1/requests/{queue_id}` |
| U-5 | 202 untuk request baru, 200 untuk replay idempoten | Ya (acuan hanya membedakan "200 request lama") |
| U-6 | Nama properti hasil: `hasil` | Ya; konsisten di sync, polling, dan webhook |
| U-7 | Penugasan client ↔ endpoint/pool belum dimodelkan di acuan (US-06 hanya menyebut "client tidak di-assign ke pool → ditolak") | `setClientEndpoints` seperti di §3.2; Klaus perlu mengonfirmasi tabelnya |
| U-8 | Kode untuk 429 "rate limit jam berjalan penuh" pada sync (BRD US-08 AC memisahkannya dari "porsi sync habis"), padahal `RateLimitKode` tertutup tiga nilai | Pakai `porsi_sync_habis` hanya untuk porsi, dan `kuota_habis` untuk rate limit penuh dengan `Retry-After` GCRA; ADR-0001 §5a.7 menyebut "429 tunggu token sync" di baris `porsi_sync_habis`, jadi bisa juga dipetakan ke situ. Perlu keputusan Shu, karena enum tertutup |
| U-9 | Lokasi kesalahan template (US-01) di `errors[{field, kode, batas}]`: `batas` dibatasi "parameter angka aturan" | `field` = JSON pointer ke template, `batas: {baris, kolom}`. Perlu Shu karena menyentuh batasan Addendum A §6.1 |
| U-10 | Daftar request milik client untuk CO (contract §8.1 menyebut status/hasil/webhook milik client-nya) tidak punya story F0 | Tidak dimasukkan. Fitcher: konfirmasi apakah layar UI membutuhkannya |
| U-11 | Pembacaan audit konfigurasi di UI tidak ada di lingkup issue | Tidak dimasukkan |

Cek terhadap UI (layar, copy) tidak saya lakukan di dokumen ini; itu bagian review Fitcher.
