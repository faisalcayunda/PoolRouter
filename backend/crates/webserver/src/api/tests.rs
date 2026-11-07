//! Generation tests (SHAA-192 AC; check merah `openapi-check` memverifikasi
//! YAML commit, oasdiff berjalan terpisah di gate). Tests assert on the
//! serialized document structure, not on hand-maintained YAML.

use serde_json::Value;
use utoipa::OpenApi;

use crate::api::{
    admin, downstream, enums,
    problem::{JumlahNamaTidakCocok, ProblemKodeAdmin, ProblemKodeDown},
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

/// 1c: admin count (67 operations: sesi, konfigurasi, kuota, log/audit, dan
/// operasi K-20; daftarnya ditentukan di `admin-api.yaml`).
#[test]
fn admin_milik_enam_puluh_tujuh_operasi() {
    assert_eq!(operations(&admin_doc()).len(), 67);
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

/// 8: `docs/masalah.md` stays one combined document, but the code sets are per
/// document and disjoint where they must be (D-2, ADR-0001 section 5a.7):
/// the union of both documents equals the anchors, and each document's set is
/// a subset of the anchors.
#[test]
fn masalah_anchors() {
    let path = masalah_path();
    let isi = std::fs::read_to_string(&path).expect("docs/masalah.md ada");
    let anchor: std::collections::HashSet<&str> = isi
        .lines()
        .filter_map(|line| line.strip_prefix("# "))
        .filter(|judul| *judul != "Kode masalah PoolRouter") // judul dokumen
        .collect();
    let down: std::collections::HashSet<&str> =
        ProblemKodeDown::ALL.iter().map(|k| k.as_str()).collect();
    let admin: std::collections::HashSet<&str> =
        ProblemKodeAdmin::ALL.iter().map(|k| k.as_str()).collect();

    for (name, kode) in [("downstream", &down), ("admin", &admin)] {
        for k in kode {
            assert!(
                anchor.contains(k),
                "anchor #{k} ({name}) tidak ada di docs/masalah.md"
            );
        }
    }

    let gabungan: std::collections::HashSet<&str> = down.union(&admin).copied().collect();
    for a in &anchor {
        assert!(
            gabungan.contains(a),
            "anchor #{a} di docs/masalah.md bukan kode downstream maupun admin"
        );
    }
    let yatim: Vec<&str> = anchor.difference(&gabungan).copied().collect();
    assert!(
        yatim.is_empty(),
        "anchor tanpa kode di dokumen mana pun: {yatim:?}"
    );
}

/// D-2: the downstream contract never exposes an admin-only code, and vice
/// versa. Guards against re-merging the two enum sets.
#[test]
fn kode_admin_tidak_bocor_ke_downstream() {
    use crate::api::{problem::ProblemKodeAdmin as A, problem::ProblemKodeDown as D};
    let admin_only = [
        A::KredensialSalah,
        A::LoginDibatasi,
        A::OriginTidakValid,
        A::AksesDitolak,
        A::PencarianNikDilarang,
        A::SecretDipakai,
        A::KredensialDicabut,
        A::BatasKredensial,
        A::WebhookDomainTidakDiizinkan,
        A::WebhookTidakHttps,
        A::MetadataWajibHilang,
        A::EksporTerlaluBesar,
    ];
    let down: Vec<&str> = D::ALL.iter().map(|k| k.as_str()).collect();
    for kode in admin_only {
        assert!(
            !down.contains(&kode.as_str()),
            "kode admin #{} bocor ke kontrak downstream",
            kode.as_str()
        );
    }
    assert_eq!(
        down.len(),
        16,
        "kontrak downstream memuat 16 kode (D-2): {down:?}"
    );
}

/// D-2: `posisi` is an admin-only concept; the downstream `ValidasiError` must
/// not carry it.
#[test]
fn posisi_tidak_ada_di_downstream() {
    let doc = downstream_doc();
    let err = &doc["components"]["schemas"]["ValidasiErrorDown"];
    let props = err["properties"].as_object().expect("properti errors");
    assert!(
        !props.contains_key("posisi"),
        "posisi bocor ke downstream: {props:?}"
    );
    let admin = admin_doc();
    let err_admin = &admin["components"]["schemas"]["ValidasiErrorAdmin"];
    assert!(
        err_admin["properties"]
            .as_object()
            .expect("properti errors admin")
            .contains_key("posisi"),
        "posisi harus ada di admin"
    );
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
    // Dua kasus diperiksa (11a/11b). Batas bawah, bukan jumlah pasti: menambah
    // contoh 422 tidak boleh merusak test ini tanpa ada regresi.
    assert!(
        jumlah >= 4,
        "kurang dari dua contoh 422 yang diperiksa: {jumlah} entri errors"
    );
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

fn schema<'a>(doc: &'a Value, nama: &str) -> &'a Value {
    doc["components"]["schemas"]
        .get(nama)
        .unwrap_or_else(|| panic!("skema {nama} tidak ada"))
}

fn properti(doc: &Value, nama: &str) -> std::collections::BTreeSet<String> {
    schema(doc, nama)["properties"]
        .as_object()
        .unwrap_or_else(|| panic!("skema {nama} tanpa properties"))
        .keys()
        .cloned()
        .collect()
}

/// A-4: the read model must expose every writable config field (minus `alasan`)
/// so the Web UI edit form (US-01/04/05/15) can be pre-filled from a read. The
/// PUT operations are full replacement: a missing field in the response means
/// the form submits a default and silently wipes the current configuration.
#[test]
fn entitas_respons_memuat_semua_field_config() {
    let doc = admin_doc();
    let pasangan = [
        ("Client", "ClientConfig"),
        ("DownstreamPool", "DownstreamPoolConfig"),
        ("Endpoint", "EndpointConfig"),
        ("Provider", "ProviderConfig"),
        ("UpstreamPool", "UpstreamPoolConfig"),
    ];
    for (entitas, config) in pasangan {
        let punya_entitas = properti(&doc, entitas);
        let wajib_config = properti(&doc, config);
        let selisih: Vec<&String> = wajib_config
            .iter()
            .filter(|f| f.as_str() != "alasan")
            .filter(|f| !punya_entitas.contains(*f))
            .collect();
        assert!(
            selisih.is_empty(),
            "{entitas} tidak memuat field {config}: {selisih:?}"
        );
    }
}

/// A-4: the PUT bodies are `*Config` references, so the pairing above really is
/// the read/write contract of the edit form.
#[test]
fn update_memakai_body_config_yang_sama() {
    let doc = admin_doc();
    let pasangan = [
        ("updateClient", "ClientConfig"),
        ("updateDownstreamPool", "DownstreamPoolConfig"),
        ("updateEndpoint", "EndpointConfig"),
        ("updateProvider", "ProviderConfig"),
        ("updateUpstreamPool", "UpstreamPoolConfig"),
    ];
    for (operasi, config) in pasangan {
        let op = operations(&doc)
            .into_iter()
            .find(|(_, op)| op["operationId"] == operasi)
            .unwrap_or_else(|| panic!("operasi {operasi} tidak ada"));
        let teks = serde_json::to_string(&op.1["requestBody"]).expect("stringify");
        assert!(
            teks.contains(&format!("#/components/schemas/{config}")),
            "{operasi} tidak memakai body {config}: {teks}"
        );
    }
}

/// Apakah `byte` termasuk kelas `[A-Za-z0-9_]` (setara `\w` pada ASCII).
fn kelas_kata(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Setara `\b` pada kedua ujung rentang `awal..akhir`: sisi dalam adalah kelas
/// kata dan sisi luar bukan kelas kata. Mencegah `R-16` cocok di tengah kata
/// seperti `FXR-16`.
fn kata_utuh(byte: &[u8], awal: usize, akhir: usize) -> bool {
    let kiri = awal == 0 || !kelas_kata(byte[awal - 1]);
    let kanan = akhir >= byte.len() || !kelas_kata(byte[akhir]);
    kiri && kanan
}

/// Apakah kata `section` di `awal` dirujuk ke RFC bernomor, misalnya
/// `RFC 6749 section 5.2`. Rujukan seperti itu adalah rujukan standar publik, bukan
/// dokumen kerja internal, jadi penjaga membiarkannya.
fn dirujuk_ke_rfc(byte: &[u8], awal: usize) -> bool {
    let sebelum = &byte[..awal];
    let Some(mulai_rfc) = sebelum.windows(4).rposition(|w| w == b"RFC ") else {
        return false;
    };
    // Harus ada nomor setelah "RFC " dan sebelum kata `section`.
    let nomor = &sebelum[mulai_rfc + 4..];
    let nomor = nomor.strip_suffix(b" ").unwrap_or(nomor);
    !nomor.is_empty() && nomor.iter().all(u8::is_ascii_digit)
}

/// Temuan pertama di satu baris teks klien, atau `None` bila bersih.
///
/// Aturannya: teks yang tampil ke klien tidak boleh memuat rujukan dokumen kerja
/// internal (PRD, ADR, data contract). Aturannya harus ditulis sebagai kalimat,
/// misalnya "Selalu `no-store`; respons tidak boleh disimpan cache".
///
/// Rujukan standar publik tetap boleh, misalnya `RFC 6749 section 5.2` dan
/// `RFC 9457`. Istilah produk "kontrak endpoint" juga boleh. Sebaliknya, kata
/// "contract"/"Contract" ditolak karena di dokumen ini selalu berarti data
/// contract internal.
fn temuan_rujukan_dokumen_kerja(baris: &str) -> Option<String> {
    let byte = baris.as_bytes();
    for (awal, karakter) in baris.char_indices() {
        let ekor = &byte[awal..];

        // `§`
        if karakter == '§' {
            return Some("§".to_string());
        }

        // `ADR-\d+`: digit bisa lebih dari satu, jadi rentangnya harus memakan
        // seluruh deretan angka agar batas kanan tidak jatuh di tengah angka.
        if ekor.starts_with(b"ADR-") && ekor.get(4).is_some_and(u8::is_ascii_digit) {
            let mut akhir = 5;
            while ekor.get(akhir).is_some_and(u8::is_ascii_digit) {
                akhir += 1;
            }
            if kata_utuh(byte, awal, awal + akhir) {
                return Some("ADR".to_string());
            }
        }

        // `[Cc]ontract`
        if (ekor.starts_with(b"contract") || ekor.starts_with(b"Contract"))
            && kata_utuh(byte, awal, awal + 8)
        {
            return Some("contract".to_string());
        }

        // `section`, kecuali yang dirujuk ke RFC tertentu: "RFC 6749 section 5.2"
        // boleh, "section 3.2.1" tidak.
        if ekor.starts_with(b"section")
            && kata_utuh(byte, awal, awal + 7)
            && !dirujuk_ke_rfc(byte, awal)
        {
            return Some("section".to_string());
        }

        // `SHAA-\d+`
        if ekor.starts_with(b"SHAA-") && ekor.get(5).is_some_and(u8::is_ascii_digit) {
            let mut akhir = 6;
            while ekor.get(akhir).is_some_and(u8::is_ascii_digit) {
                akhir += 1;
            }
            if kata_utuh(byte, awal, awal + akhir) {
                return Some("SHAA".to_string());
            }
        }

        // `F0`: penanda tahap internal, jangan dipakai di teks klien. "F0" dicari
        // sebagai kata utuh agar tidak menabrak potongan id atau payload.
        if ekor.starts_with(b"F0") && kata_utuh(byte, awal, awal + 2) {
            return Some("F0".to_string());
        }

        // `\b[A-Z]{1,2}-\d+\b`: R-16, U-3, S-05, W-08, K-20, L-04, dan sejenisnya.
        let awalan = ekor.iter().take_while(|c| c.is_ascii_uppercase()).count();
        if (1..=2).contains(&awalan)
            && ekor.get(awalan) == Some(&b'-')
            && ekor.get(awalan + 1).is_some_and(u8::is_ascii_digit)
        {
            let mut akhir = awalan + 2;
            while ekor.get(akhir).is_some_and(u8::is_ascii_digit) {
                akhir += 1;
            }
            if kata_utuh(byte, awal, awal + akhir) {
                return Some(baris[awal..awal + akhir].to_string());
            }
        }
    }
    None
}

/// Penjaga: `downstream-api.yaml` diserahkan ke BGN sebagai lampiran
/// dokumentasi, jadi seluruh teksnya, beserta `docs/masalah.md`, harus terbaca
/// tanpa dokumen internal. Rujukan di komentar kode Rust tetap boleh ada;
/// `admin-api.yaml` tidak diperiksa karena tidak diserahkan.
#[test]
fn teks_client_tanpa_rujukan_dokumen_kerja() {
    let root = masalah_path()
        .parent()
        .and_then(|d| d.parent())
        .expect("repo root")
        .to_path_buf();
    let berkas = [
        (
            "backend/openapi/downstream-api.yaml",
            std::fs::read_to_string(root.join("backend/openapi/downstream-api.yaml"))
                .expect("downstream-api.yaml ada"),
        ),
        (
            "docs/masalah.md",
            std::fs::read_to_string(masalah_path()).expect("docs/masalah.md ada"),
        ),
    ];
    for (nama, isi) in berkas {
        for (nomor, baris) in isi.lines().enumerate() {
            if let Some(temuan) = temuan_rujukan_dokumen_kerja(baris) {
                panic!(
                    "{nama}:{} memuat rujukan dokumen kerja ({temuan}): {baris}",
                    nomor + 1
                );
            }
        }
    }
}

/// Penjaga di atas harus terbukti dua arah: menangkap semua bentuk rujukan,
/// termasuk nomor lebih dari satu digit, dan tetap meloloskan rujukan standar
/// publik serta istilah produk.
#[test]
fn penjaga_rujukan_menangkap_dan_meloloskan_contoh() {
    let ditolak = [
        "Lihat R-16 untuk detail.",
        "Sesuai U-4 dan U-43.",
        "Daftar K-20 dan L-04 serta MT-03.",
        "Sesuai ADR-0001.",
        "ADR-3 juga.",
        "Lampiran SHAA-175.",
        "section 3.2.1 menjelaskan hal ini.",
        "Sesuai contract section 3.2.",
        "Berlaku sejak contract v0.7.10.",
        "Ini data contract internal.",
        "Ini data Contract internal.",
        "Rujuk §3a.7.",
        "Jendela retry habis di F0.",
    ];
    for baris in ditolak {
        assert!(
            temuan_rujukan_dokumen_kerja(baris).is_some(),
            "penjaga meloloskan rujukan dokumen kerja: {baris}"
        );
    }

    let diloloskan = [
        "Selalu `no-store`; respons tidak boleh disimpan cache.",
        "Galat memakai RFC 9457 problem+json.",
        "Kode galat mengikuti RFC 6749 section 5.2.",
        "Kunci metadata tidak ada di kontrak endpoint.",
        "Satu field input pada kontrak endpoint.",
        "Nilai `FXR-16` bukan kode dokumen kerja.",
    ];
    for baris in diloloskan {
        assert_eq!(
            temuan_rujukan_dokumen_kerja(baris),
            None,
            "penjaga salah menolak teks klien: {baris}"
        );
    }
}

/// One row of the KP-01 table: the bounds the schema must advertise and the
/// default, if any. `default: None` means the field is required and the schema
/// must not carry a `default` for it.
struct BarisKp01 {
    nama: &'static str,
    minimum: Option<f64>,
    maximum: Option<f64>,
    default: Option<&'static str>,
}

/// KP-01 table as decided by Shu for SHAA-280 (ADR-0001 section 5a "Parameter
/// per upstream pool dan rentangnya"). Kept as the expected values in one place
/// so a later edit to `UpstreamPoolConfig` cannot silently drop a bound.
const KP01: &[BarisKp01] = &[
    // Required: no default (Shu, SHAA-280).
    BarisKp01 {
        nama: "kuota_harian",
        minimum: Some(1.0),
        maximum: None,
        default: None,
    },
    BarisKp01 {
        nama: "rate_limit_per_jam",
        minimum: Some(1.0),
        maximum: None,
        default: None,
    },
    BarisKp01 {
        nama: "porsi_sync",
        minimum: Some(0.0),
        maximum: Some(1.0),
        default: None,
    },
    BarisKp01 {
        nama: "jeda_min",
        minimum: Some(0.0),
        maximum: Some(60_000.0),
        default: None,
    },
    BarisKp01 {
        nama: "jeda_maks",
        minimum: Some(0.0),
        maximum: Some(60_000.0),
        default: None,
    },
    // `ambang_berdampingan` has a lower bound only.
    BarisKp01 {
        nama: "ambang_berdampingan",
        minimum: Some(1.0),
        maximum: None,
        default: None,
    },
    // KP-01 defaults kept.
    BarisKp01 {
        nama: "margin_kuota",
        minimum: Some(0.001),
        maximum: Some(0.2),
        default: Some("0.01"),
    },
    BarisKp01 {
        nama: "batas_percobaan",
        minimum: Some(1.0),
        maximum: Some(5.0),
        default: Some("3"),
    },
    BarisKp01 {
        nama: "retry_backoff_base_detik",
        minimum: Some(1.0),
        maximum: Some(300.0),
        default: Some("5"),
    },
    BarisKp01 {
        nama: "retry_backoff_cap_detik",
        minimum: Some(1.0),
        maximum: Some(3600.0),
        default: Some("300"),
    },
    BarisKp01 {
        nama: "retry_rasio",
        minimum: Some(0.0),
        maximum: Some(0.5),
        default: Some("0.1"),
    },
    BarisKp01 {
        nama: "retry_min_per_jam",
        minimum: Some(0.0),
        maximum: Some(10_000.0),
        default: Some("20"),
    },
];

/// Fields that must never advertise a `default` in the write body
/// (`provider_id`, the jeda triad, the three quota inputs) plus `alasan`, which
/// is context rather than a setting.
const TANPA_DEFAULT: &[&str] = &[
    "provider_id",
    "kuota_harian",
    "rate_limit_per_jam",
    "porsi_sync",
    "jeda_min",
    "jeda_maks",
    "ambang_berdampingan",
    "alasan",
];

fn angka(value: &Value) -> Option<f64> {
    value.as_f64()
}

/// SHAA-280 AC 1/2: the upstream pool body carries the KP-01 ranges decided for
/// SHAA-280, the seven required fields advertise no `default`, and the
/// cross-field formula is written into the `description` of both jeda fields and
/// of `UpstreamPoolConfig` itself (JSON Schema cannot express it as a bound).
#[test]
fn e2e_upstream_pool() {
    let doc = admin_doc();
    let skema = &doc["components"]["schemas"]["UpstreamPoolConfig"];
    let properti = skema["properties"]
        .as_object()
        .expect("properties UpstreamPoolConfig");
    for baris in KP01 {
        let nama = baris.nama;
        assert!(
            properti.contains_key(nama),
            "field {nama} hilang dari UpstreamPoolConfig"
        );
    }
    for baris in KP01 {
        let nama = baris.nama;
        let field = &properti[nama];
        assert_eq!(
            angka(&field["minimum"]),
            baris.minimum,
            "{nama}: minimum tidak sesuai KP-01: {field}"
        );
        assert_eq!(
            angka(&field["maximum"]),
            baris.maximum,
            "{nama}: maximum tidak sesuai KP-01: {field}"
        );
        match baris.default {
            Some(nilai) => {
                assert_eq!(
                    angka(&field["default"]),
                    Some(nilai.parse::<f64>().expect("default numerik")),
                    "{nama}: default tidak sesuai KP-01: {field}"
                );
                assert_ne!(
                    angka(&field["example"]),
                    angka(&field["default"]),
                    "{nama}: example sama dengan default, mudah terbaca sebagai default"
                );
            }
            None => assert!(
                field.get("default").is_none(),
                "{nama}: tidak boleh punya default (wajib diisi): {field}"
            ),
        }
    }
    // `porsi_sync` is a fraction, so a kept `default` must stay a JSON number.
    assert!(
        properti["porsi_sync"].get("default").is_none(),
        "porsi_sync tidak boleh punya default: {}",
        properti["porsi_sync"]
    );

    // `ambang_berdampingan` has no upper bound at all (Shu, SHAA-280): not a
    // missing key in the table, but an explicitly absent `maximum`.
    assert!(
        properti["ambang_berdampingan"].get("maximum").is_none(),
        "ambang_berdampingan tidak boleh punya maximum: {}",
        properti["ambang_berdampingan"]
    );

    // AC 2: the cross-field formula lives in the descriptions a client reads.
    let formula = "floor(3_600_000 / rate_limit_per_jam)";
    for nama in ["jeda_min", "jeda_maks"] {
        let deskripsi = properti[nama]["description"]
            .as_str()
            .unwrap_or_else(|| panic!("{nama} tanpa description"));
        assert!(
            deskripsi.contains(formula),
            "{nama}: description tanpa rumus {formula}: {deskripsi}"
        );
    }
    let deskripsi_skema = skema["description"]
        .as_str()
        .expect("description UpstreamPoolConfig");
    assert!(
        deskripsi_skema.contains(formula),
        "UpstreamPoolConfig: description tanpa rumus {formula}: {deskripsi_skema}"
    );
    assert!(
        deskripsi_skema.contains("retry_backoff_base_detik"),
        "UpstreamPoolConfig: description tanpa batas silang retry cap: {deskripsi_skema}"
    );

    // Fields outside the KP-01 table still carry a bound; reset_kuota_* keep
    // their KP-02 defaults.
    assert_eq!(
        angka(&properti["reset_kuota_jam"]["minimum"]),
        Some(0.0),
        "reset_kuota_jam tanpa minimum"
    );
    assert_eq!(
        angka(&properti["reset_kuota_jam"]["maximum"]),
        Some(23.0),
        "reset_kuota_jam tanpa maximum"
    );
    assert!(properti["reset_kuota_jam"]["default"].is_number());
    let zona = &properti["reset_kuota_zona"];
    assert!(
        zona["default"].as_str().is_some_and(|z| !z.is_empty()),
        "reset_kuota_zona tanpa default: {zona}"
    );
    assert_ne!(
        zona["example"], zona["default"],
        "reset_kuota_zona: example sama dengan default"
    );

    // The seven required fields plus `alasan` (context, not a setting) must
    // never advertise a default.
    for tanpa_default in TANPA_DEFAULT {
        assert!(
            properti[*tanpa_default].get("default").is_none(),
            "{tanpa_default} tidak boleh punya default"
        );
    }

    // The 422 is still advertised on both write operations.
    for (method, op) in operations(&doc) {
        if op["operationId"] != "createUpstreamPool" && op["operationId"] != "updateUpstreamPool" {
            continue;
        }
        let status: Vec<&str> = op["responses"]
            .as_object()
            .expect("responses")
            .keys()
            .map(String::as_str)
            .collect();
        assert!(
            status.contains(&"422"),
            "{method}: {op:?} tanpa respons 422"
        );
    }
}

/// Valid body at the KP-01 boundaries as decided for SHAA-280; each unit test
/// below starts from this and changes exactly one or two fields.
fn pool_valid() -> admin::UpstreamPoolConfig {
    admin::UpstreamPoolConfig {
        provider_id: uuid::Uuid::nil(),
        kuota_harian: 500,
        rate_limit_per_jam: 60,
        porsi_sync: 0.2,
        margin_kuota: 0.01,
        batas_percobaan: 3,
        retry_backoff_base_detik: 5,
        retry_backoff_cap_detik: 300,
        retry_rasio: 0.1,
        retry_min_per_jam: 20,
        jendela_kuota: enums::JendelaKuota::Bergulir24j,
        reset_kuota_jam: 0,
        reset_kuota_zona: "Asia/Jakarta".to_string(),
        jeda_min: 200,
        jeda_maks: 800,
        ambang_berdampingan: 4,
        alasan: None,
    }
}

fn field_errors(pool: &admin::UpstreamPoolConfig) -> Vec<String> {
    pool.validate()
        .expect_err("body seharusnya ditolak")
        .into_iter()
        .map(|e| e.field.to_string())
        .collect()
}

/// AC 3: the pure validator accepts the reference body.
#[test]
fn validate_pool_valid_lolos() {
    assert_eq!(pool_valid().validate(), Ok(()));
}

/// AC 3: every field with both bounds accepts them exactly at `minimum` and at
/// `maximum` (inclusive), and rejects one unit outside either end.
#[test]
fn validate_pool_batas_bawah_atas_dan_di_luar() {
    // (field name, setter to min, setter to max); the body is otherwise valid so
    // the cross-field rules also hold at each boundary.
    // `kuota_harian` and `rate_limit_per_jam` keep the pre-SHAA-280 range:
    // `minimum = 1` only, no maximum (Shu, 2026-11-04).
    let mut pool = pool_valid();
    pool.kuota_harian = 1;
    assert_eq!(pool.validate(), Ok(()), "kuota_harian = minimum");
    pool.kuota_harian = 1_000_001;
    assert_eq!(pool.validate(), Ok(()), "kuota_harian tanpa maximum");
    pool.kuota_harian = 0;
    assert_eq!(field_errors(&pool), ["kuota_harian"]);

    let mut pool = pool_valid();
    pool.rate_limit_per_jam = 1;
    // rate_limit_per_jam = 1 caps the delays at 3_600_000 ms, which the jeda
    // fields stay under.
    assert_eq!(pool.validate(), Ok(()), "rate_limit_per_jam = minimum");
    pool.rate_limit_per_jam = 36_000;
    // 3_600_000 / 36_000 = 100 ms, so the delays must come down with it.
    pool.jeda_min = 50;
    pool.jeda_maks = 100;
    assert_eq!(pool.validate(), Ok(()), "rate_limit_per_jam tanpa maximum");
    pool.jeda_maks = 800;
    assert_eq!(field_errors(&pool), ["jeda_maks"], "ceiling ikut turun");
    pool.jeda_maks = 100;
    pool.rate_limit_per_jam = 0;
    // Non-positive rate: only the range error, no spurious delay error.
    assert_eq!(field_errors(&pool), ["rate_limit_per_jam"]);

    let mut pool = pool_valid();
    pool.batas_percobaan = 1;
    assert_eq!(pool.validate(), Ok(()), "batas_percobaan = minimum");
    pool.batas_percobaan = 5;
    assert_eq!(pool.validate(), Ok(()), "batas_percobaan = maximum");
    pool.batas_percobaan = 0;
    assert_eq!(field_errors(&pool), ["batas_percobaan"]);
    pool.batas_percobaan = 6;
    assert_eq!(field_errors(&pool), ["batas_percobaan"]);

    let mut pool = pool_valid();
    pool.retry_backoff_base_detik = 1;
    pool.retry_backoff_cap_detik = 1;
    assert_eq!(pool.validate(), Ok(()), "base = minimum, cap = base");
    pool.retry_backoff_base_detik = 300;
    pool.retry_backoff_cap_detik = 300;
    assert_eq!(pool.validate(), Ok(()), "base = maximum");
    pool.retry_backoff_base_detik = 0;
    assert_eq!(field_errors(&pool), ["retry_backoff_base_detik"]);
    pool.retry_backoff_base_detik = 301;
    // Out of range *and* above the cap: two different fields, so both are
    // reported. (One error per field, not one per rule.)
    assert_eq!(
        field_errors(&pool),
        ["retry_backoff_base_detik", "retry_backoff_cap_detik"]
    );

    let mut pool = pool_valid();
    pool.retry_backoff_cap_detik = 3_600;
    assert_eq!(pool.validate(), Ok(()), "cap = maximum");
    pool.retry_backoff_cap_detik = 3_601;
    assert_eq!(field_errors(&pool), ["retry_backoff_cap_detik"]);

    let mut pool = pool_valid();
    pool.retry_rasio = 0.0;
    assert_eq!(pool.validate(), Ok(()), "retry_rasio = minimum");
    pool.retry_rasio = 0.5;
    assert_eq!(pool.validate(), Ok(()), "retry_rasio = maximum");
    pool.retry_rasio = 0.500_1;
    assert_eq!(field_errors(&pool), ["retry_rasio"]);

    let mut pool = pool_valid();
    pool.retry_min_per_jam = 0;
    assert_eq!(pool.validate(), Ok(()), "retry_min_per_jam = minimum");
    pool.retry_min_per_jam = 10_000;
    assert_eq!(pool.validate(), Ok(()), "retry_min_per_jam = maximum");
    pool.retry_min_per_jam = 10_001;
    assert_eq!(field_errors(&pool), ["retry_min_per_jam"]);

    let mut pool = pool_valid();
    pool.margin_kuota = 0.001;
    assert_eq!(pool.validate(), Ok(()), "margin_kuota = minimum");
    pool.margin_kuota = 0.2;
    assert_eq!(pool.validate(), Ok(()), "margin_kuota = maximum");
    pool.margin_kuota = 0.000_9;
    assert_eq!(field_errors(&pool), ["margin_kuota"]);
    pool.margin_kuota = 0.201;
    assert_eq!(field_errors(&pool), ["margin_kuota"]);

    let mut pool = pool_valid();
    pool.porsi_sync = 0.0;
    assert_eq!(pool.validate(), Ok(()), "porsi_sync = minimum");
    pool.porsi_sync = 1.0;
    assert_eq!(pool.validate(), Ok(()), "porsi_sync = maximum");
    pool.porsi_sync = 1.01;
    assert_eq!(field_errors(&pool), ["porsi_sync"]);

    let mut pool = pool_valid();
    pool.reset_kuota_jam = 23;
    assert_eq!(pool.validate(), Ok(()), "reset_kuota_jam = maximum");
    pool.reset_kuota_jam = 24;
    assert_eq!(field_errors(&pool), ["reset_kuota_jam"]);
    pool.reset_kuota_jam = -1;
    assert_eq!(field_errors(&pool), ["reset_kuota_jam"]);
}

/// AC 3: `ambang_berdampingan` is bounded below only, at exactly 1, with no
/// maximum at all (Shu, SHAA-280 replaced the previous 1..=100 proposal).
#[test]
fn validate_pool_ambang_berdampingan_tanpa_maksimum() {
    let mut pool = pool_valid();
    pool.ambang_berdampingan = 1;
    assert_eq!(pool.validate(), Ok(()), "ambang_berdampingan = minimum");
    pool.ambang_berdampingan = 0;
    assert_eq!(field_errors(&pool), ["ambang_berdampingan"]);
    // Large values are accepted: there is no ceiling.
    pool.ambang_berdampingan = 1_000_000;
    assert_eq!(pool.validate(), Ok(()), "tanpa maximum");
}

/// AC 3: the jeda delay ceiling follows `floor(3_600_000 / rate_limit_per_jam)`,
/// and both boundaries of that formula hold: for 60 per hour the ceiling is
/// exactly 60_000 ms (accepted), one millisecond above it is refused, and at
/// 36_000 per hour the ceiling is 100 ms.
#[test]
fn validate_pool_batas_jeda_mengikuti_rate_limit() {
    let mut pool = pool_valid();
    pool.rate_limit_per_jam = 60;
    assert_eq!(
        pool.batas_jeda_dari_rate_limit(),
        Some(60_000),
        "3_600_000 / 60"
    );
    pool.jeda_min = 60_000;
    pool.jeda_maks = 60_000;
    assert_eq!(pool.validate(), Ok(()), "jeda tepat di ceiling");
    pool.jeda_maks = 60_001;
    assert_eq!(field_errors(&pool), ["jeda_maks"]);

    // The case named in AC 3: 36_000 requests per hour leave exactly 100 ms
    // each. `rate_limit_per_jam` has no maximum, so 36_000 is a valid input and
    // the only thing under test is the 100 ms jeda ceiling it derives.
    let mut pool = pool_valid();
    pool.rate_limit_per_jam = 36_000;
    assert_eq!(
        pool.batas_jeda_dari_rate_limit(),
        Some(100),
        "3_600_000 / 36_000 = 100"
    );
    pool.jeda_min = 100;
    pool.jeda_maks = 100;
    assert_eq!(pool.validate(), Ok(()), "jeda = 100 ms tepat di ceiling");
    pool.jeda_min = 0;
    pool.jeda_maks = 101;
    assert_eq!(field_errors(&pool), ["jeda_maks"]);
    // `jeda_min` above the ceiling while `jeda_maks` sits below it is also
    // `jeda_maks < jeda_min`, so the ordering rule reports `jeda_maks` too.
    pool.jeda_min = 101;
    pool.jeda_maks = 100;
    assert_eq!(field_errors(&pool), ["jeda_maks", "jeda_min"]);
    // Both above the ceiling: both reported, so the client fixes both at once.
    pool.jeda_min = 200;
    pool.jeda_maks = 800;
    assert_eq!(field_errors(&pool), ["jeda_min", "jeda_maks"]);

    // One request per hour above that: the ceiling is still derived from the
    // submitted value (3_600_000 / 36_001 = 99).
    pool.rate_limit_per_jam = 36_001;
    assert_eq!(pool.batas_jeda_dari_rate_limit(), Some(99));
    pool.jeda_min = 0;
    pool.jeda_maks = 100;
    assert_eq!(field_errors(&pool), ["jeda_maks"]);
}

/// AC 3: cross-field rules. `jeda_maks` never below `jeda_min`, and
/// `retry_backoff_cap_detik` never below `retry_backoff_base_detik`.
#[test]
fn validate_pool_batas_silang() {
    // jeda_maks < jeda_min.
    let mut pool = pool_valid();
    pool.jeda_min = 800;
    pool.jeda_maks = 799;
    assert_eq!(field_errors(&pool), ["jeda_maks"]);
    pool.jeda_maks = 800;
    assert_eq!(pool.validate(), Ok(()), "jeda_maks = jeda_min");

    // retry_backoff_cap_detik < retry_backoff_base_detik.
    let mut pool = pool_valid();
    pool.retry_backoff_base_detik = 10;
    pool.retry_backoff_cap_detik = 9;
    assert_eq!(field_errors(&pool), ["retry_backoff_cap_detik"]);
    pool.retry_backoff_cap_detik = 10;
    assert_eq!(pool.validate(), Ok(()), "cap = base");

    // Several independent violations come back together, in field order, one
    // error per field.
    let mut pool = pool_valid();
    pool.kuota_harian = 0;
    pool.retry_backoff_cap_detik = 400;
    pool.retry_backoff_base_detik = 500;
    pool.jeda_min = 900;
    assert_eq!(
        field_errors(&pool),
        [
            "kuota_harian",
            "retry_backoff_base_detik",
            "retry_backoff_cap_detik",
            "jeda_maks"
        ]
    );
}

/// SHAA-280: "required with no default" is enforced two ways, and both are
/// pinned here. The schema lists the field in `required` (utoipa derives that
/// from the plain, non-`Option` Rust types) and carries no `default`, so a body
/// that omits it is invalid *and* the server never fills it in.
#[test]
fn e2e_upstream_pool_wajib_tanpa_default() {
    let doc = admin_doc();
    let skema = &doc["components"]["schemas"]["UpstreamPoolConfig"];
    let wajib: Vec<&str> = skema["required"]
        .as_array()
        .expect("required UpstreamPoolConfig")
        .iter()
        .map(|v| v.as_str().expect("nama field"))
        .collect();
    for nama in TANPA_DEFAULT {
        // `alasan` is optional context, not a required field.
        if *nama == "alasan" {
            continue;
        }
        assert!(wajib.contains(nama), "field {nama} tidak wajib: {wajib:?}");
    }
    // The other KP-01 parameters keep their default *and* stay in `required`:
    // the client must send them, and the schema also shows the value the ADR
    // recommends. `TANPA_DEFAULT` above covers the inversion rule.
    for baris in KP01.iter().filter(|b| b.default.is_none()) {
        let nama = baris.nama;
        assert!(
            skema["properties"][nama].get("default").is_none(),
            "field wajib {nama} tidak boleh punya default"
        );
    }
}

/// The read entity mirrors stored values, so it must not advertise a default
/// (SHAA-280 scopes KP-01 ranges/defaults to the write body).
#[test]
fn upstream_pool_tanpa_default() {
    let doc = admin_doc();
    let properti = doc["components"]["schemas"]["UpstreamPool"]["properties"]
        .as_object()
        .expect("properties UpstreamPool");
    let dengan_default: Vec<&str> = properti
        .iter()
        .filter(|(_, field)| field.get("default").is_some())
        .map(|(nama, _)| nama.as_str())
        .collect();
    assert!(
        dengan_default.is_empty(),
        "UpstreamPool tidak boleh punya default: {dengan_default:?}"
    );
}

/// Lokasi `openapi/` hasil `make openapi`, sama seperti `masalah_path` mencari
/// `docs/`. Test ini sengaja membaca berkas hasil generate, bukan dokumen di
/// memori, karena AC-8 mengikat kontrak yang di-commit.
fn openapi_dir() -> std::path::PathBuf {
    if let Ok(root) = std::env::var("POOLROUTER_REPO_ROOT") {
        let kandidat = std::path::Path::new(&root).join("backend/openapi");
        if kandidat.is_dir() {
            return kandidat;
        }
    }
    let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let kandidat = dir.join("openapi");
        if kandidat.is_dir() {
            return kandidat;
        }
        assert!(
            dir.pop(),
            "openapi/ tidak ditemukan dari {}",
            env!("CARGO_MANIFEST_DIR")
        );
    }
}

/// AC-8 (R1 T1, GW4): setiap contoh `RequestStatus` di `downstream-api.yaml`
/// hasil generate memakai nilai `cancel_reason` dan `failure_reason` dari
/// enum-nya, dan hanya muncul pada status yang tepat.
///
/// Berkas yang dibaca adalah hasil `make openapi`, bukan dokumen di memori:
/// kontrak yang di-commit itulah yang disalin client. Contoh dengan nilai di
/// luar enum mengajari client mengirim nilai yang tidak pernah dibaca server
/// (`cancel_reason: client`, enum-nya `dibatalkan_client`).
#[test]
fn contoh_request_status_memakai_nilai_enum() {
    let path = openapi_dir().join("downstream-api.yaml");
    let isi =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("baca {}: {e}", path.display()));
    let doc: Value =
        yaml_serde::from_str(&isi).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));

    // Daftar nilai sah diambil dari dokumen itu sendiri, bukan disalin ke test,
    // supaya menambah nilai enum tidak memaksa menyunting dua tempat.
    let enum_dari = |nama: &str| -> Vec<String> {
        doc["components"]["schemas"][nama]["enum"]
            .as_array()
            .unwrap_or_else(|| panic!("enum {nama} tidak ada di dokumen"))
            .iter()
            .map(|v| v.as_str().expect("nilai enum berupa string").to_owned())
            .collect()
    };
    let cancel_reason = enum_dari("CancelReason");
    let failure_reason = enum_dari("FailureReason");
    assert!(!cancel_reason.is_empty() && !failure_reason.is_empty());

    let mut diperiksa = 0usize;
    for (lokasi, status, contoh) in contoh_request_status(&doc) {
        for (field, sah) in [
            ("cancel_reason", &cancel_reason),
            ("failure_reason", &failure_reason),
        ] {
            let Some(nilai) = contoh.get(field) else {
                continue;
            };
            let nilai = nilai
                .as_str()
                .unwrap_or_else(|| panic!("{lokasi}: {field} bukan string: {nilai:?}"));
            assert!(
                sah.iter().any(|v| v == nilai),
                "{lokasi}: {field} `{nilai}` di luar enum; pilihan: {sah:?}"
            );
            let ingin = if field == "cancel_reason" {
                "cancelled"
            } else {
                "failed"
            };
            assert_eq!(
                status, ingin,
                "{lokasi}: {field} hanya untuk status {ingin}"
            );
            diperiksa += 1;
        }
    }
    // Jaga test ini dari lolos kosong bila bentuk dokumen berubah.
    assert!(
        diperiksa >= 2,
        "hanya {diperiksa} contoh diperiksa di {}",
        path.display()
    );
}

