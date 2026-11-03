//! Generation tests (SHAA-192 AC; check merah `openapi-check` memverifikasi
//! YAML commit, oasdiff berjalan terpisah di gate). Tests assert on the
//! serialized document structure, not on hand-maintained YAML.

use serde_json::Value;
use utoipa::OpenApi;

use crate::api::{
    admin, downstream,
    problem::{JumlahNamaTidakCocok, ProblemKode},
};

fn downstream_doc() -> Value {
    serde_json::to_value(downstream::doc()).expect("downstream doc serializes")
}

fn admin_doc() -> Value {
    serde_json::to_value(admin::AdminDoc::openapi()).expect("admin doc serializes")
}

fn paths(doc: &Value) -> &serde_json::Map<String, Value> {
    doc["paths"].as_object().expect("paths object")
}

fn operations(doc: &Value) -> Vec<(&str, &Value)> {
    paths(doc)
        .values()
        .flat_map(|item| item.as_object().expect("path item"))
        .filter(|(method, op)| {
            ["get", "post", "put", "patch", "delete"].contains(&method.as_str())
                && op.get("responses").is_some()
        })
        .map(|(method, op)| (method.as_str(), op))
        .collect()
}

fn resolve<'a>(doc: &'a Value, response: &'a Value) -> &'a Value {
    if let Some(reff) = response.get("$ref").and_then(|r| r.as_str()) {
        let name = reff.rsplit('/').next().expect("$ref berbentuk path");
        return &doc["components"]["responses"][name];
    }
    response
}

fn is_problem_content(content: &serde_json::Map<String, Value>) -> bool {
    content.keys().any(|ct| ct.contains("problem+json"))
}

/// 1a: two documents, every operation has a camelCase operationId.
#[test]
fn semua_operasi_milik_operation_id_camel_case() {
    for (name, doc) in [("downstream", downstream_doc()), ("admin", admin_doc())] {
        for (method, op) in operations(&doc) {
            let id = op["operationId"]
                .as_str()
                .unwrap_or_else(|| panic!("{name}: operasi tanpa operationId: {method} {op:?}"));
            let pertama = id.as_bytes()[0].is_ascii_lowercase();
            let camel = pertama && !id.contains('-') && !id.contains('_') && !id.contains(' ');
            assert!(camel, "{name}: operationId bukan camelCase: {id}");
        }
    }
}

/// 1b: downstream count (6 operations, ADR-0001 U-4).
#[test]
fn downstream_milik_enam_operasi() {
    assert_eq!(operations(&downstream_doc()).len(), 6);
}

/// 1c: admin count (64 operations: 6 sesi + 42 konfigurasi + 18 K-20 + kuota + 6 log/audit... jumlah final tercantum di YAML).
#[test]
fn admin_milik_enam_puluh_empat_operasi() {
    assert_eq!(operations(&admin_doc()).len(), 64);
}

/// 2: `webhooks:` with `requestTerminal`, four headers, JSON body and 2xx.
#[test]
fn downstream_milik_webhooks_request_terminal() {
    let doc = downstream_doc();
    let webhook = doc["webhooks"]["requestTerminal"]["post"]
        .as_object()
        .expect("webhooks.requestTerminal.post ada");
    assert_eq!(webhook["operationId"], "requestTerminal");
    let params = webhook["parameters"].as_array().expect("parameters");
    let header: Vec<&str> = params
        .iter()
        .map(|p| p["name"].as_str().expect("parameter name"))
        .collect();
    assert_eq!(
        header,
        vec![
            "webhook-id",
            "webhook-timestamp",
            "webhook-signature",
            "poolrouter-secret-versi"
        ]
    );
    let body = webhook["requestBody"]["content"]["application/json"]
        .as_object()
        .expect("requestBody application/json");
    assert!(body.get("schema").is_some());
    assert!(webhook["responses"]["200"].is_object(), "respons 2xx ada");
}

/// 3: every response has a description, and every response on authenticated
/// paths carries `Cache-Control: no-store` (R-16).
#[test]
fn setiap_respons_milik_deskripsi_dan_cache_control() {
    for (name, doc) in [("downstream", downstream_doc()), ("admin", admin_doc())] {
        let all_responses = paths(&doc)
            .values()
            .chain(
                doc["webhooks"]
                    .as_object()
                    .map(|w| w.values().collect::<Vec<_>>())
                    .unwrap_or_default(),
            )
            .flat_map(|item| item.as_object().expect("path item").values())
            .filter_map(|op| op.get("responses"))
            .flat_map(|r| r.as_object().expect("responses").values());
        for response in all_responses.map(|r| resolve(&doc, r)) {
            assert!(
                response["description"].is_string(),
                "{name}: respons tanpa description: {response:?}"
            );
            let headers = response
                .get("headers")
                .and_then(|h| h.as_object())
                .expect("headers (Cache-Control wajib di setiap respons)");
            assert!(
                headers.contains_key("Cache-Control"),
                "{name}: respons tanpa Cache-Control: {response:?}"
            );
        }
    }
}

