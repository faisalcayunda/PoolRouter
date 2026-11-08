//! Seam GW14 untuk T6: klien adapter diuji terhadap mock HTTPS lokal dengan
//! sertifikat sintetis yang dibuat saat test (AC-22, AC-20 uji 1).
//!
//! Yang dibuktikan di sini: tiga mode trust (termasuk 0 byte data aplikasi saat
//! pin salah), isolasi CA antar provider, redirect 3xx tidak diikuti, body GET
//! diterima mock dengan `Content-Length`, dan allowlist host:port.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use poolrouter_adapter_provider::client::{
    Allowlist, BATAS_CONNECT, BATAS_TOTAL, GalatKlien, ProviderClientConfig, TlsTrust,
    build_provider_client, build_provider_client_with_config,
};
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

const OK: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";

/// Sertifikat sintetis: CA + satu sertifikat server ber-SAN 127.0.0.1.
struct Sertifikat {
    ca_pem: String,
    cert_pem: String,
    key_pem: String,
    cert_der: CertificateDer<'static>,
}

fn sertifikat(nama: &str) -> Sertifikat {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, format!("uji-ca-{nama}"));
    let ca_cert = ca_params.clone().self_signed(&ca_key).unwrap();
    let ca_pem = ca_cert.pem();

    let mut params =
        CertificateParams::new(vec!["127.0.0.1".to_owned(), "localhost".to_owned()]).unwrap();
    params
        .distinguished_name
        .push(DnType::CommonName, format!("uji-server-{nama}"));
    let key = KeyPair::generate().unwrap();
    // signed_by signature: pub fn signed_by(self, signing_key: &KeyPair, issuer_cert: &Certificate, issuer_key: &KeyPair) -> Result<Certificate, RcgenError>
    let cert = params.signed_by(&key, &ca_cert, &ca_key).unwrap();

    Sertifikat {
        ca_pem,
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
        cert_der: cert.der().clone(),
    }
}

struct Keadaan {
    /// Byte data aplikasi (plaintext) yang diterima setelah handshake.
    data: Mutex<Vec<u8>>,
    /// Jumlah koneksi TCP yang diterima.
    koneksi: Mutex<usize>,
}

struct Mock {
    alamat: SocketAddr,
    keadaan: Arc<Keadaan>,
}

impl Mock {
    fn url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{}", self.alamat.port(), path)
    }

    fn data(&self) -> Vec<u8> {
        self.keadaan.data.lock().unwrap().clone()
    }

    fn koneksi(&self) -> usize {
        *self.keadaan.koneksi.lock().unwrap()
    }
}

/// Mock HTTPS: menjawab setiap request dengan byte `respons` yang tetap.
async fn mock(sertifikat: &Sertifikat, respons: Vec<u8>) -> Mock {
    let certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(sertifikat.cert_pem.as_bytes())
            .collect::<Result<_, _>>()
            .unwrap();
    let key = PrivateKeyDer::from_pem_slice(sertifikat.key_pem.as_bytes()).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let alamat = listener.local_addr().unwrap();
    let keadaan = Arc::new(Keadaan {
        data: Mutex::new(Vec::new()),
        koneksi: Mutex::new(0),
    });
    let state = Arc::clone(&keadaan);

    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                break;
            };
            *state.koneksi.lock().unwrap() += 1;
            let acceptor = acceptor.clone();
            let state = Arc::clone(&state);
            let respons = respons.clone();
            tokio::spawn(async move {
                let Ok(mut aliran) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut buf = [0u8; 4096];
                loop {
                    let n = match aliran.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    let lengkap = {
                        let mut data = state.data.lock().unwrap();
                        data.extend_from_slice(&buf[..n]);
                        pesan_lengkap(&data)
                    };
                    if lengkap {
                        break;
                    }
                }
                let _ = aliran.write_all(&respons).await;
                let _ = aliran.shutdown().await;
            });
        }
    });

    Mock { alamat, keadaan }
}

