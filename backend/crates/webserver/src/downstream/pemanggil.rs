//! Panggilan provider untuk jalur sync (ARC-66, ADR-0001 rev 15 §4, §5a.4;
//! ADR-0002 D1–D5).
//!
//! Dua tahap, dan alasannya:
//!
//! 1. [`PemanggilProvider::siapkan`]: semua langkah yang bisa gagal karena
//!    konfigurasi (dokumen provider/endpoint pada versi aktif, secret, render
//!    template, klien TLS, allowlist) dijalankan **sebelum** kuota disentuh dan
//!    sebelum baris apa pun ditulis. Gagal di sini = 503 tanpa baris dan tanpa
//!    kuota, sama dengan worker yang menolak sebelum CAS (M-03: tidak ada baris
//!    log untuk panggilan yang tidak pernah keluar).
//! 2. [`PanggilanSiap::kirim`]: satu panggilan HTTP, dijalankan handler di
//!    dalam `tokio::spawn` setelah log percobaan ter-commit (AC-27, AC-29).
//!
//! Implementasi produksi [`PemanggilHttp`] memakai kode yang sama dengan worker
//! (`poolrouter_adapter_provider::dokumen` untuk render dan penyamaran secret,
//! `::hasil` untuk pemetaan HTTP/transport ke `error_class`) dan klien TLS per
//! versi provider dari `::client`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poolrouter_adapter_provider::client::{
    Allowlist, ProviderClientConfig, TlsTrust, build_provider_client_with_config, kirim,
};
use poolrouter_adapter_provider::dokumen::{self, DokumenProvider};
use poolrouter_adapter_provider::hasil::{Akhir, nilai_hasil};
use poolrouter_core::retry::ErrorClass;
use poolrouter_core::secret::{Amplop, Brankas};
use serde_json::Value;
use sqlx::types::Json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::downstream_sesi::{Fut, GalatDownstream, Hasil};
use super::permintaan::HasilPanggilan;
use crate::api::downstream::HasilElemen;

/// Satu percobaan sync. `input` hanya hidup di memori proses; implementasi
/// tidak boleh mencatatnya di log.
#[derive(Clone)]
pub struct PanggilanProvider {
    pub request_id: Uuid,
    /// `transactionId` untuk provider (K-25, ADR-0002 D2), UUIDv7 per panggilan.
    pub kode_transaksi: Uuid,
    pub provider_id: Uuid,
    pub endpoint_id: Uuid,
    pub provider_config_version: i32,
    pub endpoint_config_version: i32,
    /// `log_mode` efektif `full_payload`: `request_payload`/`response_payload`
    /// diisi keluaran log (secret sebagai penanda), selain itu NULL (R-05).
    pub full_payload: bool,
    pub input: Value,
}

impl std::fmt::Debug for PanggilanProvider {
    // `input` memuat NIK; `Debug` tidak pernah mencetaknya (AC-4).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PanggilanProvider")
            .field("request_id", &self.request_id)
            .field("kode_transaksi", &self.kode_transaksi)
            .field("provider_id", &self.provider_id)
            .finish_non_exhaustive()
    }
}

/// Hasil satu panggilan beserta `transaction_log.response_payload`.
#[derive(Debug, Clone)]
pub struct HasilProvider {
    pub hasil: HasilPanggilan,
    /// Hanya pada `full_payload` dan bila provider menjawab; nilai secret yang
    /// digemakan provider sudah diganti penanda.
    pub response_payload: Option<Value>,
}

/// Panggilan yang tertunda: menerima timeout total upstream pool terpilih.
pub type Kirim = Box<dyn FnOnce(Duration) -> Fut<'static, HasilProvider> + Send>;

/// Panggilan yang sudah dirender dan siap dikirim.
pub struct PanggilanSiap {
    /// `transaction_log.request_payload` (keluaran log), `None` pada
    /// `metadata_only`.
    pub request_payload: Option<Value>,
    pub kirim: Kirim,
}

pub trait PemanggilProvider: Send + Sync + 'static {
    /// Tahap 1. `Err` hanya `Layanan` (503) dan tidak pernah memuat detail.
    fn siapkan(&self, p: PanggilanProvider) -> Fut<'_, Hasil<PanggilanSiap>>;
}

