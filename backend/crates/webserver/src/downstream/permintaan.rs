//! Siklus hidup request downstream, sisi tulis: intake async, baris `rejected`,
//! polling, pembatalan client, dan dua transaksi jalur sync (ARC-63, ARC-65,
//! ARC-66; ADR-0001 rev 15 §3, §4, §4.1).
//!
//! Semua transisi status di sini adalah compare-and-set di PostgreSQL
//! (`... WHERE status = <lama>`), ditulis bersama baris `request_status_history`
//! dalam transaksi yang sama (T-04). Nol baris berarti pihak lain menang, dan
//! pemanggil berhenti tanpa efek samping.
//!
//! Hanya pool `pr_api` yang sampai ke modul ini. Grant kolom `pr_api` tidak
//! mengizinkan SELECT `input_payload` dan `metadata`, jadi tidak ada `RETURNING`
//! atas dua kolom itu.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use poolrouter_core::retry::ErrorClass;
use poolrouter_core::status_api::StatusInternal;
use serde_json::Value;
use sqlx::types::Json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::downstream_sesi::{Fut, GalatDownstream, Hasil, SesiKlien};
use super::intake::{FieldInputKontrak, FieldMetadataKontrak};
use crate::api::downstream::HasilElemen;
use crate::api::enums::JenisObjek;

/// Satu upstream pool rute dari downstream pool endpoint, beserta parameter
/// kuotanya. Periode dihitung sekali saat rantai dimuat, dari `received_at`
/// (ADR-0001 rev 15 §5a.1 butir 1d) lewat fungsi yang sama dengan rebuild.
#[derive(Debug, Clone, PartialEq)]
pub struct PoolRute {
    pub upstream_pool_id: Uuid,
    pub kuota_harian: i64,
    pub margin_kuota: f64,
    /// `jendela_kuota = 'kalender'`; selain itu jendela bergulir 24 jam.
    pub kalender: bool,
    pub periode_mulai: DateTime<Utc>,
    pub periode_selesai: DateTime<Utc>,
    pub porsi_sync_maks: f64,
    pub retry_rasio: f64,
    pub retry_min_per_jam: i64,
    pub parameter_dispatch: Value,
}

impl PoolRute {
    /// `floor(kuota_harian * (1 - margin))`, minimal 1 (§5a.1 butir 1).
    pub fn kuota_efektif(&self) -> i64 {
        let efektif = (self.kuota_harian as f64 * (1.0 - self.margin_kuota)).floor() as i64;
        efektif.max(1)
    }
}

/// Konfigurasi satu endpoint yang ditugaskan ke client pemanggil, sebatas yang
/// dibutuhkan intake, sync, dan baris `request`.
#[derive(Debug, Clone, PartialEq)]
pub struct RantaiEndpoint {
    pub endpoint_id: Uuid,
    pub provider_id: Uuid,
    pub downstream_pool_id: Uuid,
    pub prioritas: i32,
    pub ttl_antrean_detik: i64,
    pub provider_config_version: i32,
    pub endpoint_config_version: i32,
    /// `log_mode` efektif: endpoint bila diisi, selain itu provider.
    pub log_mode: String,
    pub wajib_log_nik: bool,
    pub field_input: Vec<FieldInputKontrak>,
    pub field_metadata: Vec<FieldMetadataKontrak>,
    /// Nama field metadata berklasifikasi `Rahasia` (P-01, P-02).
    pub metadata_rahasia: Vec<String>,
    /// Entitas pertama yang nonaktif, urutan tetap §3a.4: endpoint, downstream
    /// pool, upstream pool (semua rute nonaktif), provider.
    pub nonaktif: Option<JenisObjek>,
    /// Upstream pool rute yang aktif, urut `upstream_pool_id`.
    pub rute: Vec<PoolRute>,
    pub limiter_burst: i64,
    pub limiter_isi_ulang_mikro: i64,
}