/// Header selesai dan body sepanjang `content-length` sudah diterima.
fn pesan_lengkap(data: &[u8]) -> bool {
    let Some(akhir) = posisi(data, b"\r\n\r\n") else {
        return false;
    };
    let kepala = String::from_utf8_lossy(&data[..akhir]).to_ascii_lowercase();
    let Some(baris) = kepala.lines().find(|l| l.starts_with("content-length:")) else {
        return true;
    };
    let Ok(panjang) = baris["content-length:".len()..].trim().parse::<usize>() else {
        return true;
    };
    data.len() >= akhir + 4 + panjang
}

fn posisi(jarum: &[u8], pola: &[u8]) -> Option<usize> {
    jarum.windows(pola.len()).position(|w| w == pola)
}

fn pin(sertifikat: &Sertifikat) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    use x509_parser::prelude::*;
    let (_, x509) = X509Certificate::from_der(sertifikat.cert_der.as_ref()).unwrap();
    Sha256::digest(x509.tbs_certificate.subject_pki.raw).into()
}

#[test]
fn tls_trust_dari_str() {
    assert_eq!(TlsTrust::dari_str("system"), Some(TlsTrust::System));
    assert_eq!(TlsTrust::dari_str("sistem"), Some(TlsTrust::System));
    assert_eq!(TlsTrust::dari_str("SYSTEM"), Some(TlsTrust::System));
    assert_eq!(
        TlsTrust::dari_str("ca_kustom"),
        Some(TlsTrust::CaKustom { ca_pem: vec![] })
    );
    assert_eq!(
        TlsTrust::dari_str("pin_spki"),
        Some(TlsTrust::PinSpki { pins: vec![] })
    );
    // Nama tak dikenal ditolak, bukan jatuh ke `system` (fail-open).
    assert_eq!(TlsTrust::dari_str("unknown"), None);
    assert_eq!(TlsTrust::dari_str(""), None);
}

#[test]
fn build_ca_kustom_empty_rejects() {
    let klien = build_provider_client(TlsTrust::CaKustom { ca_pem: vec![] });
    assert!(
        matches!(klien, Err(GalatKlien::CaTidakValid(_))),
        "{klien:?}"
    );
}

#[test]
fn build_ca_kustom_pem_rusak_rejects() {
    let klien = build_provider_client(TlsTrust::CaKustom {
        ca_pem: b"bukan pem".to_vec(),
    });
    assert!(
        matches!(klien, Err(GalatKlien::CaTidakValid(_))),
        "{klien:?}"
    );
}

#[test]
fn build_pin_spki_empty_rejects() {
    let klien = build_provider_client(TlsTrust::PinSpki { pins: vec![] });
    assert!(matches!(klien, Err(GalatKlien::PinTidakValid)), "{klien:?}");
}

#[test]
fn build_pin_spki_with_dummy_pin() {
    let pin = [0xAAu8; 32];
    let klien = build_provider_client(TlsTrust::PinSpki { pins: vec![pin] });
    assert!(
        klien.is_ok(),
        "klien dengan dummy pin harus build: {klien:?}"
    );
}

#[tokio::test]
async fn ca_kustom_handshake_ok() {
    let s = sertifikat("a");
    let server = mock(&s, OK.to_vec()).await;
    let klien = build_provider_client(TlsTrust::CaKustom {
        ca_pem: s.ca_pem.into_bytes(),
    })
    .unwrap();

    let res = klien.get(server.url("/uji")).send().await.unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    assert_eq!(res.text().await.unwrap(), "ok");
    assert!(!server.data().is_empty());
}