// ------------------------------------------------------------------ port

/// Dokumen konfigurasi dan secret provider. Produksi: [`SumberPg`] (`pr_api`).
pub trait SumberPanggilan: Send + Sync + 'static {
    /// `provider_config_versi.dokumen` pada versi itu.
    fn dokumen_provider(&self, provider_id: Uuid, versi: i32) -> Fut<'_, Hasil<Value>>;
    /// `endpoint_config_versi.dokumen` pada versi itu (sumber `parameter_literal`).
    fn dokumen_endpoint(&self, endpoint_id: Uuid, versi: i32) -> Fut<'_, Hasil<Option<Value>>>;
    /// Nilai setiap secret yang disebut; satu yang hilang = galat.
    fn secret(&self, nama: Vec<String>) -> Fut<'_, Hasil<BTreeMap<String, String>>>;
}

/// Klien HTTP satu versi provider beserta allowlist tujuan (ADR-0002 D4, D5).
pub struct KlienProvider {
    http: reqwest::Client,
    allowlist: Option<Allowlist>,
}

impl KlienProvider {
    /// Produksi: trust TLS dari dokumen provider, `https_only`, tanpa redirect,
    /// dan allowlist host:port dari `base_url`.
    ///
    /// Hanya `system` yang punya bahan lengkap. Admin API (dan kontraknya) belum
    /// menyimpan PEM CA atau pin SPKI di dokumen provider, jadi `ca_kustom` dan
    /// `pin_spki` ditolak di sini (fail-closed, tanpa koneksi keluar), sama
    /// dengan worker. Lihat catatan keputusan Operator ARC-66.
    pub fn tls(dok: &DokumenProvider) -> Result<Self, String> {
        let trust = match TlsTrust::dari_str(&dok.tls_trust) {
            Some(TlsTrust::System) => TlsTrust::System,
            Some(_) => {
                return Err(format!(
                    "tls_trust {} belum punya bahan CA/pin di dokumen provider",
                    dok.tls_trust
                ));
            }
            None => return Err(format!("tls_trust {} tidak dikenal", dok.tls_trust)),
        };
        let allowlist = Allowlist::dari_base_url(&dok.base_url).map_err(|e| e.to_string())?;
        let http = build_provider_client_with_config(trust, ProviderClientConfig::default())
            .map_err(|e| e.to_string())?;
        Ok(Self {
            http,
            allowlist: Some(allowlist),
        })
    }

    /// Klien tanpa TLS dan tanpa allowlist, khusus uji terhadap mock HTTP
    /// lokal. Produksi selalu lewat [`KlienProvider::tls`] ([`TransportTls`]).
    pub fn tanpa_tls_untuk_uji(http: reqwest::Client) -> Self {
        Self {
            http,
            allowlist: None,
        }
    }

    fn izinkan(&self, url: &str) -> bool {
        match &self.allowlist {
            None => true,
            Some(a) => reqwest::Url::parse(url).is_ok_and(|u| a.izinkan(&u)),
        }
    }
}

/// Cara membangun [`KlienProvider`] untuk satu versi dokumen provider.
pub trait Transport: Send + Sync + 'static {
    fn klien(&self, dok: &DokumenProvider) -> Result<KlienProvider, String>;
}

/// Transport produksi: [`KlienProvider::tls`].
pub struct TransportTls;

impl Transport for TransportTls {
    fn klien(&self, dok: &DokumenProvider) -> Result<KlienProvider, String> {
        KlienProvider::tls(dok)
    }
}

// ------------------------------------------------------------------ produksi

struct ProviderSiap {
    dok: DokumenProvider,
    klien: KlienProvider,
}

type KunciVersi = (Uuid, i32);
type CacheProvider = Mutex<HashMap<KunciVersi, Arc<ProviderSiap>>>;
type CacheLiteral = Mutex<HashMap<KunciVersi, Arc<BTreeMap<String, String>>>>;

