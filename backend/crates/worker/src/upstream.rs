//! Port panggilan HTTP ke provider (ADR-0002 D1, D5; ADR-0001 bagian 5a.4).
//!
//! Handler hanya bergantung pada trait [`PemanggilProvider`]. Implementasi
//! produksi [`PemanggilReqwest`]:
//!
//! - tidak mengikuti redirect (redirect mengirim ulang body berkredensial ke
//!   host lain) dan tidak me-retry sendiri; retry dikendalikan PoolRouter;
//! - request dan pemetaan galat lewat `adapter_provider::client::kirim`
//!   (ARC-240): body `String` utuh ber-`Content-Length`, galat sertifikat
//!   menjadi `Tls` (tidak di-retry);
//! - timeout connect 5 s, total dari konfigurasi pool (bawaan 20 s);
//! - body respons dibaca paling banyak `BATAS_BODY_RESPONS` (ARC-241);
//! - trust TLS dari dokumen versi provider (ARC-239): `system` memakai klien
//!   bersama, `ca_kustom` dan `pin_spki` memakai klien adapter provider
//!   (`build_provider_client_with_config`) yang sama dengan jalur sync, satu
//!   klien per bahan trust. Bahan yang tidak bisa dibangun menjadi klien
//!   menjadi kegagalan `tls` (tidak di-retry), tidak pernah jatuh ke trust
//!   sistem.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use poolrouter_adapter_provider::GagalPanggilProvider;
use poolrouter_adapter_provider::client::{
    ProviderClientConfig, TlsTrust, build_provider_client_with_config, kirim,
};

/// Batas connect ke provider (ADR-0001 bagian 5a.4).
pub const BATAS_CONNECT: Duration = Duration::from_secs(5);

/// Permintaan siap kirim. URL dan body memuat secret dan NIK, jadi `Debug`
/// sengaja hanya menampilkan metode dan mode trust (bukan bahannya).
pub struct PermintaanKabel {
    pub metode: String,
    pub url: String,
    pub body: Option<String>,
    /// Trust TLS hasil `DokumenProvider::trust` (mode beserta bahannya).
    pub trust: TlsTrust,
    pub batas_total: Duration,
}

impl fmt::Debug for PermintaanKabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PermintaanKabel")
            .field("metode", &self.metode)
            .field("trust", &nama_mode(&self.trust))
            .finish()
    }
}

fn nama_mode(trust: &TlsTrust) -> &'static str {
    match trust {
        TlsTrust::System => "system",
        TlsTrust::CaKustom { .. } => "ca_kustom",
        TlsTrust::PinSpki { .. } => "pin_spki",
    }
}

/// Respons HTTP apa pun kodenya; dipindah ke adapter (ARC-66). Klasifikasi
/// ada di [`crate::keputusan`].
pub use poolrouter_adapter_provider::hasil::ResponsMentah;

/// Satu panggilan ke provider. `Err` hanya untuk kegagalan transport
/// (timeout, koneksi, TLS); respons HTTP non-2xx tetap `Ok`.
pub trait PemanggilProvider: Send + Sync {
    /// Siapkan klien untuk `trust` sebelum slot dan baris log disentuh, supaya
    /// bahan yang tidak bisa dibangun menjadi klien ditolak tanpa efek samping
    /// (M-03), sama dengan `siapkan` di jalur sync. Pesan galat tidak memuat
    /// bahan. Bawaan: tidak ada yang perlu disiapkan (pemanggil palsu).
    fn siapkan_trust(&self, _trust: &TlsTrust) -> Result<(), String> {
        Ok(())
    }

    fn panggil(
        &self,
        permintaan: &PermintaanKabel,
    ) -> impl Future<Output = Result<ResponsMentah, GagalPanggilProvider>> + Send;
}

/// Implementasi produksi di atas `reqwest`.
pub struct PemanggilReqwest {
    /// Klien `tls_trust = system`.
    klien: reqwest::Client,
    /// Satu klien (dan connection pool) per bahan `ca_kustom`/`pin_spki`. Versi
    /// dokumen provider tidak pernah berubah setelah ditulis, jadi bahan yang
    /// sama selalu memberi klien yang sama dan versi baru memberi klien baru
    /// (ADR-0002 D5), seperti cache per versi di jalur sync.
    klien_trust: Mutex<HashMap<TlsTrust, reqwest::Client>>,
}

impl PemanggilReqwest {
    pub fn baru() -> Result<Self, String> {
        let klien = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(BATAS_CONNECT)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            klien,
            klien_trust: Mutex::new(HashMap::new()),
        })
    }

    /// Klien untuk satu trust. `ca_kustom` dan `pin_spki` dirakit sekali lewat
    /// kode adapter yang sama dengan sync; kegagalan merakit (PEM yang bukan
    /// sertifikat) adalah `Tls`, tanpa koneksi keluar. Handler memanggilnya
    /// lebih dulu lewat `siapkan_trust` (sebelum CAS), jadi `panggil` selalu
    /// mendapat klien dari cache. Pesannya hanya memuat jenis galat, tidak
    /// pernah bahannya.
    fn klien_untuk(&self, trust: &TlsTrust) -> Result<reqwest::Client, GagalPanggilProvider> {
        if matches!(trust, TlsTrust::System) {
            return Ok(self.klien.clone());
        }
        let mut cache = self.klien_trust.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(k) = cache.get(trust) {
            return Ok(k.clone());
        }
        let k = build_provider_client_with_config(trust.clone(), ProviderClientConfig::default())
            .map_err(|e| GagalPanggilProvider::Tls(e.to_string()))?;
        cache.insert(trust.clone(), k.clone());
        Ok(k)
    }
}

impl PemanggilProvider for PemanggilReqwest {
    fn siapkan_trust(&self, trust: &TlsTrust) -> Result<(), String> {
        self.klien_untuk(trust).map(drop).map_err(|e| match e {
            GagalPanggilProvider::Tls(pesan) => pesan,
            _ => "klien tidak bisa dibangun".to_owned(),
        })
    }

    async fn panggil(
        &self,
        permintaan: &PermintaanKabel,
    ) -> Result<ResponsMentah, GagalPanggilProvider> {
        let klien = self.klien_untuk(&permintaan.trust)?;
        // Kirim dan pemetaan galat (termasuk galat sertifikat menjadi `Tls`,
        // ADR-0002 D5) dipakai bersama jalur sync (ARC-66, ARC-240).
        kirim(
            &klien,
            &permintaan.metode,
            &permintaan.url,
            permintaan.body.clone(),
            permintaan.batas_total,
        )
        .await
    }
}
