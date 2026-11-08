//! HTTP client ke provider (ADR-0002 D5, ADR-0004 §4.2).
//!
//! Satu-satunya pemakai `rustls`/`aws-lc-rs` di crate ini: [`build_provider_client`]
//! merakit `rustls::ClientConfig` sesuai mode trust per provider dan memasangnya ke
//! reqwest lewat `tls_backend_preconfigured`. Tidak ada mode yang mematikan
//! verifikasi (D5).
//!
//! - `system`: root store sistem (rustls-native-certs). Root kosong = gagal, bukan
//!   fallback diam-diam.
//! - `ca_kustom`: CA kustom per provider (rantai dan nama tetap diverifikasi).
//! - `pin_spki`: pin SHA-256 SPKI; masa berlaku sertifikat tidak memblokir.
//!
//! Redirect tidak diikuti (`Policy::none()`), timeout connect 5 s / total 20 s
//! (ADR-0001 §5a.4, R-04). [`Allowlist`] membatasi tujuan egress ke host:port
//! provider (D4); pemanggil memeriksanya sebelum mengirim request.

use std::sync::Arc;
use std::time::Duration;

use reqwest::{Client, Url};
use rustls::ClientConfig;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls_native_certs::load_native_certs;

/// Mode trust TLS per provider (ADR-0002 D5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsTrust {
    /// Root store sistem.
    System,
    /// CA kustom per provider (PEM, satu atau lebih sertifikat CA).
    CaKustom { ca_pem: Vec<u8> },
    /// Pin SHA-256 SPKI (satu atau lebih). Masa berlaku sertifikat tidak memblokir.
    PinSpki { pins: Vec<[u8; 32]> },
}

impl TlsTrust {
    /// Nama mode dari konfigurasi provider. `None` for an unknown name: the
    /// caller must reject it rather than fall back to `system`.
    pub fn dari_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "system" | "sistem" => Some(Self::System),
            "ca_kustom" | "ca_custom" | "ca-custom" => Some(Self::CaKustom { ca_pem: Vec::new() }),
            "pin_spki" | "pin-spki" | "pin" => Some(Self::PinSpki { pins: Vec::new() }),
            _ => None,
        }
    }
}

/// Galat saat merakit klien.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GalatKlien {
    #[error("CA kustom tidak valid: {0}")]
    CaTidakValid(String),
    #[error("pin SPKI harus 32 byte SHA-256")]
    PinTidakValid,
    #[error("gagal memuat root store sistem: {0}")]
    RootSistem(String),
    #[error("base URL provider tidak valid: {0}")]
    BaseUrlTidakValid(String),
    #[error("gagal merakit ClientConfig: {0}")]
    Config(String),
    #[error("gagal membangun klien HTTP: {0}")]
    Klien(String),
}

/// Batas waktu default (ADR-0001 §5a.4, R-04).
pub const BATAS_CONNECT: Duration = Duration::from_secs(5);
pub const BATAS_TOTAL: Duration = Duration::from_secs(20);

/// Konfigurasi optional untuk klien adapter provider.
pub struct ProviderClientConfig {
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
}

impl Default for ProviderClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: BATAS_CONNECT,
            total_timeout: BATAS_TOTAL,
        }
    }
}

/// Allowlist host:port untuk egress provider (ADR-0002 D4).
///
/// Hanya `https` yang diterima, karena admin API menolak skema lain dan
/// `Client` dibangun dengan `https_only`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allowlist {
    host: String,
    port: u16,
}

impl Allowlist {
    pub fn dari_base_url(base_url: &str) -> Result<Self, GalatKlien> {
        let url = Url::parse(base_url).map_err(|e| GalatKlien::BaseUrlTidakValid(e.to_string()))?;
        if url.scheme() != "https" {
            return Err(GalatKlien::BaseUrlTidakValid(
                "skema harus https".to_owned(),
            ));
        }
        let host = url
            .host_str()
            .ok_or_else(|| GalatKlien::BaseUrlTidakValid("host kosong".to_owned()))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| GalatKlien::BaseUrlTidakValid("port tidak diketahui".to_owned()))?;
        Ok(Self {
            host: host.to_ascii_lowercase(),
            port,
        })
    }

    pub fn izinkan(&self, url: &Url) -> bool {
        url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|h| h.eq_ignore_ascii_case(&self.host))
            && url.port_or_known_default() == Some(self.port)
    }
}