/// [`PemanggilProvider`] di atas adapter provider.
///
/// Dokumen versi konfigurasi tidak pernah berubah setelah ditulis, jadi dokumen
/// yang sudah dibaca dan klien HTTP-nya di-cache per `(id, versi)`: satu klien
/// dan connection pool per versi provider, dibangun ulang saat versi berganti
/// (ADR-0002 D5). Secret dibaca ulang setiap panggilan supaya rotasi langsung
/// berlaku.
pub struct PemanggilHttp<S, T> {
    sumber: S,
    transport: T,
    provider: CacheProvider,
    literal: CacheLiteral,
}

impl<S: SumberPanggilan, T: Transport> PemanggilHttp<S, T> {
    pub fn baru(sumber: S, transport: T) -> Self {
        Self {
            sumber,
            transport,
            provider: Mutex::new(HashMap::new()),
            literal: Mutex::new(HashMap::new()),
        }
    }

    async fn provider(&self, id: Uuid, versi: i32) -> Hasil<Arc<ProviderSiap>> {
        let ada = self
            .provider
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(id, versi))
            .cloned();
        if let Some(p) = ada {
            return Ok(p);
        }
        let mentah = self.sumber.dokumen_provider(id, versi).await?;
        let dok = dokumen::baca_dokumen_provider(&mentah).map_err(|e| {
            // Pesan hanya menyebut kunci yang bermasalah, tidak pernah nilai.
            tracing::error!(provider_id = %id, versi, error = %e,
                "sync: dokumen provider tidak bisa dipakai");
            GalatDownstream::Layanan
        })?;
        let klien = self.transport.klien(&dok).map_err(|e| {
            tracing::error!(provider_id = %id, versi, error = %e,
                "sync: klien provider tidak bisa dibangun");
            GalatDownstream::Layanan
        })?;
        let siap = Arc::new(ProviderSiap { dok, klien });
        self.provider
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((id, versi), Arc::clone(&siap));
        Ok(siap)
    }

    async fn literal(&self, id: Uuid, versi: i32) -> Hasil<Arc<BTreeMap<String, String>>> {
        let ada = self
            .literal
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(id, versi))
            .cloned();
        if let Some(l) = ada {
            return Ok(l);
        }
        let dok = self.sumber.dokumen_endpoint(id, versi).await?;
        let literal = Arc::new(dokumen::parameter_literal(dok.as_ref()));
        self.literal
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((id, versi), Arc::clone(&literal));
        Ok(literal)
    }

    async fn siapkan_dalam(&self, p: PanggilanProvider) -> Hasil<PanggilanSiap> {
        let prov = self
            .provider(p.provider_id, p.provider_config_version)
            .await?;
        let literal = self
            .literal(p.endpoint_id, p.endpoint_config_version)
            .await?;
        let input = dokumen::nilai_input(Some(&p.input));
        let nama =
            dokumen::nama_secret(&[prov.dok.base_url.as_str(), prov.dok.body_template.as_str()]);
        let secret = self.sumber.secret(nama).await?;
        let rakitan = dokumen::rakit(&prov.dok, &input, &literal, &secret, p.kode_transaksi)
            .map_err(|e| {
                // `RenderError` hanya menyebut nama placeholder, bukan nilai.
                tracing::error!(provider_id = %p.provider_id, error = %e,
                    "sync: render template provider gagal");
                GalatDownstream::Layanan
            })?;
        if !prov.klien.izinkan(&rakitan.kabel_url) {
            tracing::error!(provider_id = %p.provider_id,
                "sync: URL hasil render di luar allowlist provider");
            return Err(GalatDownstream::Layanan);
        }
        let request_payload = p
            .full_payload
            .then(|| dokumen::payload_permintaan(&prov.dok.metode, &rakitan));
        let full = p.full_payload;
        let url = rakitan.kabel_url.clone();
        let body = rakitan.kabel_body.clone();
        let kirim_ini: Kirim = Box::new(move |batas: Duration| -> Fut<'static, HasilProvider> {
            Box::pin(async move {
                let hasil = kirim(&prov.klien.http, &prov.dok.metode, &url, body, batas).await;
                let akhir = nilai_hasil(&hasil, &prov.dok.respons, &input);
                let response_payload = if full {
                    hasil
                        .as_ref()
                        .ok()
                        .map(|r| dokumen::payload_respons(&r.body, &secret))
                } else {
                    None
                };
                HasilProvider {
                    hasil: hasil_panggilan(akhir),
                    response_payload,
                }
            })
        });
        Ok(PanggilanSiap {
            request_payload,
            kirim: kirim_ini,
        })
    }
}

