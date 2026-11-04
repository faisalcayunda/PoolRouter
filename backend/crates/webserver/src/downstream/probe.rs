//! Admin-port surface: `/healthz`, `/readyz`, `/metrics` (ADR-0001 5a.9, R-13),
//! and the `webserver healthcheck` subcommand client.
//!
//! The image has no shell, `curl` or `nc`, so the subcommand speaks plain
//! HTTP/1.1 over a `TcpStream` itself.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use metrics_exporter_prometheus::PrometheusHandle;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Default admin listen address. Loopback only: `/metrics` is not for clients.
pub const DEFAULT_ADMIN_ADDR: &str = "127.0.0.1:9090";
/// Variable that overrides [`DEFAULT_ADMIN_ADDR`].
pub const ADMIN_ADDR_VAR: &str = "POOLROUTER_ADMIN_ADDR";

/// Whole-check budget for `/readyz` dependency checks.
const READY_BUDGET: Duration = Duration::from_secs(2);
/// `webserver healthcheck` total budget; must stay under 3 s (SHAA-198 AC).
pub const HEALTHCHECK_TIMEOUT: Duration = Duration::from_millis(2500);

type BoxFut<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// Dependency readiness: PostgreSQL and Redis (`pr:siap` matches). The error
/// text is logged, never returned to the caller.
pub trait Readiness: Send + Sync + 'static {
    fn check(&self) -> BoxFut<'_>;
}

#[derive(Clone)]
struct AdminState {
    ready: Arc<dyn Readiness>,
    metrics: PrometheusHandle,
}

/// Router for the admin port.
pub fn admin_router(ready: Arc<dyn Readiness>, metrics: PrometheusHandle) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(render_metrics))
        .with_state(AdminState { ready, metrics })
}

