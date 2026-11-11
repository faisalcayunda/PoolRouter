//! Galat sertifikat di `PemanggilReqwest` (ARC-240, ADR-0002 D5): server HTTPS
//! uji dengan sertifikat sintetis yang tidak ada di trust sistem harus menjadi
//! `Tls`, kelas `tls` yang tidak di-retry, bukan `Koneksi`.
//!
//! Tanpa PostgreSQL dan Redis; jalan di `make test`.

use std::sync::Arc;
use std::time::Duration;

use poolrouter_adapter_provider::{GagalPanggilProvider, kelas_error};
use poolrouter_core::retry::ErrorClass;
use poolrouter_worker::upstream::{PemanggilProvider, PemanggilReqwest, PermintaanKabel};
use rcgen::{CertificateParams, DnType, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// Server HTTPS dengan sertifikat self-signed ber-SAN 127.0.0.1. Mengembalikan
/// port; setiap koneksi dijawab 200 bila handshake lolos.
async fn server_tls_sintetis() -> u16 {
    let mut params = CertificateParams::new(vec!["127.0.0.1".to_owned()]).unwrap();
    params
        .distinguished_name
        .push(DnType::CommonName, "uji-worker-tls");
    let key = KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    let certs: Vec<CertificateDer<'static>> = vec![cert.der().clone()];
    let kunci = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(certs, kunci)
    .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut aliran) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let _ = aliran.read(&mut buf).await;
                let _ = aliran
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                    .await;
                let _ = aliran.shutdown().await;
            });
        }
    });
    port
}

fn permintaan(url: String) -> PermintaanKabel {
    PermintaanKabel {
        metode: "GET".to_owned(),
        url,
        body: Some("{\"NIK\":\"0012345678901234\"}".to_owned()),
        tls_trust: "system".to_owned(),
        batas_total: Duration::from_secs(5),
    }
}

/// Sertifikat tidak sah (bukan dari CA yang dipercaya sistem) menjadi `Tls`
/// dan kelasnya tidak retryable.
#[tokio::test]
async fn sertifikat_tidak_sah_menjadi_tls_tanpa_retry() {
    let port = server_tls_sintetis().await;
    let pemanggil = PemanggilReqwest::baru().unwrap();
    let hasil = pemanggil
        .panggil(&permintaan(format!("https://127.0.0.1:{port}/uji")))
        .await;
    let Err(gagal) = hasil else {
        panic!("sertifikat self-signed harus ditolak");
    };
    assert!(matches!(gagal, GagalPanggilProvider::Tls(_)), "{gagal:?}");
    let kelas = kelas_error(&gagal);
    assert_eq!(kelas, ErrorClass::Tls);
    assert!(!kelas.retryable());
}

/// Pesan galat tidak membawa URL (memuat NIK atau secret hasil render).
#[tokio::test]
async fn galat_tls_tidak_membawa_url() {
    let port = server_tls_sintetis().await;
    let pemanggil = PemanggilReqwest::baru().unwrap();
    let url = format!("https://127.0.0.1:{port}/uji?nik=0012345678901234");
    let Err(GagalPanggilProvider::Tls(pesan)) = pemanggil.panggil(&permintaan(url)).await else {
        panic!("harus Tls");
    };
    assert!(!pesan.contains("0012345678901234"), "{pesan}");
}

/// Mode trust selain `system` tetap ditolak tanpa koneksi keluar (perilaku
/// lama worker, tidak berubah).
#[tokio::test]
async fn tls_trust_selain_system_ditolak_sebagai_tls() {
    let pemanggil = PemanggilReqwest::baru().unwrap();
    let mut p = permintaan("https://127.0.0.1:1/uji".to_owned());
    p.tls_trust = "pin_spki".to_owned();
    let hasil = pemanggil.panggil(&p).await;
    assert!(
        matches!(hasil, Err(GagalPanggilProvider::Tls(_))),
        "{hasil:?}"
    );
}