/// Rakit `reqwest::Client` dengan trust TLS per provider (timeout default).
pub fn build_provider_client(trust: TlsTrust) -> Result<Client, GalatKlien> {
    build_provider_client_with_config(trust, ProviderClientConfig::default())
}

/// Rakit `reqwest::Client` dengan trust TLS per provider dan konfigurasi custom.
pub fn build_provider_client_with_config(
    trust: TlsTrust,
    cfg: ProviderClientConfig,
) -> Result<Client, GalatKlien> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = match trust {
        TlsTrust::System => {
            let hasil = load_native_certs();
            let mut roots = rustls::RootCertStore::empty();
            for cert in hasil.certs {
                let _ = roots.add(cert);
            }
            if roots.is_empty() {
                return Err(GalatKlien::RootSistem(format!(
                    "root store sistem kosong ({} galat baca)",
                    hasil.errors.len()
                )));
            }
            builder(Arc::clone(&provider))?
                .with_root_certificates(roots)
                .with_no_client_auth()
        }
        TlsTrust::CaKustom { ca_pem } => {
            let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&ca_pem)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| GalatKlien::CaTidakValid(e.to_string()))?;
            if certs.is_empty() {
                return Err(GalatKlien::CaTidakValid(
                    "tidak ada sertifikat CA di PEM".to_owned(),
                ));
            }
            let mut roots = rustls::RootCertStore::empty();
            for c in certs {
                roots
                    .add(c)
                    .map_err(|e| GalatKlien::CaTidakValid(e.to_string()))?;
            }
            builder(Arc::clone(&provider))?
                .with_root_certificates(roots)
                .with_no_client_auth()
        }
        TlsTrust::PinSpki { pins } => {
            if pins.is_empty() {
                return Err(GalatKlien::PinTidakValid);
            }
            let verifier = PinSpkiVerifier::new(pins, Arc::clone(&provider));
            builder(Arc::clone(&provider))?
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(verifier))
                .with_no_client_auth()
        }
    };

    Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(cfg.connect_timeout)
        .timeout(cfg.total_timeout)
        .tls_backend_preconfigured(config)
        .build()
        .map_err(|e| GalatKlien::Klien(e.to_string()))
}

fn builder(
    provider: Arc<dyn rustls::crypto::CryptoProvider>,
) -> Result<rustls::ConfigBuilder<rustls::ClientConfig, rustls::WantsVerifier>, GalatKlien> {
    rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| GalatKlien::Config(e.to_string()))
}

/// Verifier pin SPKI SHA-256. Masa berlaku sertifikat tidak memblokir (D5).
/// Handshake ditandatangani dengan verifikasi nyata, bukan diam-diam.
#[derive(Debug)]
struct PinSpkiVerifier {
    pins: Vec<[u8; 32]>,
    provider: Arc<dyn rustls::crypto::CryptoProvider>,
}

impl PinSpkiVerifier {
    fn new(pins: Vec<[u8; 32]>, provider: Arc<dyn rustls::crypto::CryptoProvider>) -> Self {
        Self { pins, provider }
    }
}

impl ServerCertVerifier for PinSpkiVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        use sha2::{Digest, Sha256};
        let spki = spki_dari_cert(end_entity).ok_or(rustls::Error::InvalidCertificate(
            rustls::CertificateError::UnknownIssuer,
        ))?;
        let hash = Sha256::digest(&spki);
        let hash: [u8; 32] = hash.into();
        if self.pins.contains(&hash) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.provider
            .signature_verification_algorithms()
            .verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.provider
            .signature_verification_algorithms()
            .verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms()
            .supported_schemes()
    }
}

/// Ekstrak SubjectPublicKeyInfo (DER) dari sertifikat X.509.
fn spki_dari_cert(cert: &CertificateDer<'_>) -> Option<Vec<u8>> {
    use x509_parser::prelude::*;
    let (_, parsed) = X509Certificate::from_der(cert.as_ref()).ok()?;
    Some(parsed.tbs_certificate.subject_pki.raw.to_vec())
}
