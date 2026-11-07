//! `error_class` mapping tests (AC-24, ADR-0001 R-03 table from ARC-4).

use poolrouter_adapter_provider::error::{GagalPanggilProvider, dari_status_http, kelas_error};
use poolrouter_core::retry::ErrorClass;

#[test]
fn timeout_dipetakan_ke_timeout() {
    assert_eq!(
        kelas_error(&GagalPanggilProvider::Timeout),
        ErrorClass::Timeout
    );
}

#[test]
fn koneksi_dipetakan_ke_koneksi() {
    assert_eq!(
        kelas_error(&GagalPanggilProvider::Koneksi("reset".into())),
        ErrorClass::Koneksi
    );
}

#[test]
fn tls_dipetakan_ke_tls() {
    assert_eq!(
        kelas_error(&GagalPanggilProvider::Tls("sertifikat".into())),
        ErrorClass::Tls
    );
}

#[test]
fn respons_tidak_valid_dipetakan_ke_respons_tidak_valid() {
    assert_eq!(
        kelas_error(&GagalPanggilProvider::ResponsTidakValid("envelope".into())),
        ErrorClass::ResponsTidakValid
    );
}

#[test]
fn http_401_dan_403_jadi_auth_ditolak() {
    for status in [401u16, 403] {
        assert_eq!(
            kelas_error(&dari_status_http(status)),
            ErrorClass::AuthDitolak,
            "status {status}"
        );
    }
}

#[test]
fn http_429_jadi_rate_limit() {
    assert_eq!(
        kelas_error(&dari_status_http(429)),
        ErrorClass::RateLimitProvider
    );
}

#[test]
fn http_5xx_jadi_provider_sementara() {
    for status in [500u16, 502, 503, 504] {
        assert_eq!(
            kelas_error(&dari_status_http(status)),
            ErrorClass::ProviderSementara,
            "status {status}"
        );
    }
}

#[test]
fn http_4xx_lain_jadi_lainnya_dan_final() {
    // A 400/404/422 is a configuration fault: retrying cannot fix it.
    for status in [400u16, 404, 422] {
        let kelas = kelas_error(&dari_status_http(status));
        assert_eq!(kelas, ErrorClass::Lainnya, "status {status}");
        assert!(!kelas.retryable(), "status {status} harus final");
    }
}

#[test]
fn kuota_habis_jadi_kuota_provider_habis() {
    let kelas = kelas_error(&GagalPanggilProvider::KuotaHabis);
    assert_eq!(kelas, ErrorClass::KuotaProviderHabis);
    assert!(!kelas.retryable());
}

#[test]
fn kelas_retry_sejalan_dengan_tabel_retry_arc4() {
    // The adapter may not invent retry behaviour; these four are the only
    // retryable outcomes in the shared table.
    assert!(kelas_error(&GagalPanggilProvider::Timeout).retryable());
    assert!(kelas_error(&GagalPanggilProvider::Koneksi("x".into())).retryable());
    assert!(kelas_error(&dari_status_http(429)).retryable());
    assert!(kelas_error(&dari_status_http(503)).retryable());

    assert!(!kelas_error(&GagalPanggilProvider::Tls("x".into())).retryable());
    assert!(!kelas_error(&GagalPanggilProvider::ResponsTidakValid("x".into())).retryable());
    assert!(!kelas_error(&dari_status_http(401)).retryable());
}

#[test]
fn status_tak_lazim_tetap_jadi_kelas() {
    // Defensive: a misbehaving provider must never panic the adapter. A 2xx
    // reaching this path is a caller error, so it is final, not retried.
    assert_eq!(
        kelas_error(&dari_status_http(599)),
        ErrorClass::ProviderSementara
    );
    assert_eq!(kelas_error(&dari_status_http(200)), ErrorClass::Lainnya);
    assert!(!kelas_error(&dari_status_http(200)).retryable());
}
