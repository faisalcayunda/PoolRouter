//! Template rendering tests (ADR-0002 D1, D2; AC-20).
//!
//! Namespaces are a closed set per ADR-0001 §7: `input.*`, `secret.*`,
//! `system.kode_transaksi` and `endpoint.*`.

use std::collections::BTreeMap;

use poolrouter_adapter_provider::template::{Nilai, RenderError, TemplateRequest, render_template};

fn peta(pasangan: &[(&str, &str)]) -> BTreeMap<String, String> {
    pasangan
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn nilai_lengkap() -> Nilai {
    Nilai {
        sistem: peta(&[("kode_transaksi", "TRX-0001")]),
        input: peta(&[("NIK", "3200000000000001")]),
        secret: peta(&[("USER_ID", "u-sintetis"), ("KUNCI_UJI", "example")]),
        endpoint: peta(&[("TRESHOLD", "80")]),
    }
}

fn templat_get() -> TemplateRequest {
    TemplateRequest {
        method: "GET".to_string(),
        path: "/dukcapil/verify".to_string(),
        query: peta(&[
            ("NIK", "{{input.NIK}}"),
            ("TRESHOLD", "{{endpoint.TRESHOLD}}"),
        ]),
        header: peta(&[("X-Transaction-Id", "{{system.kode_transaksi}}")]),
        body: None,
    }
}

#[test]
fn merender_placeholder_di_path_query_dan_header() {
    let hasil = render_template(&templat_get(), &nilai_lengkap()).expect("harus berhasil");

    assert_eq!(hasil.path, "/dukcapil/verify");
    assert_eq!(hasil.query["NIK"], "3200000000000001");
    assert_eq!(hasil.query["TRESHOLD"], "80");
    assert_eq!(hasil.header["X-Transaction-Id"], "TRX-0001");
    assert_eq!(hasil.body, None);
}

#[test]
fn method_diteruskan_apa_adanya() {
    // D1: the template model is method-agnostic, so a POST body renders too.
    let templat = TemplateRequest {
        method: "POST".to_string(),
        path: "/dukcapil/verify".to_string(),
        query: BTreeMap::new(),
        header: peta(&[("Content-Type", "application/json")]),
        body: Some(r#"{"NIK":"{{input.NIK}}"}"#.to_string()),
    };

    let hasil = render_template(&templat, &nilai_lengkap()).expect("harus berhasil");

    assert_eq!(hasil.method, "POST");
    assert_eq!(hasil.body.as_deref(), Some(r#"{"NIK":"3200000000000001"}"#));
}

#[test]
fn placeholder_secret_terisi_dari_nilai() {
    let templat = TemplateRequest {
        method: "GET".to_string(),
        path: "/verify".to_string(),
        query: peta(&[("user", "{{secret.USER_ID}}")]),
        header: BTreeMap::new(),
        body: None,
    };

    let hasil = render_template(&templat, &nilai_lengkap()).expect("harus berhasil");
    assert_eq!(hasil.query["user"], "u-sintetis");
}

#[test]
fn placeholder_tak_dikenal_gagal_tertutup() {
    // U-43: no partially filled request may leave the adapter.
    let templat = TemplateRequest {
        method: "GET".to_string(),
        path: "/verify".to_string(),
        query: peta(&[("NIK", "{{input.NIK}}"), ("X", "{{input.TIDAK_ADA}}")]),
        header: BTreeMap::new(),
        body: None,
    };

    let galat = render_template(&templat, &nilai_lengkap()).expect_err("harus gagal");
    assert_eq!(
        galat,
        RenderError::PlaceholderTakDikenal("input.TIDAK_ADA".to_string())
    );
}

#[test]
fn placeholder_tanpa_awalan_gagal() {
    let templat = TemplateRequest {
        method: "GET".to_string(),
        path: "/verify/{{NIK}}".to_string(),
        query: BTreeMap::new(),
        header: BTreeMap::new(),
        body: None,
    };

    let galat = render_template(&templat, &nilai_lengkap()).expect_err("harus gagal");
    assert_eq!(galat, RenderError::TanpaAwalan("NIK".to_string()));
}

#[test]
fn namespace_metadata_ditolak() {
    // ADR-0001 §7: {{metadata.*}} must never reach a provider.
    let templat = TemplateRequest {
        method: "GET".to_string(),
        path: "/verify".to_string(),
        query: peta(&[("x", "{{metadata.client_id}}")]),
        header: BTreeMap::new(),
        body: None,
    };

    let galat = render_template(&templat, &nilai_lengkap()).expect_err("harus gagal");
    assert_eq!(
        galat,
        RenderError::PlaceholderTakDikenal("metadata.client_id".to_string())
    );
}

#[test]
fn kunci_system_selain_kode_transaksi_ditolak() {
    // D2: F0 closes {{system.*}} to kode_transaksi alone.
    let mut nilai = nilai_lengkap();
    nilai.sistem.insert("tenant".to_string(), "t-1".to_string());

    let templat = TemplateRequest {
        method: "GET".to_string(),
        path: "/verify".to_string(),
        query: peta(&[("t", "{{system.tenant}}")]),
        header: BTreeMap::new(),
        body: None,
    };

    let galat = render_template(&templat, &nilai).expect_err("harus gagal");
    assert_eq!(
        galat,
        RenderError::PlaceholderTakDikenal("system.tenant".to_string())
    );
}

#[test]
fn kurung_kurawal_tidak_utuh_gagal() {
    let templat = TemplateRequest {
        method: "GET".to_string(),
        path: "/verify/{{input.NIK".to_string(),
        query: BTreeMap::new(),
        header: BTreeMap::new(),
        body: None,
    };

    let galat = render_template(&templat, &nilai_lengkap()).expect_err("harus gagal");
    // Offset is absolute within the whole input, not the current slice.
    assert_eq!(galat, RenderError::KurungTidakUtuh(8));
}

#[test]
fn teks_tanpa_placeholder_diteruskan() {
    let templat = TemplateRequest {
        method: "GET".to_string(),
        path: "/dukcapil/verify".to_string(),
        query: peta(&[("mode", "full")]),
        header: BTreeMap::new(),
        body: None,
    };

    let hasil = render_template(&templat, &Nilai::default()).expect("harus berhasil");
    assert_eq!(hasil.query["mode"], "full");
}

#[test]
fn placeholder_diulang_beberapa_kali_terisi_semua() {
    let templat = TemplateRequest {
        method: "GET".to_string(),
        path: "/{{input.NIK}}/verify/{{input.NIK}}".to_string(),
        query: BTreeMap::new(),
        header: BTreeMap::new(),
        body: None,
    };

    let hasil = render_template(&templat, &nilai_lengkap()).expect("harus berhasil");
    assert_eq!(hasil.path, "/3200000000000001/verify/3200000000000001");
}

#[test]
fn placeholder_berdampingan_terisi_semua() {
    let templat = TemplateRequest {
        method: "GET".to_string(),
        path: "/{{input.NIK}}{{endpoint.TRESHOLD}}".to_string(),
        query: BTreeMap::new(),
        header: BTreeMap::new(),
        body: None,
    };

    let hasil = render_template(&templat, &nilai_lengkap()).expect("harus berhasil");
    assert_eq!(hasil.path, "/320000000000000180");
}

#[test]
fn placeholder_kosong_gagal() {
    let templat = TemplateRequest {
        method: "GET".to_string(),
        path: "/verify/{{}}".to_string(),
        query: BTreeMap::new(),
        header: BTreeMap::new(),
        body: None,
    };

    let galat = render_template(&templat, &nilai_lengkap()).expect_err("harus gagal");
    assert_eq!(galat, RenderError::Kosong(String::new()));
}

#[test]
fn literal_secret_di_parameter_literal_dikirim_apa_adanya() {
    // U-44: a value taken from `parameter_literal` (endpoint config) is
    // never re-rendered, so a literal `{{secret.X}}` cannot be smuggled
    // into the upstream request as an expanded secret.
    let mut nilai = nilai_lengkap();
    nilai
        .endpoint
        .insert("TRESHOLD".to_string(), "{{secret.KUNCI_UJI}}".to_string());

    let templat = TemplateRequest {
        method: "GET".to_string(),
        path: "/dukcapil/verify".to_string(),
        query: peta(&[("TRESHOLD", "{{endpoint.TRESHOLD}}")]),
        header: BTreeMap::new(),
        body: None,
    };

    let hasil = render_template(&templat, &nilai).expect("harus berhasil");
    assert_eq!(hasil.query["TRESHOLD"], "{{secret.KUNCI_UJI}}");
}

#[test]
fn render_get_dengan_body_menghasilkan_content_length_implicit() {
    // M2: render-level proof that a GET may carry a body (D1). The on-wire
    // Content-Length / chunked checks live in T6/T7 with a mock server.
    let templat = TemplateRequest {
        method: "GET".to_string(),
        path: "/dukcapil/verify".to_string(),
        query: BTreeMap::new(),
        header: BTreeMap::new(),
        body: Some(r#"{"NIK":"{{input.NIK}}"}"#.to_string()),
    };

    let hasil = render_template(&templat, &nilai_lengkap()).expect("harus berhasil");

    assert_eq!(hasil.method, "GET");
    assert_eq!(hasil.body.as_deref(), Some(r#"{"NIK":"3200000000000001"}"#));
}

#[test]
fn penutup_tanpa_pembuka_dibiarkan_literal() {
    // Only an unclosed opener is an error; a stray `}}` is plain text.
    let templat = TemplateRequest {
        method: "GET".to_string(),
        path: "/verify/a}}b".to_string(),
        query: BTreeMap::new(),
        header: BTreeMap::new(),
        body: None,
    };

    let hasil = render_template(&templat, &nilai_lengkap()).expect("harus berhasil");
    assert_eq!(hasil.path, "/verify/a}}b");
}