impl RantaiEndpoint {
    /// Metadata yang boleh disimpan pada baris `rejected`: hanya kunci kontrak
    /// yang bukan `Rahasia` (P-01). Selain objek menjadi `{}`.
    pub fn metadata_untuk_tolak(&self, metadata: Option<&Value>) -> Value {
        let mut out = serde_json::Map::new();
        if let Some(Value::Object(m)) = metadata {
            for (k, v) in m {
                let dikenal = self.field_metadata.iter().any(|f| &f.nama_field == k);
                if dikenal && !self.metadata_rahasia.iter().any(|r| r == k) {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
        Value::Object(out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Sync,
    Async,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Async => "async",
        }
    }
}

/// Satu baris `rejected`. `metadata` sudah disaring pemanggil.
#[derive(Debug, Clone, PartialEq)]
pub struct Penolakan {
    pub request_id: Uuid,
    pub mode: Mode,
    pub received_at: DateTime<Utc>,
    pub reject_reason: &'static str,
    pub http_status: u16,
    pub metadata: Value,
}

/// Request async yang diterima (`queued`).
#[derive(Debug, Clone, PartialEq)]
pub struct Antrean {
    pub request_id: Uuid,
    pub dispatch_key: Uuid,
    pub received_at: DateTime<Utc>,
    pub input: Value,
    pub metadata: Value,
    /// `(idempotency_key, idempotency_fingerprint)`.
    pub idempoten: Option<(String, String)>,
    pub referensi_client: Option<String>,
}

/// Baris yang memegang sebuah `Idempotency-Key`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarisIdempoten {
    pub request_id: Uuid,
    pub status: StatusInternal,
    pub attempt_count: i32,
    pub sidik: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HasilAntre {
    Baru,
    /// Unique constraint menengahi dua request dengan kunci sama; yang kalah
    /// membaca baris pemenang (§4.1).
    Ada(BarisIdempoten),
}

/// Status sebuah request untuk polling dan jawaban 409.
#[derive(Debug, Clone, PartialEq)]
pub struct BarisStatus {
    pub request_id: Uuid,
    pub status: StatusInternal,
    pub attempt_count: i32,
    pub failure_reason: Option<String>,
    pub cancel_reason: Option<String>,
    pub output: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HasilBatal {
    Dibatalkan,
    /// Tidak ada, milik client lain, atau `rejected` (404 yang sama).
    TidakAda,
    /// Request ada tetapi tidak lagi `queued`/`dispatched` dengan
    /// `attempt_count = 0`.
    TidakBisa(BarisStatus),
}

/// Transaksi pertama jalur sync: request `in_flight`, history, dan baris log
/// dengan `status_akses` NULL, sebelum panggilan upstream (§4).
#[derive(Debug, Clone, PartialEq)]
pub struct MulaiSync {
    pub request_id: Uuid,
    pub kode_transaksi: Uuid,
    pub upstream_pool_id: Uuid,
    pub received_at: DateTime<Utc>,
    pub requested_at: DateTime<Utc>,
    pub input: Value,
    pub metadata: Value,
    pub nik: Option<String>,
    pub referensi_client: Option<String>,
    /// Keluaran log render (secret sebagai penanda), hanya pada `full_payload`
    /// (ADR-0001 bagian 7, R-05).
    pub request_payload: Option<Value>,
}

/// Jawaban provider yang sudah dibaca adapter.
#[derive(Debug, Clone)]
pub enum HasilPanggilan {
    Berhasil {
        http_status: u16,
        hasil: BTreeMap<String, HasilElemen>,
    },
    Gagal {
        kelas: ErrorClass,
        http_status: Option<u16>,
    },
}

/// Status akhir sync yang ditulis untuk satu hasil panggilan (ADR-0001 rev 15
/// §4.1, tabel kode HTTP, empat baris terakhir).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AkhirSync {
    pub status: StatusInternal,
    pub failure_reason: Option<&'static str>,
    pub status_akses: &'static str,
    pub error_class: Option<&'static str>,
}

/// Pemetaan murni hasil panggilan ke kolom request dan log. `timeout` menjadi
/// `timeout_sync`; kelas sementara (`koneksi`, `rate_limit_provider`,
/// `provider_sementara`) menjadi `retry_habis` karena sync hanya punya satu
/// percobaan efektif; kelas lain `provider_error_permanen`.
pub fn akhir_sync(hasil: &HasilPanggilan) -> AkhirSync {
    match hasil {
        HasilPanggilan::Berhasil { .. } => AkhirSync {
            status: StatusInternal::Done,
            failure_reason: None,
            status_akses: "berhasil",
            error_class: None,
        },
        HasilPanggilan::Gagal { kelas, .. } => {
            let failure = match kelas {
                ErrorClass::Timeout => "timeout_sync",
                k if k.retryable() => "retry_habis",
                _ => "provider_error_permanen",
            };
            let akses = match kelas {
                ErrorClass::Timeout | ErrorClass::Koneksi | ErrorClass::Tls => "gagal_jaringan",
                _ => "gagal_provider",
            };
            AkhirSync {
                status: StatusInternal::Failed,
                failure_reason: Some(failure),
                status_akses: akses,
                error_class: Some(kelas.as_str()),
            }
        }
    }
}

/// Transaksi kedua jalur sync.
#[derive(Debug, Clone)]
pub struct SelesaiSync {
    pub request_id: Uuid,
    pub kode_transaksi: Uuid,
    /// Waktu hasil diterima; `transaction_log.responded_at` hanya diisi bila
    /// provider menjawab HTTP (R-07: kegagalan TLS dan timeout tanpa respons).
    pub responded_at: DateTime<Utc>,
    pub duration_ms: i32,
    pub hasil: HasilPanggilan,
    /// Hanya pada `full_payload` dan bila provider menjawab.
    pub response_payload: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TulisSync {
    Ditulis,
    /// Compare-and-set `in_flight` menghasilkan nol baris (reconciler menang):
    /// respons mengikuti status di PostgreSQL (T-06).
    Kalah(Option<BarisStatus>),
}

/// Persistensi operasi downstream. Implementasi produksi: [`PgPermintaan`];
/// uji router: [`fake::FakePermintaan`].
pub trait PenyimpanPermintaan: Send + Sync + 'static {
    /// Rantai endpoint yang ditugaskan ke `klien`; `None` bila tidak ditugaskan
    /// atau tidak ada (404 yang sama).
    fn muat_rantai(
        &self,
        klien: SesiKlien,
        endpoint: Uuid,
        sekarang: DateTime<Utc>,
    ) -> Fut<'_, Hasil<Option<RantaiEndpoint>>>;

    fn cari_idempoten(
        &self,
        klien: SesiKlien,
        kunci: String,
    ) -> Fut<'_, Hasil<Option<BarisIdempoten>>>;

    fn simpan_ditolak(
        &self,
        klien: SesiKlien,
        rantai: Arc<RantaiEndpoint>,
        tolak: Penolakan,
    ) -> Fut<'_, Hasil<()>>;

    fn simpan_antre(
        &self,
        klien: SesiKlien,
        rantai: Arc<RantaiEndpoint>,
        antrean: Antrean,
    ) -> Fut<'_, Hasil<HasilAntre>>;

    /// `None` untuk request yang tidak ada, milik client lain, atau `rejected`.
    fn status(&self, klien: SesiKlien, request_id: Uuid) -> Fut<'_, Hasil<Option<BarisStatus>>>;

    fn batalkan(&self, klien: SesiKlien, request_id: Uuid) -> Fut<'_, Hasil<HasilBatal>>;

    fn mulai_sync(
        &self,
        klien: SesiKlien,
        rantai: Arc<RantaiEndpoint>,
        mulai: MulaiSync,
    ) -> Fut<'_, Hasil<()>>;

    fn selesai_sync(&self, klien: SesiKlien, selesai: SelesaiSync) -> Fut<'_, Hasil<TulisSync>>;
}

fn layanan<E>(_: E) -> GalatDownstream {
    GalatDownstream::Layanan
}

fn status_internal(teks: &str) -> Hasil<StatusInternal> {
    teks.parse::<StatusInternal>()
        .map_err(|_| GalatDownstream::Layanan)
}

/// Nama `{{input.NAMA}}` di template body provider: kontrak input endpoint
/// menurut ADR-0002 rev 5 D6. Parameter `{{endpoint.*}}`, `{{secret.*}}`, dan
/// `{{system.*}}` bukan bagian kontrak input client.
pub fn field_input_dari_template(template: &str) -> Vec<FieldInputKontrak> {
    let mut nama: Vec<String> = Vec::new();
    let mut sisa = template;
    while let Some(i) = sisa.find("{{") {
        sisa = &sisa[i + 2..];
        let Some(j) = sisa.find("}}") else {
            break;
        };
        let isi = sisa[..j].trim();
        let n = isi.strip_prefix("input.").unwrap_or_default();
        if !n.is_empty() && !nama.iter().any(|x| x == n) {
            nama.push(n.to_owned());
        }
        sisa = &sisa[j + 2..];
    }
    nama.into_iter()
        .map(|nama| FieldInputKontrak {
            nama,
            tipe: "string".to_owned(),
            // Elemen yang tidak dikirim dihilangkan dari output (ADR-0002 P-1),
            // jadi tidak ada elemen yang wajib dari sisi kontrak input.
            wajib: false,
        })
        .collect()
}

/// PostgreSQL, sebagai `pr_api`.
#[derive(Clone)]
pub struct PgPermintaan {
    pool: PgPool,
}

impl PgPermintaan {
    pub fn baru(pool: PgPool) -> Arc<Self> {
        Arc::new(Self { pool })
    }

    async fn muat_rantai_dalam(
        &self,
        klien: SesiKlien,
        endpoint: Uuid,
        sekarang: DateTime<Utc>,
    ) -> Hasil<Option<RantaiEndpoint>> {
        let Some(b) = sqlx::query(
            "SELECT e.aktif AS endpoint_aktif, e.config_versi_aktif AS endpoint_versi, \
                    e.provider_id, p.aktif AS provider_aktif, p.config_versi_aktif AS provider_versi, \
                    p.wajib_log_nik, pcv.dokumen AS provider_dokumen, pcv.log_mode AS provider_log_mode, \
                    ecv.log_mode AS endpoint_log_mode, d.downstream_pool_id, d.aktif AS pool_aktif, \
                    d.prioritas, d.ttl_antrean_detik, c.limiter_penolakan_burst, \
                    c.limiter_penolakan_isi_ulang_per_detik::float8 AS limiter_isi \
               FROM client_endpoint ce \
               JOIN downstream_endpoint e ON e.downstream_endpoint_id = ce.downstream_endpoint_id \
               JOIN provider p ON p.provider_id = e.provider_id \
               JOIN downstream_pool d ON d.downstream_pool_id = e.downstream_pool_id \
               JOIN client c ON c.client_id = ce.client_id \
               LEFT JOIN provider_config_versi pcv \
                      ON pcv.provider_id = p.provider_id AND pcv.versi = p.config_versi_aktif \
               LEFT JOIN endpoint_config_versi ecv \
                      ON ecv.downstream_endpoint_id = e.downstream_endpoint_id \
                     AND ecv.versi = e.config_versi_aktif \
              WHERE ce.client_id = $1 AND ce.downstream_endpoint_id = $2",
        )
        .bind(klien.client_id)
        .bind(endpoint)
        .fetch_optional(&self.pool)
        .await
        .map_err(layanan)?
        else {
            return Ok(None);
        };

        // Versi konfigurasi aktif yang belum ada berarti endpoint belum siap
        // dipakai; itu masalah layanan, bukan versi 0 karangan.
        let endpoint_versi: i32 = b
            .try_get::<Option<i32>, _>("endpoint_versi")
            .map_err(layanan)?
            .ok_or(GalatDownstream::Layanan)?;
        let provider_versi: i32 = b
            .try_get::<Option<i32>, _>("provider_versi")
            .map_err(layanan)?
            .ok_or(GalatDownstream::Layanan)?;
        let dokumen: Value = b
            .try_get::<Option<Json<Value>>, _>("provider_dokumen")
            .map_err(layanan)?
            .ok_or(GalatDownstream::Layanan)?
            .0;
        let provider_log_mode: String = b
            .try_get::<Option<String>, _>("provider_log_mode")
            .map_err(layanan)?
            .ok_or(GalatDownstream::Layanan)?;
        let endpoint_log_mode: Option<String> = b.try_get("endpoint_log_mode").map_err(layanan)?;
        let downstream_pool_id: Uuid = b.try_get("downstream_pool_id").map_err(layanan)?;
        let endpoint_aktif: bool = b.try_get("endpoint_aktif").map_err(layanan)?;
        let pool_aktif: bool = b.try_get("pool_aktif").map_err(layanan)?;
        let provider_aktif: bool = b.try_get("provider_aktif").map_err(layanan)?;
        let ttl: i32 = b.try_get("ttl_antrean_detik").map_err(layanan)?;
        let burst: i32 = b.try_get("limiter_penolakan_burst").map_err(layanan)?;
        let isi: f64 = b.try_get("limiter_isi").map_err(layanan)?;

        let meta = sqlx::query(
            "SELECT nama_field, wajib, nilai_diizinkan, klasifikasi \
               FROM endpoint_metadata_field WHERE downstream_endpoint_id = $1 ORDER BY nama_field",
        )
        .bind(endpoint)
        .fetch_all(&self.pool)
        .await
        .map_err(layanan)?;
        let mut field_metadata = Vec::with_capacity(meta.len());
        let mut metadata_rahasia = Vec::new();
        for r in &meta {
            let nama_field: String = r.try_get("nama_field").map_err(layanan)?;
            let klasifikasi: String = r.try_get("klasifikasi").map_err(layanan)?;
            let nilai: Option<Json<Value>> = r.try_get("nilai_diizinkan").map_err(layanan)?;
            let nilai_diizinkan = match nilai.map(|j| j.0) {
                None | Some(Value::Null) => None,
                Some(v) => Some(serde_json::from_value::<Vec<String>>(v).map_err(layanan)?),
            };
            if klasifikasi == "Rahasia" {
                metadata_rahasia.push(nama_field.clone());
            }
            field_metadata.push(FieldMetadataKontrak {
                nama_field,
                wajib: r.try_get("wajib").map_err(layanan)?,
                nilai_diizinkan,
            });
        }

        let baris_rute = sqlx::query(
            "SELECT u.upstream_pool_id, u.aktif, u.kuota_harian, u.margin_kuota::float8 AS margin_kuota, \
                    u.porsi_sync_maks::float8 AS porsi_sync_maks, u.retry_rasio::float8 AS retry_rasio, \
                    u.retry_min_per_jam, u.jendela_kuota, u.parameter_dispatch, \
                    u.reset_kuota_jam, u.reset_kuota_zona, u.reset_kuota_berlaku_mulai, \
                    u.reset_kuota_jam_berikut, u.reset_kuota_zona_berikut \
               FROM downstream_pool_rute r \
               JOIN upstream_pool u ON u.upstream_pool_id = r.upstream_pool_id \
              WHERE r.downstream_pool_id = $1 \
              ORDER BY u.upstream_pool_id",
        )
        .bind(downstream_pool_id)
        .fetch_all(&self.pool)
        .await
        .map_err(layanan)?;
        let mut rute = Vec::new();
        for r in &baris_rute {
            if !r.try_get::<bool, _>("aktif").map_err(layanan)? {
                continue;
            }
            // Fungsi periode yang sama dengan rebuild dan snapshot, supaya kunci
            // Redis intake sama dengan kunci yang mereka tulis (M-02).
            let periode = poolrouter_core::kuota::sumber_pg::periode_dari_baris(r, sekarang)
                .map_err(layanan)?;
            let jendela: String = r.try_get("jendela_kuota").map_err(layanan)?;
            rute.push(PoolRute {
                upstream_pool_id: r.try_get("upstream_pool_id").map_err(layanan)?,
                kuota_harian: i64::from(r.try_get::<i32, _>("kuota_harian").map_err(layanan)?),
                margin_kuota: r.try_get("margin_kuota").map_err(layanan)?,
                kalender: jendela == "kalender",
                periode_mulai: periode.mulai,
                periode_selesai: periode.selesai,
                porsi_sync_maks: r.try_get("porsi_sync_maks").map_err(layanan)?,
                retry_rasio: r.try_get("retry_rasio").map_err(layanan)?,
                retry_min_per_jam: i64::from(
                    r.try_get::<i32, _>("retry_min_per_jam").map_err(layanan)?,
                ),
                parameter_dispatch: r
                    .try_get::<Json<Value>, _>("parameter_dispatch")
                    .map_err(layanan)?
                    .0,
            });
        }

        let nonaktif = if !endpoint_aktif {
            Some(JenisObjek::Endpoint)
        } else if !pool_aktif {
            Some(JenisObjek::DownstreamPool)
        } else if rute.is_empty() {
            Some(JenisObjek::UpstreamPool)
        } else if !provider_aktif {
            Some(JenisObjek::Provider)
        } else {
            None
        };

        let template = dokumen
            .get("body_template")
            .and_then(Value::as_str)
            .unwrap_or_default();
        Ok(Some(RantaiEndpoint {
            endpoint_id: endpoint,
            provider_id: b.try_get("provider_id").map_err(layanan)?,
            downstream_pool_id,
            prioritas: b.try_get("prioritas").map_err(layanan)?,
            ttl_antrean_detik: i64::from(ttl),
            provider_config_version: provider_versi,
            endpoint_config_version: endpoint_versi,
            log_mode: endpoint_log_mode.unwrap_or(provider_log_mode),
            wajib_log_nik: b.try_get("wajib_log_nik").map_err(layanan)?,
            field_input: field_input_dari_template(template),
            field_metadata,
            metadata_rahasia,
            nonaktif,
            rute,
            limiter_burst: i64::from(burst),
            limiter_isi_ulang_mikro: isi_ulang_mikro(isi),
        }))
    }

    async fn cari_idempoten_dalam(
        &self,
        klien: SesiKlien,
        kunci: &str,
    ) -> Hasil<Option<BarisIdempoten>> {
        let Some(r) = sqlx::query(
            "SELECT request_id, status, attempt_count, idempotency_fingerprint \
               FROM request WHERE client_id = $1 AND idempotency_key = $2",
        )
        .bind(klien.client_id)
        .bind(kunci)
        .fetch_optional(&self.pool)
        .await
        .map_err(layanan)?
        else {
            return Ok(None);
        };
        Ok(Some(BarisIdempoten {
            request_id: r.try_get("request_id").map_err(layanan)?,
            status: status_internal(&r.try_get::<String, _>("status").map_err(layanan)?)?,
            attempt_count: r.try_get("attempt_count").map_err(layanan)?,
            sidik: r
                .try_get::<Option<String>, _>("idempotency_fingerprint")
                .map_err(layanan)?
                .unwrap_or_default(),
        }))
    }

    async fn simpan_ditolak_dalam(
        &self,
        klien: SesiKlien,
        rantai: &RantaiEndpoint,
        t: Penolakan,
    ) -> Hasil<()> {
        let ttl = match t.mode {
            Mode::Async => {
                Some(t.received_at + chrono::Duration::seconds(rantai.ttl_antrean_detik))
            }
            Mode::Sync => None,
        };
        let mut tx = self.pool.begin().await.map_err(layanan)?;
        sqlx::query(
            "INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, \
               downstream_pool_id, provider_id, provider_config_version, endpoint_config_version, \
               mode, prioritas, prioritas_efektif, status, reject_reason, reject_http_status, \
               metadata, received_at, ttl_expires_at, completed_at, attempt_count) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10, 'rejected', $11, $12, $13, \
                     $14, $15, $14, 0)",
        )
        .bind(t.request_id)
        .bind(klien.client_id)
        .bind(klien.credential_id)
        .bind(rantai.endpoint_id)
        .bind(rantai.downstream_pool_id)
        .bind(rantai.provider_id)
        .bind(rantai.provider_config_version)
        .bind(rantai.endpoint_config_version)
        .bind(t.mode.as_str())
        .bind(rantai.prioritas)
        .bind(t.reject_reason)
        .bind(i32::from(t.http_status))
        .bind(Json(t.metadata.clone()))
        .bind(t.received_at)
        .bind(ttl)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %ringkas(&e), "downstream: baris rejected gagal ditulis");
            GalatDownstream::Layanan
        })?;
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor) \
             VALUES ($1, 1, NULL, 'rejected', $2, 'webserver')",
        )
        .bind(t.request_id)
        .bind(t.reject_reason)
        .execute(&mut *tx)
        .await
        .map_err(layanan)?;
        tx.commit().await.map_err(layanan)
    }

    async fn simpan_antre_dalam(
        &self,
        klien: SesiKlien,
        rantai: &RantaiEndpoint,
        a: Antrean,
    ) -> Hasil<HasilAntre> {
        let ttl = a.received_at + chrono::Duration::seconds(rantai.ttl_antrean_detik);
        let (kunci, sidik) = match &a.idempoten {
            Some((k, s)) => (Some(k.as_str()), Some(s.as_str())),
            None => (None, None),
        };
        let mut tx = self.pool.begin().await.map_err(layanan)?;
        // Unique index parsial menengahi dua request dengan kunci yang sama: yang
        // kalah tidak menulis apa pun lalu membaca baris pemenang (§4.1, AC-12).
        let baru: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, \
               downstream_pool_id, provider_id, provider_config_version, endpoint_config_version, \
               mode, prioritas, prioritas_efektif, status, idempotency_key, idempotency_fingerprint, \
               referensi_client, input_payload, metadata, received_at, ttl_expires_at, dispatch_key, \
               attempt_count) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'async', $9, $9, 'queued', $10, $11, $12, $13, \
                     $14, $15, $16, $17, 0) \
             ON CONFLICT (client_id, idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING \
             RETURNING request_id",
        )
        .bind(a.request_id)
        .bind(klien.client_id)
        .bind(klien.credential_id)
        .bind(rantai.endpoint_id)
        .bind(rantai.downstream_pool_id)
        .bind(rantai.provider_id)
        .bind(rantai.provider_config_version)
        .bind(rantai.endpoint_config_version)
        .bind(rantai.prioritas)
        .bind(kunci)
        .bind(sidik)
        .bind(a.referensi_client.as_deref())
        .bind(Json(a.input.clone()))
        .bind(Json(a.metadata.clone()))
        .bind(a.received_at)
        .bind(ttl)
        .bind(a.dispatch_key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %ringkas(&e), "downstream: insert queued gagal");
            GalatDownstream::Layanan
        })?;
        if baru.is_none() {
            tx.rollback().await.map_err(layanan)?;
            let kunci = kunci.ok_or(GalatDownstream::Layanan)?;
            return self
                .cari_idempoten_dalam(klien, kunci)
                .await?
                .map(HasilAntre::Ada)
                .ok_or(GalatDownstream::Layanan);
        }
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) \
             VALUES ($1, 1, NULL, 'queued', 'webserver')",
        )
        .bind(a.request_id)
        .execute(&mut *tx)
        .await
        .map_err(layanan)?;
        tx.commit().await.map_err(layanan)?;
        Ok(HasilAntre::Baru)
    }

    async fn status_dalam(&self, klien: SesiKlien, id: Uuid) -> Hasil<Option<BarisStatus>> {
        let Some(r) = sqlx::query(
            "SELECT request_id, status, attempt_count, failure_reason, cancel_reason, output_payload \
               FROM request WHERE request_id = $1 AND client_id = $2",
        )
        .bind(id)
        .bind(klien.client_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(layanan)?
        else {
            return Ok(None);
        };
        let status = status_internal(&r.try_get::<String, _>("status").map_err(layanan)?)?;
        // S-02: baris `rejected` tidak pernah dipolling; 404 yang sama.
        if status == StatusInternal::Rejected {
            return Ok(None);
        }
        Ok(Some(BarisStatus {
            request_id: r.try_get("request_id").map_err(layanan)?,
            status,
            attempt_count: r.try_get("attempt_count").map_err(layanan)?,
            failure_reason: r.try_get("failure_reason").map_err(layanan)?,
            cancel_reason: r.try_get("cancel_reason").map_err(layanan)?,
            output: r
                .try_get::<Option<Json<Value>>, _>("output_payload")
                .map_err(layanan)?
                .map(|j| j.0),
        }))
    }

    async fn batalkan_dalam(&self, klien: SesiKlien, id: Uuid) -> Hasil<HasilBatal> {
        let mut tx = self.pool.begin().await.map_err(layanan)?;
        // Compare-and-set dengan status lama dikunci di statement yang sama:
        // pembatalan kedua yang menunggu kunci membaca ulang baris, tidak lagi
        // cocok, dan mendapat nol baris (AC-19).
        let lama: Option<String> = sqlx::query_scalar(
            "WITH lama AS ( \
               SELECT request_id, status FROM request \
                WHERE request_id = $1 AND client_id = $2 \
                  AND status IN ('queued', 'dispatched') AND attempt_count = 0 \
                FOR UPDATE) \
             UPDATE request r \
                SET status = 'cancelled', cancel_reason = 'dibatalkan_client', \
                    cancelled_by = 'client', completed_at = now(), \
                    webhook_status = CASE WHEN EXISTS ( \
                        SELECT 1 FROM client_webhook w WHERE w.client_id = $2 AND w.aktif) \
                      THEN 'menunggu' ELSE 'tidak_dikirim' END \
               FROM lama \
              WHERE r.request_id = lama.request_id AND r.status = lama.status \
             RETURNING lama.status",
        )
        .bind(id)
        .bind(klien.client_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %ringkas(&e), "downstream: pembatalan gagal");
            GalatDownstream::Layanan
        })?;
        let Some(lama) = lama else {
            tx.rollback().await.map_err(layanan)?;
            return Ok(match self.status_dalam(klien, id).await? {
                None => HasilBatal::TidakAda,
                Some(s) => HasilBatal::TidakBisa(s),
            });
        };
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor) \
             SELECT $1, coalesce(max(seq), 0) + 1, $2, 'cancelled', 'dibatalkan_client', 'client' \
               FROM request_status_history WHERE request_id = $1",
        )
        .bind(id)
        .bind(&lama)
        .execute(&mut *tx)
        .await
        .map_err(layanan)?;
        // P-02 di transaksi transisi terminal yang sama (§3).
        sqlx::query("SELECT fn_purge_p02($1)")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(layanan)?;
        tx.commit().await.map_err(layanan)?;
        Ok(HasilBatal::Dibatalkan)
    }

    async fn mulai_sync_dalam(
        &self,
        klien: SesiKlien,
        rantai: &RantaiEndpoint,
        m: MulaiSync,
    ) -> Hasil<()> {
        let mut tx = self.pool.begin().await.map_err(layanan)?;
        sqlx::query(
            "INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, \
               downstream_pool_id, provider_id, upstream_pool_id, provider_config_version, \
               endpoint_config_version, mode, prioritas, prioritas_efektif, status, \
               referensi_client, input_payload, metadata, received_at, first_attempt_at, \
               attempt_count) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'sync', $10, $10, 'in_flight', $11, $12, \
                     $13, $14, $15, 1)",
        )
        .bind(m.request_id)
        .bind(klien.client_id)
        .bind(klien.credential_id)
        .bind(rantai.endpoint_id)
        .bind(rantai.downstream_pool_id)
        .bind(rantai.provider_id)
        .bind(m.upstream_pool_id)
        .bind(rantai.provider_config_version)
        .bind(rantai.endpoint_config_version)
        .bind(rantai.prioritas)
        .bind(m.referensi_client.as_deref())
        .bind(Json(m.input.clone()))
        .bind(Json(m.metadata.clone()))
        .bind(m.received_at)
        .bind(m.requested_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %ringkas(&e), "downstream: insert in_flight gagal");
            GalatDownstream::Layanan
        })?;
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) \
             VALUES ($1, 1, NULL, 'in_flight', 'webserver')",
        )
        .bind(m.request_id)
        .execute(&mut *tx)
        .await
        .map_err(layanan)?;
        sqlx::query(
            "INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, \
               upstream_pool_id, client_id, downstream_endpoint_id, provider_config_version, \
               endpoint_config_version, nik, requested_at, log_mode, request_payload, worker_id) \
             VALUES ($1, $2, 1, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'webserver')",
        )
        .bind(m.kode_transaksi)
        .bind(m.request_id)
        .bind(rantai.provider_id)
        .bind(m.upstream_pool_id)
        .bind(klien.client_id)
        .bind(rantai.endpoint_id)
        .bind(rantai.provider_config_version)
        .bind(rantai.endpoint_config_version)
        .bind(m.nik.as_deref())
        .bind(m.requested_at)
        .bind(rantai.log_mode.as_str())
        .bind(m.request_payload.map(Json))
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %ringkas(&e), "downstream: insert log percobaan sync gagal");
            GalatDownstream::Layanan
        })?;
        tx.commit().await.map_err(layanan)
    }

    async fn selesai_sync_dalam(&self, klien: SesiKlien, s: SelesaiSync) -> Hasil<TulisSync> {
        let akhir = akhir_sync(&s.hasil);
        let (http, output) = match &s.hasil {
            HasilPanggilan::Berhasil { http_status, hasil } => (
                Some(i32::from(*http_status)),
                Some(serde_json::to_value(hasil).map_err(layanan)?),
            ),
            // R-07: kegagalan TLS tidak punya respons HTTP.
            HasilPanggilan::Gagal {
                kelas: ErrorClass::Tls,
                ..
            } => (None, None),
            HasilPanggilan::Gagal { http_status, .. } => (http_status.map(i32::from), None),
        };
        let mut tx = self.pool.begin().await.map_err(layanan)?;
        // Log dulu, dengan predikat R-03: respons terlambat tidak menimpa
        // `tidak_diketahui` dari reconciler.
        sqlx::query(
            "UPDATE transaction_log \
                SET responded_at = $2, duration_ms = $3, status_akses = $4, error_class = $5, \
                    provider_http_status = $6, response_payload = $7 \
              WHERE kode_transaksi = $1 AND status_akses IS NULL",
        )
        .bind(s.kode_transaksi)
        // `chk_tls`/R-07: tanpa respons HTTP, `responded_at` NULL.
        .bind(http.is_some().then_some(s.responded_at))
        .bind(s.duration_ms.max(0))
        .bind(akhir.status_akses)
        .bind(akhir.error_class)
        .bind(http)
        .bind(s.response_payload.clone().map(Json))
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %ringkas(&e), "downstream: log sync gagal dilengkapi");
            GalatDownstream::Layanan
        })?;
        let menang: Option<Uuid> = sqlx::query_scalar(
            "UPDATE request \
                SET status = $2, completed_at = $3, final_kode_transaksi = $4, \
                    output_payload = $5, failure_reason = $6, sumber_hasil = $7 \
              WHERE request_id = $1 AND status = 'in_flight' \
             RETURNING request_id",
        )
        .bind(s.request_id)
        .bind(akhir.status.as_str())
        .bind(s.responded_at)
        .bind(s.kode_transaksi)
        .bind(output.map(Json))
        .bind(akhir.failure_reason)
        .bind(output_sumber(&akhir))
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %ringkas(&e), "downstream: status akhir sync gagal ditulis");
            GalatDownstream::Layanan
        })?;
        if menang.is_none() {
            tx.commit().await.map_err(layanan)?;
            return Ok(TulisSync::Kalah(
                self.status_dalam(klien, s.request_id).await?,
            ));
        }
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor) \
             SELECT $1, coalesce(max(seq), 0) + 1, 'in_flight', $2, $3, 'webserver' \
               FROM request_status_history WHERE request_id = $1",
        )
        .bind(s.request_id)
        .bind(akhir.status.as_str())
        .bind(akhir.failure_reason)
        .execute(&mut *tx)
        .await
        .map_err(layanan)?;
        tx.commit().await.map_err(layanan)?;
        Ok(TulisSync::Ditulis)
    }
}

