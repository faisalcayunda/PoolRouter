//! ARC-239: jalur sync memakai bahan TLS per provider (`tls_ca_pem`,
//! `tls_pin_spki`) dari dokumen versi konfigurasi, end-to-end terhadap mock
//! HTTPS lokal dengan sertifikat sintetis yang dibuat saat tes.
//!
//! Setiap mode punya satu kasus yang harus lolos dan satu yang harus gagal
//! tanpa satu byte pun data aplikasi sampai ke mock, supaya terbukti trust
//! benar-benar dipakai, bukan sekadar diterima.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poolrouter_webserver::downstream::downstream_sesi::{Fut, GalatDownstream, Hasil};
use poolrouter_webserver::downstream::pemanggil::{
    PanggilanProvider, PemanggilHttp, PemanggilProvider, SumberPanggilan, TransportTls,
};
use poolrouter_webserver::downstream::permintaan::HasilPanggilan;
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use uuid::Uuid;

const JAWABAN: &str = r#"{"content":[{"NAMA_LGKP":"Sesuai (90)"}]}"#;

/// CA sintetis plus satu sertifikat server ber-SAN 127.0.0.1 yang ditandatangani CA itu.
struct Sertifikat {
    ca_pem: String,
    cert_der: CertificateDer<'static>,
    key_der: Vec<u8>,
    /// SHA-256 dari SPKI sertifikat server, base64 standar (bentuk `tls_pin_spki`).
    pin: String,
}

fn sertifikat(nama: &str) -> Sertifikat {
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

struct MockHttps {
    alamat: SocketAddr,
    /// Byte data aplikasi yang diterima setelah handshake berhasil.
    data: Arc<Mutex<Vec<u8>>>,
}

async fn mock_https(s: &Sertifikat) -> MockHttps {
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
    let data = Arc::new(Mutex::new(Vec::new()));
    let catat = Arc::clone(&data);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            let catat = Arc::clone(&catat);
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
                    // Body provider `{}` selalu kecil: kepala lengkap = pesan lengkap.
                    if d.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let respons = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{JAWABAN}",
                    JAWABAN.len()
                );
                let _ = aliran.write_all(respons.as_bytes()).await;
                let _ = aliran.shutdown().await;
            });
        }
    });
    MockHttps { alamat, data }
}

struct SumberUji(Value);

impl SumberPanggilan for SumberUji {
    fn dokumen_provider(&self, _id: Uuid, _versi: i32) -> Fut<'_, Hasil<Value>> {
        let d = self.0.clone();
        Box::pin(async move { Ok(d) })
    }
    fn dokumen_endpoint(&self, _id: Uuid, _versi: i32) -> Fut<'_, Hasil<Option<Value>>> {
        Box::pin(async { Ok(None) })
    }
    fn secret(&self, nama: Vec<String>) -> Fut<'_, Hasil<BTreeMap<String, String>>> {
        assert!(nama.is_empty(), "dokumen uji tidak memakai secret");
        Box::pin(async { Ok(BTreeMap::new()) })
    }
}

/// Dokumen versi provider seperti yang ditulis admin API, plus bahan TLS.
fn dokumen(mock: &MockHttps, tls: Value) -> Value {
    let mut dok = json!({
        "metode": "POST",
        "base_url": format!("https://127.0.0.1:{}/api", mock.alamat.port()),
        "body_template": "{}",
        "respons_skema": {
            "parser": "status_skor",
            "literal": [
                {"teks": "Sesuai", "status": "sesuai"},
                {"teks": "Tidak Sesuai", "status": "tidak_sesuai"}
            ],
            "parameter_output": [{"nama": "NAMA_LGKP"}]
        },
        "parameters": [],
        "log_mode": "metadata_only"
    });
    for (k, v) in tls.as_object().unwrap() {
        dok[k] = v.clone();
    }
    dok
}

fn panggilan() -> PanggilanProvider {
    PanggilanProvider {
        request_id: Uuid::now_v7(),
        kode_transaksi: Uuid::now_v7(),
        provider_id: Uuid::now_v7(),
        endpoint_id: Uuid::now_v7(),
        provider_config_version: 1,
        endpoint_config_version: 1,
        full_payload: false,
        input: json!({"NAMA_LGKP": "Budi Sintetis"}),
    }
}