impl<S: SumberPanggilan, T: Transport> PemanggilProvider for PemanggilHttp<S, T> {
    fn siapkan(&self, p: PanggilanProvider) -> Fut<'_, Hasil<PanggilanSiap>> {
        Box::pin(async move { self.siapkan_dalam(p).await })
    }
}

/// [`Akhir`] (bentuk worker) → [`HasilPanggilan`] (bentuk handler). Output
/// sukses berupa peta elemen → `{status, skor}` yang sama dengan
/// `output_payload` worker.
pub fn hasil_panggilan(akhir: Akhir) -> HasilPanggilan {
    match (akhir.error_class, akhir.output) {
        (None, Some(output)) => {
            match serde_json::from_value::<BTreeMap<String, HasilElemen>>(output) {
                Ok(hasil) => HasilPanggilan::Berhasil {
                    http_status: akhir.http_status.unwrap_or(200),
                    hasil,
                },
                Err(_) => HasilPanggilan::Gagal {
                    kelas: ErrorClass::ResponsTidakValid,
                    http_status: akhir.http_status,
                },
            }
        }
        (kelas, _) => HasilPanggilan::Gagal {
            kelas: kelas.unwrap_or(ErrorClass::Lainnya),
            http_status: akhir.http_status,
        },
    }
}

/// [`SumberPanggilan`] di PostgreSQL sebagai `pr_api` (grant SELECT
/// `provider_config_versi`, `endpoint_config_versi`, `provider_secret`).
/// Secret dibuka di proses dengan KEK (`POOLROUTER_KEK`), seperti worker.
pub struct SumberPg {
    pool: PgPool,
    brankas: Brankas,
}

impl SumberPg {
    pub fn baru(pool: PgPool, brankas: Brankas) -> Self {
        Self { pool, brankas }
    }

    async fn dokumen_provider_dalam(&self, id: Uuid, versi: i32) -> Hasil<Value> {
        let dok: Option<Json<Value>> = sqlx::query_scalar(
            "SELECT dokumen FROM provider_config_versi WHERE provider_id = $1 AND versi = $2",
        )
        .bind(id)
        .bind(versi)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GalatDownstream::Layanan)?;
        dok.map(|j| j.0).ok_or(GalatDownstream::Layanan)
    }

    async fn dokumen_endpoint_dalam(&self, id: Uuid, versi: i32) -> Hasil<Option<Value>> {
        let dok: Option<Json<Value>> = sqlx::query_scalar(
            "SELECT dokumen FROM endpoint_config_versi \
              WHERE downstream_endpoint_id = $1 AND versi = $2",
        )
        .bind(id)
        .bind(versi)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GalatDownstream::Layanan)?;
        Ok(dok.map(|j| j.0))
    }

    async fn secret_dalam(&self, nama: Vec<String>) -> Hasil<BTreeMap<String, String>> {
        let mut peta = BTreeMap::new();
        if nama.is_empty() {
            return Ok(peta);
        }
        let baris = sqlx::query(
            "SELECT nama_secret, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id \
               FROM provider_secret WHERE nama_secret = ANY($1)",
        )
        .bind(nama.clone())
        .fetch_all(&self.pool)
        .await
        .map_err(|_| GalatDownstream::Layanan)?;
        for b in &baris {
            let nonce: Vec<u8> = b
                .try_get("secret_nonce")
                .map_err(|_| GalatDownstream::Layanan)?;
            let amplop = Amplop {
                nama: b
                    .try_get("nama_secret")
                    .map_err(|_| GalatDownstream::Layanan)?,
                ciphertext: b
                    .try_get("secret_ciphertext")
                    .map_err(|_| GalatDownstream::Layanan)?,
                dek_terbungkus: b
                    .try_get("secret_dek_terbungkus")
                    .map_err(|_| GalatDownstream::Layanan)?,
                nonce: <[u8; 12]>::try_from(nonce.as_slice())
                    .map_err(|_| GalatDownstream::Layanan)?,
                kek_id: b.try_get("kek_id").map_err(|_| GalatDownstream::Layanan)?,
            };
            let nilai = self.brankas.dekripsi(&amplop).map_err(|_| {
                tracing::error!(secret = %amplop.nama, "sync: secret provider tidak bisa dibuka");
                GalatDownstream::Layanan
            })?;
            peta.insert(amplop.nama.clone(), nilai.ke_dalam());
        }
        if let Some(hilang) = nama.iter().find(|n| !peta.contains_key(*n)) {
            tracing::error!(secret = %hilang, "sync: secret provider tidak ada");
            return Err(GalatDownstream::Layanan);
        }
        Ok(peta)
    }
}