fn output_sumber(akhir: &AkhirSync) -> Option<&'static str> {
    (akhir.status == StatusInternal::Done).then_some("provider")
}

/// Isi ulang limiter penolakan dalam mikrodetik per token, dari kolom
/// `limiter_penolakan_isi_ulang_per_detik` (L-03).
pub fn isi_ulang_mikro(per_detik: f64) -> i64 {
    if per_detik <= 0.0 {
        return poolrouter_core::kuota::store::ISI_ULANG_PENOLAKAN_MIKRO;
    }
    ((1_000_000.0 / per_detik).ceil() as i64).max(1)
}

/// Pesan galat sqlx tanpa nilai bind: hanya kode SQLSTATE dan nama constraint,
/// supaya log tidak pernah memuat NIK atau input (AC-4).
fn ringkas(e: &sqlx::Error) -> String {
    match e {
        sqlx::Error::Database(d) => format!(
            "sqlstate={} constraint={}",
            d.code().unwrap_or_default(),
            d.constraint().unwrap_or("-")
        ),
        lain => lain.to_string().chars().take(80).collect(),
    }
}

impl PenyimpanPermintaan for PgPermintaan {
    fn muat_rantai(
        &self,
        klien: SesiKlien,
        endpoint: Uuid,
        sekarang: DateTime<Utc>,
    ) -> Fut<'_, Hasil<Option<RantaiEndpoint>>> {
        Box::pin(async move { self.muat_rantai_dalam(klien, endpoint, sekarang).await })
    }

    fn cari_idempoten(
        &self,
        klien: SesiKlien,
        kunci: String,
    ) -> Fut<'_, Hasil<Option<BarisIdempoten>>> {
        Box::pin(async move { self.cari_idempoten_dalam(klien, &kunci).await })
    }

    fn simpan_ditolak(
        &self,
        klien: SesiKlien,
        rantai: Arc<RantaiEndpoint>,
        tolak: Penolakan,
    ) -> Fut<'_, Hasil<()>> {
        Box::pin(async move { self.simpan_ditolak_dalam(klien, &rantai, tolak).await })
    }

    fn simpan_antre(
        &self,
        klien: SesiKlien,
        rantai: Arc<RantaiEndpoint>,
        antrean: Antrean,
    ) -> Fut<'_, Hasil<HasilAntre>> {
        Box::pin(async move { self.simpan_antre_dalam(klien, &rantai, antrean).await })
    }

    fn status(&self, klien: SesiKlien, request_id: Uuid) -> Fut<'_, Hasil<Option<BarisStatus>>> {
        Box::pin(async move { self.status_dalam(klien, request_id).await })
    }

    fn batalkan(&self, klien: SesiKlien, request_id: Uuid) -> Fut<'_, Hasil<HasilBatal>> {
        Box::pin(async move { self.batalkan_dalam(klien, request_id).await })
    }

    fn mulai_sync(
        &self,
        klien: SesiKlien,
        rantai: Arc<RantaiEndpoint>,
        mulai: MulaiSync,
    ) -> Fut<'_, Hasil<()>> {
        Box::pin(async move { self.mulai_sync_dalam(klien, &rantai, mulai).await })
    }

    fn selesai_sync(&self, klien: SesiKlien, selesai: SelesaiSync) -> Fut<'_, Hasil<TulisSync>> {
        Box::pin(async move { self.selesai_sync_dalam(klien, selesai).await })
    }
}