/// Liveness: the process is up. No dependency is touched, so a PostgreSQL or
/// Redis outage cannot trigger cascading restarts.
async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(State(s): State<AdminState>) -> Response {
    match tokio::time::timeout(READY_BUDGET, s.ready.check()).await {
        Ok(Ok(())) => (StatusCode::OK, "ok").into_response(),
        Ok(Err(reason)) => {
            tracing::warn!(reason = %reason, "readiness failed");
            (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response()
        }
        Err(_) => {
            tracing::warn!("readiness check timed out");
            (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response()
        }
    }
}

async fn render_metrics(State(s): State<AdminState>) -> Response {
    let mut res = s.metrics.render().into_response();
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    res
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum HealthcheckError {
    #[error("admin address is not valid")]
    BadAddress,
    #[error("cannot reach the admin port")]
    Unreachable,
    #[error("timed out")]
    Timeout,
    #[error("readiness answered with status {0}")]
    NotReady(u16),
    #[error("unreadable response")]
    BadResponse,
}

/// Admin address from the environment, or the default.
pub fn admin_addr_from_env() -> Result<SocketAddr, HealthcheckError> {
    let raw = std::env::var(ADMIN_ADDR_VAR).unwrap_or_else(|_| DEFAULT_ADMIN_ADDR.to_owned());
    raw.parse().map_err(|_| HealthcheckError::BadAddress)
}

/// GET `/readyz` on `addr`: `Ok` only for status 200, within `timeout`.
/// An unspecified bind address (`0.0.0.0`) is probed through loopback.
pub async fn healthcheck(addr: SocketAddr, timeout: Duration) -> Result<(), HealthcheckError> {
    let mut target = addr;
    if target.ip().is_unspecified() {
        target.set_ip(if target.is_ipv4() {
            std::net::Ipv4Addr::LOCALHOST.into()
        } else {
            std::net::Ipv6Addr::LOCALHOST.into()
        });
    }
    tokio::time::timeout(timeout, probe(target))
        .await
        .map_err(|_| HealthcheckError::Timeout)?
}

async fn probe(addr: SocketAddr) -> Result<(), HealthcheckError> {
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|_| HealthcheckError::Unreachable)?;
    stream
        .write_all(b"GET /readyz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .map_err(|_| HealthcheckError::Unreachable)?;
    // Only the status line matters; 64 bytes hold "HTTP/1.1 200 ...".
    let mut buf = [0u8; 64];
    let mut got = 0;
    while got < buf.len() && !buf[..got].contains(&b'\n') {
        match stream.read(&mut buf[got..]).await {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(_) => return Err(HealthcheckError::Unreachable),
        }
    }
    let line = std::str::from_utf8(&buf[..got]).map_err(|_| HealthcheckError::BadResponse)?;
    let mut parts = line.split_ascii_whitespace();
    let (Some(proto), Some(code)) = (parts.next(), parts.next()) else {
        return Err(HealthcheckError::BadResponse);
    };
    if !proto.starts_with("HTTP/1.") {
        return Err(HealthcheckError::BadResponse);
    }
    let code: u16 = code.parse().map_err(|_| HealthcheckError::BadResponse)?;
    if code == 200 {
        Ok(())
    } else {
        Err(HealthcheckError::NotReady(code))
    }
}

/// Entry for `webserver healthcheck`: exit 0 when ready, 1 otherwise.
pub async fn healthcheck_exit_code() -> std::process::ExitCode {
    let result = match admin_addr_from_env() {
        Ok(addr) => healthcheck(addr, HEALTHCHECK_TIMEOUT).await,
        Err(e) => Err(e),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("healthcheck failed: {e}");
            std::process::ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use metrics_exporter_prometheus::PrometheusBuilder;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::net::TcpListener;
    use tower::ServiceExt;

    struct Flag(AtomicBool);
    impl Readiness for Flag {
        fn check(&self) -> BoxFut<'_> {
            let ok = self.0.load(Ordering::SeqCst);
            Box::pin(async move { if ok { Ok(()) } else { Err("redis down".into()) } })
        }
    }

    struct Hang;
    impl Readiness for Hang {
        fn check(&self) -> BoxFut<'_> {
            Box::pin(std::future::pending())
        }
    }

    fn handle() -> PrometheusHandle {
        PrometheusBuilder::new().build_recorder().handle()
    }

    async fn get_status(router: &Router, path: &str) -> StatusCode {
        router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn healthz_ignores_dependencies_but_readyz_does_not() {
        let flag = Arc::new(Flag(AtomicBool::new(false)));
        let r = admin_router(flag.clone(), handle());
        assert_eq!(get_status(&r, "/healthz").await, 200);
        assert_eq!(get_status(&r, "/readyz").await, 503);
        flag.0.store(true, Ordering::SeqCst);
        assert_eq!(get_status(&r, "/readyz").await, 200);
    }

    #[tokio::test]
    async fn readyz_times_out_as_not_ready() {
        let r = admin_router(Arc::new(Hang), handle());
        assert_eq!(get_status(&r, "/readyz").await, 503);
    }

    #[tokio::test]
    async fn readyz_body_does_not_leak_the_reason() {
        let r = admin_router(Arc::new(Flag(AtomicBool::new(false))), handle());
        let res = r
            .oneshot(
                axum::http::Request::builder()
                    .uri("/readyz")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(res.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"not ready");
    }

    #[tokio::test]
    async fn metrics_endpoint_renders_recorded_values() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let h = recorder.handle();
        metrics::with_local_recorder(&recorder, || {
            metrics::counter!("poolrouter_token_gagal_total", "alasan" => "dibatasi").increment(1);
        });
        let r = admin_router(Arc::new(Flag(AtomicBool::new(true))), h);
        let res = r
            .oneshot(
                axum::http::Request::builder()
                    .uri("/metrics")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let body = to_bytes(res.into_body(), 64 * 1024).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            text.contains("poolrouter_token_gagal_total{alasan=\"dibatasi\"} 1"),
            "{text}"
        );
    }

    /// Serve `router` on an ephemeral loopback port.
    async fn serve(router: Router) -> SocketAddr {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, router).await.unwrap() });
        addr
    }

    #[tokio::test]
    async fn healthcheck_exit_ok_when_ready() {
        let addr = serve(admin_router(
            Arc::new(Flag(AtomicBool::new(true))),
            handle(),
        ))
        .await;
        assert_eq!(healthcheck(addr, HEALTHCHECK_TIMEOUT).await, Ok(()));
    }

    #[tokio::test]
    async fn healthcheck_fails_when_readyz_fails() {
        let addr = serve(admin_router(
            Arc::new(Flag(AtomicBool::new(false))),
            handle(),
        ))
        .await;
        assert_eq!(
            healthcheck(addr, HEALTHCHECK_TIMEOUT).await,
            Err(HealthcheckError::NotReady(503))
        );
    }

    #[tokio::test]
    async fn healthcheck_fails_when_port_does_not_answer() {
        // Bind then drop to get a port nobody listens on.
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        assert_eq!(
            healthcheck(addr, HEALTHCHECK_TIMEOUT).await,
            Err(HealthcheckError::Unreachable)
        );
    }

    #[tokio::test]
    async fn healthcheck_gives_up_on_a_silent_server_under_three_seconds() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            let (_s, _) = l.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(30)).await;
        });
        let started = std::time::Instant::now();
        assert_eq!(
            healthcheck(addr, HEALTHCHECK_TIMEOUT).await,
            Err(HealthcheckError::Timeout)
        );
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[tokio::test]
    async fn healthcheck_rejects_garbage_response() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let _ = s.write_all(b"SSH-2.0-nope\r\n").await;
        });
        assert_eq!(
            healthcheck(addr, HEALTHCHECK_TIMEOUT).await,
            Err(HealthcheckError::BadResponse)
        );
    }

    #[test]
    fn default_admin_addr_parses() {
        assert!(DEFAULT_ADMIN_ADDR.parse::<SocketAddr>().is_ok());
    }
}
