//! Port panggilan HTTP ke provider (ADR-0002 D1, D5; ADR-0001 bagian 5a.4).
//!
//! Handler hanya bergantung pada trait [`PemanggilProvider`]. Implementasi
//! produksi [`PemanggilReqwest`]:
//!
//! - tidak mengikuti redirect (redirect mengirim ulang body berkredensial ke
//!   host lain) dan tidak me-retry sendiri; retry dikendalikan PoolRouter;
//! - body dikirim dengan `Content-Length` (body `String` utuh, bukan stream);
//! - timeout connect 5 s, total dari konfigurasi pool (bawaan 20 s);
//! - body respons dibaca paling banyak `BATAS_BODY_RESPONS` (ARC-241);
//! - hanya `tls_trust = system`. Mode `ca_kustom` dan `pin_spki` dibangun di
//!   klien adapter provider ARC-64/ARC-221 yang belum masuk `main`; sampai
//!   itu, worker menolaknya tanpa koneksi keluar sebagai kegagalan `tls`
//!   (tidak di-retry), bukan diam-diam memakai trust sistem.

use std::fmt;
use std::future::Future;
use std::time::Duration;

use poolrouter_adapter_provider::GagalPanggilProvider;
use poolrouter_adapter_provider::client::{BATAS_BODY_RESPONS, baca_body};

/// Batas connect ke provider (ADR-0001 bagian 5a.4).
pub const BATAS_CONNECT: Duration = Duration::from_secs(5);

/// Permintaan siap kirim. URL dan body memuat secret dan NIK, jadi `Debug`
/// sengaja hanya menampilkan metode dan mode trust.
pub struct PermintaanKabel {
    pub metode: String,
    pub url: String,
    pub body: Option<String>,
    pub tls_trust: String,
    pub batas_total: Duration,
}

impl fmt::Debug for PermintaanKabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PermintaanKabel")
            .field("metode", &self.metode)
            .field("tls_trust", &self.tls_trust)
            .finish()
    }
}

/// Respons HTTP apa pun kodenya; dipindah ke adapter (ARC-66). Klasifikasi
/// ada di [`crate::keputusan`].
pub use poolrouter_adapter_provider::hasil::ResponsMentah;

/// Satu panggilan ke provider. `Err` hanya untuk kegagalan transport
/// (timeout, koneksi, TLS); respons HTTP non-2xx tetap `Ok`.
pub trait PemanggilProvider: Send + Sync {
    fn panggil(
        &self,
        permintaan: &PermintaanKabel,
    ) -> impl Future<Output = Result<ResponsMentah, GagalPanggilProvider>> + Send;
}

/// Implementasi produksi di atas `reqwest`.
pub struct PemanggilReqwest {
    klien: reqwest::Client,
}

impl PemanggilReqwest {
    pub fn baru() -> Result<Self, String> {
        let klien = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(BATAS_CONNECT)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { klien })
    }
}

/// Galat `reqwest` tanpa URL: URL hasil render bisa memuat NIK atau secret.
fn petakan(e: reqwest::Error) -> GagalPanggilProvider {
    if e.is_timeout() {
        GagalPanggilProvider::Timeout
    } else {
        GagalPanggilProvider::Koneksi(e.without_url().to_string())
    }
}

impl PemanggilProvider for PemanggilReqwest {
    async fn panggil(
        &self,
        permintaan: &PermintaanKabel,
    ) -> Result<ResponsMentah, GagalPanggilProvider> {
        if permintaan.tls_trust != "system" {
            return Err(GagalPanggilProvider::Tls(format!(
                "tls_trust {} belum didukung worker",
                permintaan.tls_trust
            )));
        }
        // Metode dari dokumen yang sudah divalidasi admin API (GET/POST). Metode
        // lain adalah konfigurasi rusak: `HttpKlien` dipetakan ke `lainnya`,
        // yang tidak di-retry.
        let metode = reqwest::Method::from_bytes(permintaan.metode.as_bytes())
            .map_err(|_| GagalPanggilProvider::HttpKlien { status: 0 })?;
        let mut rb = self
            .klien
            .request(metode, permintaan.url.as_str())
            .timeout(permintaan.batas_total);
        if let Some(body) = &permintaan.body {
            rb = rb
                .header("content-type", "application/json")
                .body(body.clone());
        }
        let jawab = rb.send().await.map_err(petakan)?;
        let status = jawab.status().as_u16();
        let body = baca_body(jawab, BATAS_BODY_RESPONS).await?;
        Ok(ResponsMentah { status, body })
    }
}