pub mod fake {
    //! Penyimpan di memori untuk uji router: aturan compare-and-set yang sama
    //! (status lama + `attempt_count = 0` untuk pembatalan), tanpa database.

    use std::sync::Mutex;

    use super::*;

    #[derive(Debug, Clone)]
    pub struct BarisFake {
        pub client_id: Uuid,
        pub mode: Mode,
        pub status: StatusInternal,
        pub attempt_count: i32,
        pub reject_reason: Option<&'static str>,
        pub reject_http: Option<u16>,
        pub idempoten: Option<(String, String)>,
        pub input: Option<Value>,
        pub metadata: Value,
        pub failure_reason: Option<String>,
        pub cancel_reason: Option<String>,
        pub output: Option<Value>,
        pub history: Vec<StatusInternal>,
        pub log: Vec<(Uuid, Option<&'static str>)>,
    }

    #[derive(Default)]
    pub struct FakePermintaan {
        pub rantai: Mutex<Option<RantaiEndpoint>>,
        pub baris: Mutex<BTreeMap<Uuid, BarisFake>>,
        pub mati: Mutex<bool>,
    }

    impl FakePermintaan {
        pub fn baru() -> Arc<Self> {
            Arc::new(Self::default())
        }

        pub fn atur_rantai(&self, rantai: RantaiEndpoint) {
            *self.rantai.lock().unwrap() = Some(rantai);
        }

