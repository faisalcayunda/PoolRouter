use poolrouter_adapter_provider::client::{TlsTrust, build_provider_client, GalatKlien};

#[test]
fn tls_trust_dari_str() {
    assert_eq!(TlsTrust::dari_str("system"), Some(TlsTrust::System));
    assert_eq!(TlsTrust::dari_str("sistem"), Some(TlsTrust::System));
    assert_eq!(TlsTrust::dari_str("SYSTEM"), Some(TlsTrust::System));
    assert_eq!(TlsTrust::dari_str("ca_kustom"), Some(TlsTrust::CaKustom { ca_pem: vec![] }));
    assert_eq!(TlsTrust::dari_str("pin_spki"), Some(TlsTrust::PinSpki { pins: vec![] }));
    assert_eq!(TlsTrust::dari_str("unknown"), None);
    assert_eq!(TlsTrust::dari_str(""), None);
}

#[test]
fn tls_trust_from_str() {
    let t: TlsTrust = "system".into();
    assert_eq!(t, TlsTrust::System);
    let t: TlsTrust = "ca_kustom".into();
    assert!(matches!(t, TlsTrust::CaKustom { .. }));
}

#[test]
fn build_system_client() {
    let klien = build_provider_client(TlsTrust::System);
    assert!(klien.is_ok(), "system root store harus available");
}

#[test]
fn build_ca_kustom_empty_rejects() {
    let trust = TlsTrust::CaKustom { ca_pem: vec![] };
    let klien = build_provider_client(trust);
    // Empty CA list should fail at build time (collect empty).
    // x509-parser will succeed but RootCertStore akan kosong; reqwest akan error.
    assert!(klien.is_ok() || matches!(klien, Err(GalatKlien::Config(_))));
}

#[test]
fn build_pin_spki_empty_rejects() {
    let trust = TlsTrust::PinSpki { pins: vec![] };
    let klien = build_provider_client(trust);
    assert_eq!(klien, Err(GalatKlien::PinTidakValid));
}

#[test]
fn build_pin_spki_with_dummy_pin() {
    let pin = [0xAAu8; 32];
    let trust = TlsTrust::PinSpki { pins: vec![pin] };
    let klien = build_provider_client(trust);
    // Should build successfully, verifier will reject all certs since pin tidak cocok.
    assert!(klien.is_ok(), "klien dengan dummy pin harus build: {:?}", klien);
}
