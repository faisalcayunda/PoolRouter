//! Harness PG untuk kontrak K-20. Dijalankan hanya dengan database uji.
//! Fixture tidak memakai data produksi dan seluruh test di-ignore secara default.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use tower::ServiceExt;

/// Helper request bersama tiket pecahan 2–3: status dan body dapat diperiksa
/// tanpa menyentuh jaringan.
pub async fn kirim(app: axum::Router, method: &str, path: &str, body: Option<&str>) -> Response {
    let mut request = Request::builder().method(method).uri(path);
    if body.is_some() {
        request = request.header("content-type", "application/json");
    }
    app.oneshot(request.body(Body::from(body.unwrap_or_default().to_owned())).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "membutuhkan PostgreSQL dan router K-20 dari tiket pecahan 2–3"]
async fn helper_mengembalikan_status_http() {
    let app = axum::Router::new().fallback(|| async { StatusCode::NO_CONTENT });
    assert_eq!(kirim(app, "GET", "/admin/v1/k20", None).await.status(), StatusCode::NO_CONTENT);
}