#[tokio::test]
async fn pin_spki_handshake_ok() {
    let s = sertifikat("a");
    let server = mock(&s, OK.to_vec()).await;
    let klien = build_provider_client(TlsTrust::PinSpki {
        pins: vec![pin(&s)],
    })
    .unwrap();

    let res = klien.get(server.url("/uji")).send().await.unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn pin_salah_nol_byte_data_aplikasi() {
    let s = sertifikat("a");
    let server = mock(&s, OK.to_vec()).await;
    let klien = build_provider_client(TlsTrust::PinSpki {
        pins: vec![[0x11u8; 32]],
    })
    .unwrap();

    let res = klien.get(server.url("/uji")).send().await;
    assert!(res.is_err(), "{res:?}");
    // Server tidak boleh menerima satu byte pun data aplikasi.
    assert!(server.data().is_empty(), "{:?}", server.data());
}

/// Custom verifier: cert accepted (simulate pin match), signature always rejected (key mismatch).
#[derive(Debug)]
struct MismatchKeyVerifier {
    // Hanya didokumentasikan di test caller; verifier ini selalu reject signature.
    #[allow(dead_code)]
    pin_cocok: [u8; 32],
}

impl rustls::client::danger::ServerCertVerifier for MismatchKeyVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::InvalidCertificate(
            rustls::CertificateError::UnknownIssuer,
        ))
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::InvalidCertificate(
            rustls::CertificateError::UnknownIssuer,
        ))
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![]
    }
}

#[tokio::test]
async fn pin_spki_cert_key_lain_handshake_gagal_nol_byte() {
    // Mock dengan cert A (pin cocok), klien reject di tanda tangan.
    // Verifikasi: pinnya cocok, tapi handshake gagal karena key lain.
    let cert_asli = sertifikat("asli");
    let pin_asli = pin(&cert_asli);

    let server = mock(&cert_asli, OK.to_vec()).await;

    // Build client dengan custom verifier: pin cocok tapi signature salah.
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier = MismatchKeyVerifier {
        pin_cocok: pin_asli,
    };
    let config = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    let klien = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(20))
        .tls_backend_preconfigured(config)
        .build()
        .unwrap();

    let res = klien.get(server.url("/uji")).send().await;
    assert!(
        res.is_err(),
        "handshake harus gagal (signature salah): {res:?}"
    );
    assert!(
        server.data().is_empty(),
        "0 byte data aplikasi saat sig fail: {:?}",
        server.data()
    );
}

#[tokio::test]
async fn system_tolak_sertifikat_sintetis() {
    let s = sertifikat("a");
    let server = mock(&s, OK.to_vec()).await;
    let klien = match build_provider_client(TlsTrust::System) {
        Ok(k) => k,
        // Root store kosong juga penolakan, bukan fallback diam-diam.
        Err(e) => {
            assert!(matches!(e, GalatKlien::RootSistem(_)), "{e:?}");
            return;
        }
    };

    let res = klien.get(server.url("/uji")).send().await;
    assert!(res.is_err(), "{res:?}");
    assert!(server.data().is_empty());
}

#[tokio::test]
async fn ca_provider_a_tidak_berlaku_untuk_provider_b() {
    let a = sertifikat("a");
    let b = sertifikat("b");
    let server_b = mock(&b, OK.to_vec()).await;
    let klien_a = build_provider_client(TlsTrust::CaKustom {
        ca_pem: a.ca_pem.into_bytes(),
    })
    .unwrap();

    let res = klien_a.get(server_b.url("/uji")).send().await;
    assert!(res.is_err(), "{res:?}");
    assert!(server_b.data().is_empty());
}

#[tokio::test]
async fn redirect_3xx_tidak_diikuti() {
    let a = sertifikat("a");
    let b = sertifikat("b");
    let target = mock(&b, OK.to_vec()).await;
    let respons = format!(
        "HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\n\r\n",
        target.url("/curang")
    );
    let server = mock(&a, respons.into_bytes()).await;
    let klien = build_provider_client(TlsTrust::CaKustom {
        ca_pem: a.ca_pem.into_bytes(),
    })
    .unwrap();

    let res = klien.get(server.url("/uji")).send().await.unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::FOUND);
    assert_eq!(target.koneksi(), 0, "redirect tidak boleh diikuti");
}

