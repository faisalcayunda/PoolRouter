//! ARC-239: worker memakai bahan TLS per provider (`tls_ca_pem`, `tls_pin_spki`)
//! dari dokumen versi konfigurasi, end-to-end terhadap mock HTTPS lokal dengan
//! sertifikat sintetis yang dibuat saat tes. Cermin
//! `webserver/tests/sync_tls_bahan.rs`.
//!
//! Setiap mode punya satu kasus yang harus lolos dan satu yang harus gagal
//! menjadi `tls` (tidak di-retry) tanpa satu byte pun data aplikasi sampai ke
//! mock, supaya terbukti trust benar-benar dipakai, bukan sekadar diterima.
//!
//! Tanpa PostgreSQL dan Redis; jalan di `make test`.

use std::time::Duration;

use poolrouter_adapter_provider::{GagalPanggilProvider, kelas_error};
use poolrouter_core::retry::ErrorClass;
use poolrouter_worker::render::baca_dokumen_provider;
use poolrouter_worker::upstream::{
    PemanggilProvider, PemanggilReqwest, PermintaanKabel, ResponsMentah,
};
use serde_json::{Value, json};
use tls_uji::{MockHttps, mock_https as mock_https_jawaban, sertifikat};

mod tls_uji;

const JAWABAN: &str = r#"{"content":[{"NAMA_LGKP":"Sesuai (90)"}]}"#;

