//! Dukungan uji TLS bersama (ARC-239): CA dan sertifikat server sintetis dibuat
//! saat tes (tanpa secret nyata) dan mock HTTPS lokal yang mencatat byte data
//! aplikasi yang sampai setelah handshake.
#![allow(dead_code)] // dipakai sebagian oleh tiap biner uji

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// CA sintetis plus satu sertifikat server ber-SAN 127.0.0.1 yang ditandatangani CA itu.
pub struct Sertifikat {
    pub ca_pem: String,
    pub cert_der: CertificateDer<'static>,
    pub key_der: Vec<u8>,
    /// SHA-256 dari SPKI sertifikat server, base64 standar (bentuk `tls_pin_spki`).
    pub pin: String,
}

pub fn sertifikat(nama: &str) -> Sertifikat {
    use base64::Engine as _;
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, format!("uji-ca-{nama}"));
    let ca = ca_params.self_signed(&ca_key).unwrap();

    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec!["127.0.0.1".to_owned()]).unwrap();
    params
        .distinguished_name
        .push(DnType::CommonName, format!("uji-server-{nama}"));
    let cert = params.signed_by(&key, &ca, &ca_key).unwrap();

    let hash = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, &key.public_key_der());
    Sertifikat {
        ca_pem: ca.pem(),
        cert_der: cert.der().clone(),
        key_der: key.serialize_der(),
        pin: base64::engine::general_purpose::STANDARD.encode(hash.as_ref()),
    }
}

pub struct MockHttps {
    pub alamat: SocketAddr,
    /// Byte data aplikasi yang diterima setelah handshake berhasil.
    pub data: Arc<Mutex<Vec<u8>>>,
}

/// `jawaban` = body JSON yang dikembalikan mock (200) setelah handshake lolos.
pub async fn mock_https(s: &Sertifikat, jawaban: &str) -> MockHttps {
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![s.cert_der.clone()],
        PrivateKeyDer::from(PrivatePkcs8KeyDer::from(s.key_der.clone())),
    )
    .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let alamat = listener.local_addr().unwrap();
    let jawaban = jawaban.to_owned();
    let data = Arc::new(Mutex::new(Vec::new()));
    let catat = Arc::clone(&data);
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let catat = Arc::clone(&catat);
            let jawaban = jawaban.clone();
            tokio::spawn(async move {
                let Ok(mut aliran) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut buf = [0u8; 4096];
                loop {
                    let n = match aliran.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => n,
                    };
                    let mut d = catat.lock().unwrap();
                    d.extend_from_slice(&buf[..n]);
                    // Body uji kecil: kepala lengkap = pesan lengkap.
                    if d.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let respons = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{jawaban}",
                    jawaban.len()
                );
                let _ = aliran.write_all(respons.as_bytes()).await;
                let _ = aliran.shutdown().await;
            });
        }
    });
    MockHttps { alamat, data }
}