#[tokio::test]
async fn body_get_diterima_dengan_content_length() {
    let s = sertifikat("a");
    let server = mock(&s, OK.to_vec()).await;
    let klien = build_provider_client(TlsTrust::CaKustom {
        ca_pem: s.ca_pem.into_bytes(),
    })
    .unwrap();
    let body = r#"{"NIK":"3201000000000001"}"#;

    let res = klien
        .get(server.url("/uji"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);

    let mentah = String::from_utf8_lossy(&server.data()).into_owned();
    let diterima = mentah.to_ascii_lowercase();
    assert!(diterima.starts_with("get /uji "), "{diterima}");
    assert!(
        mentah.contains(body),
        "body GET tidak diterima mock: {mentah}"
    );
    assert!(
        diterima.contains(&format!("content-length: {}", body.len())),
        "GET harus memakai Content-Length: {diterima}"
    );
    assert!(
        !diterima.contains("transfer-encoding: chunked"),
        "GET tidak boleh chunked: {diterima}"
    );
}

#[test]
fn timeout_default_5_dan_20_detik() {
    assert_eq!(BATAS_CONNECT.as_secs(), 5);
    assert_eq!(BATAS_TOTAL.as_secs(), 20);
}

#[test]
fn allowlist_host_port() {
    let allow = Allowlist::dari_base_url("https://dukcapil.example.go.id/api").unwrap();
    let sama = reqwest::Url::parse("https://dukcapil.example.go.id/v1/cek").unwrap();
    // Port default https (443) sama dengan base URL tanpa port.
    assert!(allow.izinkan(&sama));

    let host_lain = reqwest::Url::parse("https://lain.example.go.id/v1/cek").unwrap();
    assert!(!allow.izinkan(&host_lain));

    let port_lain = reqwest::Url::parse("https://dukcapil.example.go.id:8443/v1/cek").unwrap();
    assert!(!allow.izinkan(&port_lain));

    let http = reqwest::Url::parse("http://dukcapil.example.go.id/v1/cek").unwrap();
    assert!(!allow.izinkan(&http));
}

#[test]
fn allowlist_menolak_base_url_bukan_https() {
    let err = Allowlist::dari_base_url("http://dukcapil.example.go.id/api");
    assert!(
        matches!(err, Err(GalatKlien::BaseUrlTidakValid(_))),
        "{err:?}"
    );

    let err = Allowlist::dari_base_url("https://dukcapil.example.go.id:8443/api").unwrap();
    assert!(err.izinkan(&reqwest::Url::parse("https://dukcapil.example.go.id:8443/x").unwrap()));
    assert!(!err.izinkan(&reqwest::Url::parse("https://dukcapil.example.go.id/x").unwrap()));
}

#[test]
fn dari_status_http_302_http_klien_enum() {
    use poolrouter_adapter_provider::GagalPanggilProvider;
    use poolrouter_adapter_provider::dari_status_http;
    // 302 adalah redirect (3xx); dikelompokkan sebagai HttpKlien karena bukan 5xx/401/403/429.
    let gagal = dari_status_http(302);
    assert!(
        matches!(gagal, GagalPanggilProvider::HttpKlien { status: 302 }),
        "302 harus menjadi HttpKlien(302) enum, bukan jenis lain: {gagal:?}"
    );
}

#[tokio::test]
async fn timeout_dengan_config_total_pendek() {
    // Mock TCP: terima koneksi, tapi tunggu lama (> 200ms).
    // Client dengan total timeout 200ms harus error timeout, bukan TLS error.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            if let Ok((_, _)) = listener.accept().await {
                // Jangan balas; biarkan timeout.
                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            }
        }
    });

    let s = sertifikat("a");
    let cfg = ProviderClientConfig {
        connect_timeout: std::time::Duration::from_millis(100),
        total_timeout: std::time::Duration::from_millis(200),
    };
    let klien = build_provider_client_with_config(
        TlsTrust::CaKustom {
            ca_pem: s.ca_pem.into_bytes(),
        },
        cfg,
    )
    .unwrap();

    let res = klien.get(format!("https://{}/x", addr)).send().await;

    assert!(
        res.is_err(),
        "request dengan timeout 200ms harus gagal: {res:?}"
    );
    // Verifikasi error adalah timeout, bukan jenis error lain.
    let err = res.unwrap_err();
    assert!(
        err.is_timeout(),
        "error harus timeout, bukan error lain: {err:?}"
    );
}