/// Dokumen versi provider seperti yang ditulis admin API, plus bahan TLS.
fn dokumen(mock: &MockHttps, tls: &Value) -> Value {
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

async fn mock_https(s: &tls_uji::Sertifikat) -> MockHttps {
    mock_https_jawaban(s, JAWABAN).await
}

/// Langkah yang sama dengan handler: dokumen → `trust()` → `siapkan_trust` →
/// permintaan kabel → `PemanggilReqwest`. `None` = bahan ditolak sebelum
/// koneksi keluar (handler menolak sebelum CAS).
async fn panggil(
    pemanggil: &PemanggilReqwest,
    dok: &Value,
) -> Option<Result<ResponsMentah, GagalPanggilProvider>> {
    let dok = baca_dokumen_provider(dok).ok()?;
    let trust = dok.trust().ok()?;
    pemanggil.siapkan_trust(&trust).ok()?;
    let kabel = PermintaanKabel {
        metode: dok.metode.clone(),
        url: dok.base_url.clone(),
        body: Some("{}".to_owned()),
        trust,
        batas_total: Duration::from_secs(10),
    };
    Some(pemanggil.panggil(&kabel).await)
}

fn berhasil(h: &Option<Result<ResponsMentah, GagalPanggilProvider>>) -> bool {
    matches!(h, Some(Ok(r)) if r.status == 200)
}

fn gagal_tls(h: &Option<Result<ResponsMentah, GagalPanggilProvider>>) -> bool {
    let Some(Err(g)) = h else {
        return false;
    };
    let kelas = kelas_error(g);
    matches!(g, GagalPanggilProvider::Tls(_)) && kelas == ErrorClass::Tls && !kelas.retryable()
}

#[tokio::test]
async fn ca_kustom_dengan_ca_penerbit_berhasil() {
    let s = sertifikat("a");
    let mock = mock_https(&s).await;
    let dok = dokumen(
        &mock,
        &json!({"tls_trust": "ca_kustom", "tls_ca_pem": s.ca_pem}),
    );
    let h = panggil(&PemanggilReqwest::baru().unwrap(), &dok).await;
    assert!(
        berhasil(&h),
        "ca_kustom dengan CA yang benar harus lolos: {h:?}"
    );
    assert!(!mock.data.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ca_kustom_dengan_ca_lain_menjadi_tls_tanpa_data_aplikasi() {
    let s = sertifikat("a");
    let lain = sertifikat("b");
    let mock = mock_https(&s).await;
    let dok = dokumen(
        &mock,
        &json!({"tls_trust": "ca_kustom", "tls_ca_pem": lain.ca_pem}),
    );
    let h = panggil(&PemanggilReqwest::baru().unwrap(), &dok).await;
    assert!(
        gagal_tls(&h),
        "CA yang bukan penerbit harus menjadi tls: {h:?}"
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
        &json!({"tls_trust": "pin_spki", "tls_pin_spki": [lain.pin, s.pin]}),
    );
    let h = panggil(&PemanggilReqwest::baru().unwrap(), &dok).await;
    assert!(
        berhasil(&h),
        "pin_spki dengan pin yang cocok harus lolos: {h:?}"
    );
}

#[tokio::test]
async fn pin_spki_dengan_pin_lain_menjadi_tls_tanpa_data_aplikasi() {
    let s = sertifikat("a");
    let lain = sertifikat("b");
    let mock = mock_https(&s).await;
    let dok = dokumen(
        &mock,
        &json!({"tls_trust": "pin_spki", "tls_pin_spki": [lain.pin]}),
    );
    let h = panggil(&PemanggilReqwest::baru().unwrap(), &dok).await;
    assert!(
        gagal_tls(&h),
        "pin yang tidak cocok harus menjadi tls: {h:?}"
    );
    assert!(mock.data.lock().unwrap().is_empty(), "0 byte data aplikasi");
}

/// Satu pemanggil melayani dua provider dengan trust berbeda: klien tidak
/// tertukar (cache per bahan), dan pemanggilan ulang memakai klien yang sama.
#[tokio::test]
async fn satu_pemanggil_beberapa_provider_tidak_tertukar() {
    let a = sertifikat("a");
    let b = sertifikat("b");
    let mock_a = mock_https(&a).await;
    let mock_b = mock_https(&b).await;
    let dok_a = dokumen(
        &mock_a,
        &json!({"tls_trust": "ca_kustom", "tls_ca_pem": a.ca_pem}),
    );
    let dok_b = dokumen(
        &mock_b,
        &json!({"tls_trust": "pin_spki", "tls_pin_spki": [b.pin]}),
    );
    // Silang: dokumen A menunjuk mock B dan sebaliknya harus ditolak.
    let silang_a = dokumen(
        &mock_b,
        &json!({"tls_trust": "ca_kustom", "tls_ca_pem": a.ca_pem}),
    );
    let pemanggil = PemanggilReqwest::baru().unwrap();
    for _ in 0..2 {
        assert!(berhasil(&panggil(&pemanggil, &dok_a).await));
        assert!(berhasil(&panggil(&pemanggil, &dok_b).await));
        assert!(gagal_tls(&panggil(&pemanggil, &silang_a).await));
    }
}

/// PEM yang lolos `trust()` tetapi tidak bisa dibangun menjadi klien (bukan
/// sertifikat) ditolak oleh `siapkan_trust`, yang dipanggil handler sebelum
/// CAS (M-03), tanpa koneksi keluar. `panggil` sendiri tetap `tls` (tidak
/// pernah jatuh ke trust sistem) bila dipanggil tanpa persiapan itu.
#[tokio::test]
async fn pem_rusak_ditolak_siapkan_trust_dan_panggil_tetap_tls() {
    let s = sertifikat("a");
    let mock = mock_https(&s).await;
    let dok = dokumen(
        &mock,
        &json!({"tls_trust": "ca_kustom", "tls_ca_pem": "bukan pem"}),
    );
    let pemanggil = PemanggilReqwest::baru().unwrap();
    assert!(panggil(&pemanggil, &dok).await.is_none());

    let dok = baca_dokumen_provider(&dok).unwrap();
    let kabel = PermintaanKabel {
        metode: dok.metode.clone(),
        url: dok.base_url.clone(),
        body: Some("{}".to_owned()),
        trust: dok.trust().unwrap(),
        batas_total: Duration::from_secs(10),
    };
    assert!(gagal_tls(&Some(pemanggil.panggil(&kabel).await)));
    assert!(mock.data.lock().unwrap().is_empty());
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
    let pemanggil = PemanggilReqwest::baru().unwrap();
    for tls in kasus {
        let dok = dokumen(&mock, &tls);
        assert!(panggil(&pemanggil, &dok).await.is_none(), "{tls}");
    }
    assert!(mock.data.lock().unwrap().is_empty());
}