impl SumberPanggilan for SumberPg {
    fn dokumen_provider(&self, provider_id: Uuid, versi: i32) -> Fut<'_, Hasil<Value>> {
        Box::pin(async move { self.dokumen_provider_dalam(provider_id, versi).await })
    }

    fn dokumen_endpoint(&self, endpoint_id: Uuid, versi: i32) -> Fut<'_, Hasil<Option<Value>>> {
        Box::pin(async move { self.dokumen_endpoint_dalam(endpoint_id, versi).await })
    }

    fn secret(&self, nama: Vec<String>) -> Fut<'_, Hasil<BTreeMap<String, String>>> {
        Box::pin(async move { self.secret_dalam(nama).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::enums::StatusHasil;
    use serde_json::json;

    fn dok(base_url: &str, tls: &str) -> DokumenProvider {
        dokumen::baca_dokumen_provider(&json!({
            "metode": "POST",
            "base_url": base_url,
            "body_template": "{}",
            "tls_trust": tls,
            "respons_skema": {"parser": "status_skor", "literal": [], "parameter_output": []}
        }))
        .unwrap()
    }

    #[test]
    fn output_worker_menjadi_hasil_elemen_api() {
        let akhir = Akhir {
            status_akses: "berhasil",
            error_class: None,
            http_status: Some(200),
            ada_respons: true,
            output: Some(json!({
                "NAMA_LGKP": {"status": "sesuai", "skor": 90},
                "TMPT_LHR": {"status": "tidak_sesuai", "skor": null}
            })),
        };
        let HasilPanggilan::Berhasil { http_status, hasil } = hasil_panggilan(akhir) else {
            panic!("harus berhasil");
        };
        assert_eq!(http_status, 200);
        assert_eq!(hasil["NAMA_LGKP"].status, StatusHasil::Sesuai);
        assert_eq!(hasil["NAMA_LGKP"].skor, Some(90));
        assert_eq!(hasil["TMPT_LHR"].status, StatusHasil::TidakSesuai);
        assert_eq!(hasil["TMPT_LHR"].skor, None);
    }

    #[test]
    fn gagal_membawa_kelas_dan_kode_http() {
        let akhir = Akhir {
            status_akses: "gagal_provider",
            error_class: Some(ErrorClass::ProviderSementara),
            http_status: Some(503),
            ada_respons: true,
            output: None,
        };
        let HasilPanggilan::Gagal { kelas, http_status } = hasil_panggilan(akhir) else {
            panic!("harus gagal");
        };
        assert_eq!(kelas, ErrorClass::ProviderSementara);
        assert_eq!(http_status, Some(503));
    }

    #[test]
    fn transport_produksi_menolak_http_dan_mode_tanpa_bahan() {
        // ADR-0002 D4/D5: egress hanya https ke host:port base_url.
        assert!(KlienProvider::tls(&dok("http://127.0.0.1:8000/api", "system")).is_err());
        for mode in ["ca_kustom", "pin_spki", "matikan"] {
            assert!(
                KlienProvider::tls(&dok("https://127.0.0.1:8000/api", mode)).is_err(),
                "{mode}"
            );
        }
    }

    #[test]
    fn allowlist_menolak_url_render_ke_host_lain() {
        let k = KlienProvider {
            http: reqwest::Client::new(),
            allowlist: Some(Allowlist::dari_base_url("https://10.0.0.1:8000/api").unwrap()),
        };
        assert!(k.izinkan("https://10.0.0.1:8000/api/x"));
        assert!(!k.izinkan("https://10.0.0.2:8000/api/x"));
        assert!(!k.izinkan("http://10.0.0.1:8000/api/x"));
        assert!(!k.izinkan("bukan url"));
    }
}