        pub fn ubah_rantai(&self, f: impl FnOnce(&mut RantaiEndpoint)) {
            if let Some(r) = self.rantai.lock().unwrap().as_mut() {
                f(r);
            }
        }

        pub fn matikan(&self) {
            *self.mati.lock().unwrap() = true;
        }

        /// Baris request yang tersimpan, urut `request_id` (UUIDv7 = waktu).
        pub fn semua(&self) -> Vec<(Uuid, BarisFake)> {
            self.baris
                .lock()
                .unwrap()
                .iter()
                .map(|(k, v)| (*k, v.clone()))
                .collect()
        }

        /// Menyisipkan baris langsung, untuk menyiapkan status yang tidak bisa
        /// dicapai lewat API (mis. `in_flight` async atau menunggu retry).
        pub fn sisipkan(&self, id: Uuid, baris: BarisFake) {
            self.baris.lock().unwrap().insert(id, baris);
        }

        fn cek(&self) -> Hasil<()> {
            if *self.mati.lock().unwrap() {
                Err(GalatDownstream::Layanan)
            } else {
                Ok(())
            }
        }

        fn status_baris(id: Uuid, b: &BarisFake) -> BarisStatus {
            BarisStatus {
                request_id: id,
                status: b.status,
                attempt_count: b.attempt_count,
                failure_reason: b.failure_reason.clone(),
                cancel_reason: b.cancel_reason.clone(),
                output: b.output.clone(),
            }
        }
    }