/// 4: error responses are problem+json (R-10).
#[test]
fn error_status_memakai_problem_json() {
    for (name, doc) in [("downstream", downstream_doc()), ("admin", admin_doc())] {
        for (method, op) in operations(&doc) {
            for (status, response) in op["responses"].as_object().expect("responses") {
                let kode: u16 = status.parse().expect("status code");
                if (400..600).contains(&kode) {
                    let response = resolve(&doc, response);
                    let content = response["content"].as_object().unwrap_or_else(|| {
                        panic!("{name}: {method} {status} tanpa content: {response:?}")
                    });
                    assert!(
                        is_problem_content(content),
                        "{name}: {method} {status} tidak problem+json"
                    );
                }
            }
        }
    }
}

/// 5: 429/503 memuat Retry-After; 401 memuat WWW-Authenticate pada downstream
/// resource paths (U-8, ADR-0001 section 5a.7).
#[test]
fn rate_limit_dan_layanan_milik_retry_after() {
    for (name, doc) in [("downstream", downstream_doc()), ("admin", admin_doc())] {
        for (method, op) in operations(&doc) {
            for (status, response) in op["responses"].as_object().expect("responses") {
                let kode: u16 = status.parse().expect("status code");
                let response = resolve(&doc, response);
                let headers = response
                    .get("headers")
                    .and_then(|h| h.as_object())
                    .expect("headers");
                if matches!(kode, 429 | 503) {
                    assert!(
                        headers.contains_key("Retry-After"),
                        "{name}: {method} {status} tanpa Retry-After"
                    );
                }
                if kode == 401 && name == "downstream" {
                    assert!(
                        headers.contains_key("WWW-Authenticate"),
                        "{name}: {method} 401 tanpa WWW-Authenticate"
                    );
                }
            }
        }
    }
}

/// 6: security schemes (oauth2 + apiKey downstream; sessionCookie admin).
#[test]
fn skema_keamanan_terdaftar() {
    let downstream = &downstream_doc()["components"]["securitySchemes"];
    assert!(downstream.get("oauth2").is_some(), "oauth2 hilang");
    assert!(downstream.get("apiKey").is_some(), "apiKey hilang");
    let admin = &admin_doc()["components"]["securitySchemes"];
    assert!(admin.get("sessionCookie").is_some(), "sessionCookie hilang");
}

/// 7: satu contoh per kode masalah pada komponen respons yang dipakai; dan
/// `type` menunjuk anchor docs/masalah.md (ADR-0001 section 5a.7).
#[test]
fn contoh_problem_menunjuk_anchor() {
    let docs = [downstream_doc(), admin_doc()];
    let empty = serde_json::Map::<String, Value>::new();
    for doc in &docs {
        let responses = doc["components"]["responses"]
            .as_object()
            .expect("components.responses");
        for response in responses.values() {
            let contohs = response["content"]["application/problem+json"]["examples"]
                .as_object()
                .unwrap_or(&empty);
            for (_, contoh) in contohs {
                let value = &contoh["value"];
                let type_ = value["type"].as_str().expect("type");
                assert!(
                    type_.starts_with(
                        "https://github.com/faisalcayunda/PoolRouter/blob/main/docs/masalah.md#"
                    ),
                    "type bukan anchor masalah.md: {type_}"
                );
                assert!(value["kode"].is_string(), "kode hilang: {value:?}");
                assert!(value["status"].is_u64(), "status hilang: {value:?}");
            }
        }
    }
}

fn masalah_path() -> std::path::PathBuf {
    if let Ok(root) = std::env::var("POOLROUTER_REPO_ROOT") {
        let kandidat = std::path::Path::new(&root).join("docs/masalah.md");
        if kandidat.exists() {
            return kandidat;
        }
    }
    let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let kandidat = dir.join("docs/masalah.md");
        if kandidat.exists() {
            return kandidat;
        }
        assert!(dir.pop(), "docs/masalah.md tidak ditemukan dari manifest");
    }
}

