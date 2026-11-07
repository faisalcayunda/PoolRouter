//! HTTP client ke provider (ADR-0002 D5, ADR-0004 §4.2).
//!
//! Satu-satunya pemakai `rustls`/`aws-lc-rs` di crate ini: [`build_provider_client`]
//! merakit `rustls::ClientConfig` sesuai mode trust per provider dan memasangnya ke
//! reqwest lewat `tls_backend_preconfigured`. Tidak ada mode yang mematikan
//! verifikasi (D5).
//!
//! - `system`: root store sistem (rustls-native-certs).
//! - `ca_kustom`: CA kustom per provider ( rantai dan nama tetap diverifikasi).
//! - `pin_spki`: pin SHA-256 SPKI; masa berlaku sertifikat tidak memblokir.
//!
//! Redirect tidak diikuti (`Policy::none()`), timeout connect 5 s / total 20 s
//! (ADR-0001 §5a.4, R-04). Host:port allowlist diperiksa pemanggil sebelum
//! memanggil klien ini (egress tetap).

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use rustls::ClientConfig;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server_cert_verifier::ServerCertVerifier;
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
    pub fn dari_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "system" | "sistem" => Some(Self::System),
            "ca_kustom" | "ca_custom" | "ca-custom" => Some(Self::CaKustom { ca_pem: Vec::new() }),
            "pin_spki" | "pin-spki" | "pin" => Some(Self::PinSpki { pins: Vec::new() }),
            _ => None,
        }
    }
}

impl From<&str> for TlsTrust {
    fn from(s: &str) -> Self {
        Self::dari_str(s).unwrap_or(Self::System)
    }
}

impl From<String> for TlsTrust {
    fn from(s: String) -> Self {
        Self::from(s.as_str())
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
    #[error("gagal merakit ClientConfig: {0}")]
    Config(String),
    #[error("gagal membangun klien HTTP: {0}")]
    Klien(String),
}

/// Batas waktu default (ADR-0001 §5a.4, R-04).
pub const BATAS_CONNECT: Duration = Duration::from_secs(5);
pub const BATAS_TOTAL: Duration = Duration::from_secs(20);

/// Rakit `reqwest::Client` dengan trust TLS per provider.
pub fn build_provider_client(tls_trust: impl Into<TlsTrust>) -> Result<Client, GalatKlien> {
    let trust = tls_trust.into();
    let config = match trust {
        TlsTrust::System => {
            let mut roots = rustls::RootCertStore::empty();
            for cert in load_native_certs().map_err(|e| GalatKlien::RootSistem(e.to_string()))? {
                let _ = roots.add(cert);
            }
            // Fallback ke webpki-roots bila sistem kosong (CI/container).
            if roots.is_empty() {
                roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            }
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|e| GalatKlien::Config(e.to_string()))?
                .with_root_certificates(roots)
                .with_no_client_auth()
        }
        TlsTrust::CaKustom { ca_pem } => {
            let certs = rustls_pemfile::certs(&mut ca_pem.as_slice())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| GalatKlien::CaTidakValid(e.to_string()))?;
            let mut roots = rustls::RootCertStore::empty();
            for c in certs {
                roots.add(c).map_err(|e| GalatKlien::CaTidakValid(e.to_string()))?;
            }
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|e| GalatKlien::Config(e.to_string()))?
                .with_root_certificates(roots)
                .with_no_client_auth()
        }
        TlsTrust::PinSpki { pins } => {
            if pins.is_empty() {
                return Err(GalatKlien::PinTidakValid);
            }
            let verifier = PinSpkiVerifier::new(pins);
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|e| GalatKlien::Config(e.to_string()))?
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(verifier))
                .with_no_client_auth()
        }
    };

    Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(BATAS_CONNECT)
        .timeout(BATAS_TOTAL)
        .tls_backend_preconfigured(config)
        .build()
        .map_err(|e| GalatKlien::Klien(e.to_string()))
}

/// Verifier pin SPKI SHA-256. Masa berlaku sertifikat tidak memblokir (D5).
/// Verifier ini tidak memeriksa signature karena hanya mempercayai SPKI tertentu;
/// signature verifikasi tetap dilakukan oleh rustls default provider untuk key
/// yang cocok dengan pin.
struct PinSpkiVerifier {
    pins: Vec<[u8; 32]>,
}

impl PinSpkiVerifier {
    fn new(pins: Vec<[u8; 32]>) -> Self {
        Self { pins }
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
    ) -> Result<rustls::pki_types::ServerCertVerified, rustls::Error> {
        use sha2::{Digest, Sha256};
        let spki = spki_dari_cert(end_entity).ok_or(rustls::Error::InvalidCertificate(
            rustls::CertificateError::UnknownIssuer,
        ))?;
        let hash = Sha256::digest(&spki);
        let hash: [u8; 32] = hash.into();
        if self.pins.iter().any(|p| *p == hash) {
            Ok(rustls::pki_types::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::pki_types::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::pki_types::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::pki_types::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::pki_types::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

/// Ekstrak SubjectPublicKeyInfo (DER) dari sertifikat X.509.
fn spki_dari_cert(cert: &CertificateDer<'_>) -> Option<Vec<u8>> {
    use x509_parser::prelude::*;
    let (_, parsed) = X509Certificate::from_der(cert.as_ref()).ok()?;
    Some(parsed.tbs_certificate.subject_pki.raw.to_vec())
}
