//! Batas ukuran body respons pada jalur worker (ARC-241). Jalur sync memakai
//! `poolrouter_adapter_provider::client::kirim` dan diuji di crate adapter;
//! worker punya klien sendiri, jadi batas yang sama dibuktikan di sini.
//! Tanpa PostgreSQL atau Redis: provider tiruan wiremock di http biasa.

use std::time::Duration;

use poolrouter_adapter_provider::GagalPanggilProvider;
use poolrouter_adapter_provider::client::BATAS_BODY_RESPONS;
use poolrouter_worker::upstream::{PemanggilProvider, PemanggilReqwest, PermintaanKabel};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Panjang body yang diterima worker (bukan isinya, supaya pesan gagal tetap pendek).
async fn panggil(n: usize) -> Result<usize, GagalPanggilProvider> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; n]))
        .mount(&server)
        .await;
    let p = PemanggilReqwest::baru().expect("klien reqwest");
    let permintaan = PermintaanKabel {
        metode: "POST".to_owned(),
        url: format!("{}/api", server.uri()),
        body: Some("{}".to_owned()),
        tls_trust: "system".to_owned(),
        batas_total: Duration::from_secs(10),
    };
    p.panggil(&permintaan).await.map(|r| r.body.len())
}

#[tokio::test]
async fn body_tepat_batas_diterima() {
    let panjang = panggil(BATAS_BODY_RESPONS).await.expect("sebesar batas");
    assert_eq!(panjang, BATAS_BODY_RESPONS);
}

#[tokio::test]
async fn body_melebihi_batas_ditolak() {
    let hasil = panggil(BATAS_BODY_RESPONS + 1).await;
    assert!(
        matches!(hasil, Err(GagalPanggilProvider::ResponsTidakValid(_))),
        "{hasil:?}"
    );
}