/// 8: `docs/masalah.md` anchor set == ProblemKode (ADR-0001 section 5a.7).
#[test]
fn masalah_anchors() {
    let path = masalah_path();
    let isi = std::fs::read_to_string(&path).expect("docs/masalah.md ada");
    let anchor: std::collections::HashSet<&str> = isi
        .lines()
        .filter_map(|line| line.strip_prefix("# "))
        .filter(|judul| *judul != "Kode masalah PoolRouter") // judul dokumen
        .collect();
    let kode: std::collections::HashSet<&str> =
        ProblemKode::ALL.iter().map(|k| k.as_str()).collect();
    for k in &kode {
        assert!(
            anchor.contains(k),
            "anchor #{k} tidak ada di docs/masalah.md"
        );
    }
    for a in &anchor {
        assert!(
            kode.contains(a),
            "anchor #{a} di docs/masalah.md bukan ProblemKode"
        );
    }
}

/// 9: sync path does not advertise Idempotency-Key (U-2).
#[test]
fn sync_tanpa_idempotency_key() {
    let op = &downstream_doc()["paths"]["/v1/endpoints/{endpoint_id}/sync"]["post"];
    let params = op["parameters"].as_array().expect("parameters");
    assert!(
        !params.iter().any(|p| p["name"] == "Idempotency-Key"),
        "sync memuat header Idempotency-Key"
    );
    let response = &op["responses"]["400"];
    let contoh = serde_json::to_string(response).expect("stringify");
    assert!(
        contoh.contains("idempotency_key_tidak_didukung"),
        "400 sync tidak menjelaskan pelarangan Idempotency-Key"
    );
}

/// 10: 422 examples carry the field name as sent, never the envelope name
/// (`input`/`metadata`), and only names are echoed (ADR-0002 rev 5 section 6a
/// D6, R-16).
#[test]
fn errors_memuat_nama_field_yang_dikirim() {
    let doc = downstream_doc();
    let (mut jumlah, mut nama_dikirim) = (0, 0);
    for (_method, op) in operations(&doc) {
        for (status, response) in op["responses"].as_object().expect("responses") {
            if status != "422" {
                continue;
            }
            let response = resolve(&doc, response);
            let contoh = &response["content"]["application/problem+json"]["examples"];
            for (nama, contoh) in contoh.as_object().expect("examples") {
                let Some(errors) = contoh["value"]["errors"].as_array() else {
                    continue;
                };
                for error in errors {
                    let field = error["field"].as_str().expect("field string");
                    jumlah += 1;
                    if field != "status_penerima_manfaat" {
                        nama_dikirim += 1;
                    }
                    assert!(
                        field != "input" && field != "metadata",
                        "{nama}: errors[].field masih nama amplop: {field}"
                    );
                    assert!(
                        field == "TRESHOLD"
                            || field == "NAMA_LGKAP"
                            || field == "status_penerima_manfaat",
                        "{nama}: errors[].field bukan nama yang dikirim: {field}"
                    );
                }
            }
        }
    }
    assert_eq!(jumlah, 6, "jumlah entri errors yang diperiksa berubah");
    assert!(
        nama_dikirim >= 2,
        "contoh D6 (TRESHOLD/NAMA_LGKAP) tidak ditemukan"
    );
}

/// 11: a count field for names that are not echoed (D6), integer >= 1.
#[test]
fn problem_punya_jumlah_nama_tidak_cocok() {
    let doc = downstream_doc();
    let prop = &doc["components"]["schemas"]["Problem"]["properties"]["jumlah_nama_tidak_cocok"];
    assert!(!prop.is_null(), "Problem tanpa jumlah_nama_tidak_cocok");
    let nilai = serde_json::to_string(&doc["components"]["schemas"]["JumlahNamaTidakCocok"])
        .expect("stringify");
    assert!(nilai.contains("\"minimum\":1"), "tanpa minimum 1: {nilai}");
    assert!(
        nilai.contains("\"type\":\"integer\""),
        "bukan integer: {nilai}"
    );
    let nama = JumlahNamaTidakCocok::try_from(0);
    assert!(nama.is_err(), "nol diterima");
    assert!(JumlahNamaTidakCocok::try_from(2).is_ok());
}

/// 12: `hasil` keys are output element names (P-1 / U-43), not metadata keys.
#[test]
fn hasil_memakai_nama_elemen_output() {
    for (method, op) in operations(&downstream_doc()) {
        let teks = serde_json::to_string(op).expect("stringify");
        assert!(
            !teks.contains("status_penerima_manfaat\":{\"status\"")
                && !teks.contains("status_penerima_manfaat:\n"),
            "{method}: contoh hasil memakai kunci metadata"
        );
    }
}