/// Setiap contoh `RequestStatus` di dokumen (respons operasi maupun body
/// webhook), lengkap dengan status API-nya.
fn contoh_request_status(doc: &Value) -> Vec<(String, String, serde_json::Map<String, Value>)> {
    let mut hasil = Vec::new();
    let Some(paths) = doc["paths"].as_object() else {
        return hasil;
    };
    for (path, item) in paths {
        let Some(ops) = item.as_object() else {
            continue;
        };
        for (metode, op) in ops {
            if let Some(respons) = op["responses"].as_object() {
                for (kode, r) in respons {
                    let mut media: Vec<&Value> = Vec::new();
                    if let Some(m) = r["content"].as_object() {
                        media.extend(m.values());
                    }
                    if let Some(m) = resolve(doc, r)["content"].as_object() {
                        media.extend(m.values());
                    }
                    for content in media {
                        if content["schema"]["$ref"].as_str()
                            != Some("#/components/schemas/RequestStatus")
                        {
                            continue;
                        }
                        for (nama, contoh) in content["examples"]
                            .as_object()
                            .unwrap_or(&serde_json::Map::new())
                        {
                            push_contoh(
                                &mut hasil,
                                format!("{path} {metode} {kode} #{nama}"),
                                contoh,
                            );
                        }
                    }
                }
            }
        }
        // Body webhook `requestTerminal`, dikirim ke URL client.
        let body = &item["post"]["requestBody"]["content"]["application/json"]["examples"];
        for (nama, contoh) in body.as_object().unwrap_or(&serde_json::Map::new()) {
            push_contoh(&mut hasil, format!("{path} webhook #{nama}"), contoh);
        }
    }
    hasil
}

fn push_contoh(
    hasil: &mut Vec<(String, String, serde_json::Map<String, Value>)>,
    lokasi: String,
    contoh: &Value,
) {
    let Some(isi) = contoh["value"].as_object() else {
        return;
    };
    let status = isi["status"].as_str().unwrap_or_default().to_owned();
    hasil.push((lokasi, status, isi.clone()));
}