/// Jalur produksi sync: `PemanggilHttp` + `TransportTls`. `None` = `siapkan`
/// menolak (503 tanpa koneksi keluar).
async fn panggil(dok: Value) -> Option<HasilPanggilan> {
    let pemanggil = PemanggilHttp::baru(SumberUji(dok), TransportTls);
    let siap = match pemanggil.siapkan(panggilan()).await {
        Ok(siap) => siap,
        Err(GalatDownstream::Layanan) => return None,
        Err(_) => panic!("siapkan harus gagal sebagai Layanan"),
    };
    Some((siap.kirim)(Duration::from_secs(10)).await.hasil)
}

fn berhasil(h: Option<HasilPanggilan>) -> bool {
    matches!(
        h,
        Some(HasilPanggilan::Berhasil {
            http_status: 200,
            ..
        })
    )
}

#[tokio::test]
async fn ca_kustom_dengan_ca_penerbit_berhasil() {
    let s = sertifikat("a");
    let mock = mock_https(&s).await;
    let dok = dokumen(
        &mock,
        json!({"tls_trust": "ca_kustom", "tls_ca_pem": s.ca_pem}),
    );
    let h = panggil(dok).await;
    assert!(berhasil(h), "ca_kustom dengan CA yang benar harus lolos");
    assert!(!mock.data.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ca_kustom_dengan_ca_lain_gagal_tanpa_data_aplikasi() {
    let s = sertifikat("a");
    let lain = sertifikat("b");
    let mock = mock_https(&s).await;
    let dok = dokumen(
        &mock,
        json!({"tls_trust": "ca_kustom", "tls_ca_pem": lain.ca_pem}),
    );
    let h = panggil(dok).await;
    assert!(
        matches!(h, Some(HasilPanggilan::Gagal { .. })),
        "CA yang bukan penerbit harus ditolak"
    );
    assert!(mock.data.lock().unwrap().is_empty(), "0 byte data aplikasi");
}

#[tokio::test]
async fn pin_spki_dengan_pin_yang_cocok_berhasil() {
    let s = sertifikat("a");
    let lain = sertifikat("b");
    let mock = mock_https(&s).await;
    // Dua pin (pin lama dan baru, urutan rotasi): salah satu cocok sudah cukup.
    let dok = dokumen(
        &mock,
        json!({"tls_trust": "pin_spki", "tls_pin_spki": [lain.pin, s.pin]}),
    );
    let h = panggil(dok).await;
    assert!(berhasil(h), "pin_spki dengan pin yang cocok harus lolos");
}

#[tokio::test]
async fn pin_spki_dengan_pin_lain_gagal_tanpa_data_aplikasi() {
    let s = sertifikat("a");
    let lain = sertifikat("b");
    let mock = mock_https(&s).await;
    let dok = dokumen(
        &mock,
        json!({"tls_trust": "pin_spki", "tls_pin_spki": [lain.pin]}),
    );
    let h = panggil(dok).await;
    assert!(
        matches!(h, Some(HasilPanggilan::Gagal { .. })),
        "pin yang tidak cocok harus ditolak"
    );
    assert!(mock.data.lock().unwrap().is_empty(), "0 byte data aplikasi");
}

#[tokio::test]
async fn mode_tanpa_bahan_atau_bahan_tidak_cocok_ditolak_sebelum_koneksi() {
    let s = sertifikat("a");
    let mock = mock_https(&s).await;
    let kasus = [
        json!({"tls_trust": "ca_kustom"}),
        json!({"tls_trust": "pin_spki"}),
        json!({"tls_trust": "ca_kustom", "tls_ca_pem": s.ca_pem, "tls_pin_spki": [s.pin]}),
        json!({"tls_trust": "pin_spki", "tls_pin_spki": ["bukan-base64"]}),
        json!({"tls_trust": "system", "tls_ca_pem": s.ca_pem}),
    ];
    for tls in kasus {
        let dok = dokumen(&mock, tls.clone());
        assert!(panggil(dok).await.is_none(), "{tls}");
    }
    assert!(mock.data.lock().unwrap().is_empty());
}