    pub fn baris_kosong(client_id: Uuid, mode: Mode, status: StatusInternal) -> BarisFake {
        BarisFake {
            client_id,
            mode,
            status,
            attempt_count: 0,
            reject_reason: None,
            reject_http: None,
            idempoten: None,
            input: None,
            metadata: Value::Object(serde_json::Map::new()),
            failure_reason: None,
            cancel_reason: None,
            output: None,
            history: vec![status],
            log: Vec::new(),
        }
    }

    impl PenyimpanPermintaan for FakePermintaan {
        fn muat_rantai(
            &self,
            _klien: SesiKlien,
            endpoint: Uuid,
            _sekarang: DateTime<Utc>,
        ) -> Fut<'_, Hasil<Option<RantaiEndpoint>>> {
            Box::pin(async move {
                self.cek()?;
                Ok(self
                    .rantai
                    .lock()
                    .unwrap()
                    .clone()
                    .filter(|r| r.endpoint_id == endpoint))
            })
        }

        fn cari_idempoten(
            &self,
            klien: SesiKlien,
            kunci: String,
        ) -> Fut<'_, Hasil<Option<BarisIdempoten>>> {
            Box::pin(async move {
                self.cek()?;
                Ok(self
                    .baris
                    .lock()
                    .unwrap()
                    .iter()
                    .find_map(|(id, b)| match &b.idempoten {
                        Some((k, s)) if b.client_id == klien.client_id && *k == kunci => {
                            Some(BarisIdempoten {
                                request_id: *id,
                                status: b.status,
                                attempt_count: b.attempt_count,
                                sidik: s.clone(),
                            })
                        }
                        _ => None,
                    }))
            })
        }

