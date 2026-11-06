//! Reusable K-20 request and database-function harness.
//!
//! The ignored tests use the same `pr_admin_api` connection as the real
//! service. Fixtures are loaded by `make -C backend uji-pg`; no production
//! credentials or identifiers belong in this file.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use poolrouter_webserver::admin::k20::{JenisEntitas, K20Service, validasi_alasan};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

/// Sends a request while keeping status and body assertions at the call site.
pub async fn kirim(app: Router, method: &str, path: &str, body: Option<&str>) -> Response {
    let mut request = Request::builder().method(method).uri(path);
    if body.is_some() {
        request = request.header("content-type", "application/json");
    }
    app.oneshot(
        request
            .body(Body::from(body.unwrap_or_default().to_owned()))
            .unwrap(),
    )
    .await
    .unwrap()
}

#[derive(Clone, Default)]
struct Penyangga(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl<'a> MakeWriter<'a> for Penyangga {
    type Writer = Penulis;

    fn make_writer(&'a self) -> Self::Writer {
        Penulis(self.0.clone())
    }
}

struct Penulis(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Penulis {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Captures one tracing subscriber scope for redaction assertions.
pub fn penangkap_log() -> (Penyangga, tracing::subscriber::DefaultGuard) {
    let writer = Penyangga::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(writer.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    (writer, guard)
}

pub fn isi_log(writer: &Penyangga) -> String {
    String::from_utf8(writer.0.lock().unwrap().clone()).unwrap()
}

#[tokio::test]
#[ignore = "membutuhkan PostgreSQL dan router K-20 dari tiket pecahan 2–3"]
async fn helper_mengembalikan_status_http() {
    let app = Router::new().fallback(|| async { StatusCode::NO_CONTENT });
    assert_eq!(
        kirim(app, "GET", "/admin/v1/k20", None).await.status(),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
#[ignore = "jalankan dengan make -C backend uji-pg dan fixture K-20"]
async fn service_menggunakan_fungsi_k20_dan_menulis_audit_tanpa_data_sensitif() {
    let url = std::env::var("POOLROUTER_TEST_DATABASE_URL").expect("uji-pg belum disiapkan");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let service = K20Service::baru(pool.clone());
    let id = Uuid::parse_str("10000000-0000-7000-8000-000000000011").unwrap();
    let operator = Uuid::parse_str("00000000-0000-7000-8000-000000000001").unwrap();
    let nik_sintetis = "0000000000000000";
    let payload_sintetis = "payload-sintetis-k20";
    let (log_writer, _log_guard) = penangkap_log();

    let preview = service.pratinjau(JenisEntitas::Client, id).await.unwrap();
    assert!(preview.akan_cancelled >= 0 && preview.akan_failed >= 0);

    let synthetic_reason = "uji K20 sintetis";
    assert!(validasi_alasan(synthetic_reason));
    let hasil = service
        .nonaktifkan(JenisEntitas::Client, id, operator, synthetic_reason)
        .await;
    assert!(hasil.is_ok());
    let audit: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT alasan, nilai_baru::text FROM audit_konfigurasi WHERE entitas_id = $1 ORDER BY terjadi_at DESC LIMIT 1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(audit.0.as_deref() == Some(synthetic_reason));
    assert!(audit.1.as_deref().unwrap_or_default().contains("aktif"));
    assert!(!audit.1.as_deref().unwrap_or_default().contains("secret"));
    let log = isi_log(&log_writer);
    assert!(!log.contains(nik_sintetis));
    assert!(!log.contains(payload_sintetis));
}