        fn simpan_ditolak(
            &self,
            klien: SesiKlien,
            _rantai: Arc<RantaiEndpoint>,
            t: Penolakan,
        ) -> Fut<'_, Hasil<()>> {
            Box::pin(async move {
                self.cek()?;
                let mut b = baris_kosong(klien.client_id, t.mode, StatusInternal::Rejected);
                b.reject_reason = Some(t.reject_reason);
                b.reject_http = Some(t.http_status);
                b.metadata = t.metadata;
                self.baris.lock().unwrap().insert(t.request_id, b);
                Ok(())
            })
        }

        fn simpan_antre(
            &self,
            klien: SesiKlien,
            _rantai: Arc<RantaiEndpoint>,
            a: Antrean,
        ) -> Fut<'_, Hasil<HasilAntre>> {
            Box::pin(async move {
                self.cek()?;
                let mut semua = self.baris.lock().unwrap();
                if let Some((k, _)) = &a.idempoten {
                    let ada = semua.iter().find_map(|(id, b)| match &b.idempoten {
                        Some((kk, s)) if b.client_id == klien.client_id && kk == k => {
                            Some(BarisIdempoten {
                                request_id: *id,
                                status: b.status,
                                attempt_count: b.attempt_count,
                                sidik: s.clone(),
                            })
                        }
                        _ => None,
                    });
                    if let Some(ada) = ada {
                        return Ok(HasilAntre::Ada(ada));
                    }
                }
                let mut b = baris_kosong(klien.client_id, Mode::Async, StatusInternal::Queued);
                b.idempoten = a.idempoten;
                b.input = Some(a.input);
                b.metadata = a.metadata;
                semua.insert(a.request_id, b);
                Ok(HasilAntre::Baru)
            })
        }

        fn status(
            &self,
            klien: SesiKlien,
            request_id: Uuid,
        ) -> Fut<'_, Hasil<Option<BarisStatus>>> {
            Box::pin(async move {
                self.cek()?;
                Ok(self
                    .baris
                    .lock()
                    .unwrap()
                    .get(&request_id)
                    .filter(|b| {
                        b.client_id == klien.client_id && b.status != StatusInternal::Rejected
                    })
                    .map(|b| Self::status_baris(request_id, b)))
            })
        }

        fn batalkan(&self, klien: SesiKlien, request_id: Uuid) -> Fut<'_, Hasil<HasilBatal>> {
            Box::pin(async move {
                self.cek()?;
                let mut semua = self.baris.lock().unwrap();
                let Some(b) = semua.get_mut(&request_id).filter(|b| {
                    b.client_id == klien.client_id && b.status != StatusInternal::Rejected
                }) else {
                    return Ok(HasilBatal::TidakAda);
                };
                let boleh = matches!(
                    b.status,
                    StatusInternal::Queued | StatusInternal::Dispatched
                ) && b.attempt_count == 0;
                if !boleh {
                    return Ok(HasilBatal::TidakBisa(Self::status_baris(request_id, b)));
                }
                b.status = StatusInternal::Cancelled;
                b.cancel_reason = Some("dibatalkan_client".to_owned());
                b.history.push(StatusInternal::Cancelled);
                // P-02.
                b.input = None;
                Ok(HasilBatal::Dibatalkan)
            })
        }

        fn mulai_sync(
            &self,
            klien: SesiKlien,
            _rantai: Arc<RantaiEndpoint>,
            m: MulaiSync,
        ) -> Fut<'_, Hasil<()>> {
            Box::pin(async move {
                self.cek()?;
                let mut b = baris_kosong(klien.client_id, Mode::Sync, StatusInternal::InFlight);
                b.attempt_count = 1;
                b.input = Some(m.input);
                b.metadata = m.metadata;
                b.log.push((m.kode_transaksi, None));
                self.baris.lock().unwrap().insert(m.request_id, b);
                Ok(())
            })
        }

        fn selesai_sync(&self, klien: SesiKlien, s: SelesaiSync) -> Fut<'_, Hasil<TulisSync>> {
            Box::pin(async move {
                self.cek()?;
                let akhir = akhir_sync(&s.hasil);
                let mut semua = self.baris.lock().unwrap();
                let Some(b) = semua
                    .get_mut(&s.request_id)
                    .filter(|b| b.client_id == klien.client_id)
                else {
                    return Ok(TulisSync::Kalah(None));
                };
                for l in &mut b.log {
                    if l.0 == s.kode_transaksi && l.1.is_none() {
                        l.1 = Some(akhir.status_akses);
                    }
                }
                if b.status != StatusInternal::InFlight {
                    return Ok(TulisSync::Kalah(Some(Self::status_baris(s.request_id, b))));
                }
                b.status = akhir.status;
                b.failure_reason = akhir.failure_reason.map(str::to_owned);
                if let HasilPanggilan::Berhasil { hasil, .. } = &s.hasil {
                    b.output = serde_json::to_value(hasil).ok();
                }
                b.history.push(akhir.status);
                Ok(TulisSync::Ditulis)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kontrak_input_hanya_dari_namespace_input() {
        let t = "{\"NIK\":\"{{input.NIK}}\",\"NAMA_LGKP\":\"{{ input.NAMA_LGKP }}\",\
                 \"TRESHOLD\":\"{{endpoint.TRESHOLD}}\",\"USER_ID\":\"{{secret.USER_ID}}\",\
                 \"transactionId\":\"{{system.kode_transaksi}}\",\"x\":\"{{input.NIK}}\"}";
        let nama: Vec<String> = field_input_dari_template(t)
            .into_iter()
            .map(|f| f.nama)
            .collect();
        assert_eq!(nama, vec!["NIK".to_owned(), "NAMA_LGKP".to_owned()]);
    }

    #[test]
    fn akhir_sync_mengikuti_tabel_adr() {
        let gagal = |kelas| HasilPanggilan::Gagal {
            kelas,
            http_status: None,
        };
        assert_eq!(
            akhir_sync(&gagal(ErrorClass::Timeout)).failure_reason,
            Some("timeout_sync")
        );
        for k in [
            ErrorClass::Koneksi,
            ErrorClass::RateLimitProvider,
            ErrorClass::ProviderSementara,
        ] {
            assert_eq!(akhir_sync(&gagal(k)).failure_reason, Some("retry_habis"));
        }
        for k in [
            ErrorClass::Tls,
            ErrorClass::ResponsTidakValid,
            ErrorClass::AuthDitolak,
            ErrorClass::KuotaProviderHabis,
            ErrorClass::Lainnya,
        ] {
            assert_eq!(
                akhir_sync(&gagal(k)).failure_reason,
                Some("provider_error_permanen")
            );
        }
        assert_eq!(
            akhir_sync(&gagal(ErrorClass::Tls)).status_akses,
            "gagal_jaringan"
        );
        assert_eq!(
            akhir_sync(&gagal(ErrorClass::AuthDitolak)).status_akses,
            "gagal_provider"
        );
        let ok = akhir_sync(&HasilPanggilan::Berhasil {
            http_status: 200,
            hasil: BTreeMap::new(),
        });
        assert_eq!(ok.status, StatusInternal::Done);
        assert_eq!(ok.status_akses, "berhasil");
    }

    #[test]
    fn isi_ulang_limiter_dari_per_detik() {
        assert_eq!(isi_ulang_mikro(1.0), 1_000_000);
        assert_eq!(isi_ulang_mikro(4.0), 250_000);
        assert_eq!(isi_ulang_mikro(0.0), 1_000_000);
    }

    #[test]
    fn kuota_efektif_dibulatkan_ke_bawah_minimal_satu() {
        let mut r = PoolRute {
            upstream_pool_id: Uuid::nil(),
            kuota_harian: 1000,
            margin_kuota: 0.01,
            kalender: false,
            periode_mulai: Utc::now(),
            periode_selesai: Utc::now(),
            porsi_sync_maks: 0.2,
            retry_rasio: 0.1,
            retry_min_per_jam: 20,
            parameter_dispatch: Value::Null,
        };
        assert_eq!(r.kuota_efektif(), 990);
        r.kuota_harian = 1;
        assert_eq!(r.kuota_efektif(), 1);
    }
}
