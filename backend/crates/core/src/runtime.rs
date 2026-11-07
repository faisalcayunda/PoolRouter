//! Process bootstrap shared by every binary: logging and graceful shutdown.

use axum::Router;
use axum::http::{HeaderValue, header};
use axum::response::IntoResponse;
use axum::routing::get;
use metrics_exporter_prometheus::PrometheusBuilder;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

/// Initialise JSON logging. The filter comes from `RUST_LOG` (default `info`).
///
/// Request bodies, rendered URLs and sensitive headers must never be logged
/// (ADR-0001 section 7.1).
pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // A second call (for example from tests) is a no-op, not an error.
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .try_init();
}

/// Resolves when the process receives SIGINT or SIGTERM, then cancels `token`.
pub async fn cancel_on_signal(token: CancellationToken) {
    wait_for_signal().await;
    token.cancel();
}

#[cfg(unix)]
async fn wait_for_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    // Failing to register a handler is a startup fault; stop instead of
    // running a process that cannot be shut down gracefully.
    let mut term = signal(SignalKind::terminate()).expect("register SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

/// Entry point for the binaries: validate configuration (contract section 1),
/// then run. Bad configuration exits non-zero and names the variables, never
/// their values. `--check` skips the configuration load (smoke test).
pub async fn start(
    name: &'static str,
    component: crate::config::Component,
    args: &[String],
) -> std::process::ExitCode {
    if !args.iter().any(|a| a == "--check")
        && let Err(err) = crate::config::Config::from_env(component)
    {
        init_tracing();
        tracing::error!(component = name, error = %err, "configuration rejected");
        return std::process::ExitCode::from(2);
    }
    run_skeleton(name, args).await;
    std::process::ExitCode::SUCCESS
}

/// Probe the component readiness endpoint for Compose healthchecks.
pub async fn healthcheck() -> std::process::ExitCode {
    let addr = std::env::var("POOLROUTER_ADMIN_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9090".to_owned());
    healthcheck_at(&addr).await
}

/// Probe `/readyz` on `addr` (used by tests with an explicit address).
pub async fn healthcheck_at(addr: &str) -> std::process::ExitCode {
    let url = format!("http://{addr}/readyz");
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reqwest::Client::new().get(url).send(),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .is_some_and(|response| response.status().is_success());
    if result { std::process::ExitCode::SUCCESS } else { std::process::ExitCode::FAILURE }
}

/// Expose the local readiness endpoint used by the container probe.
pub fn spawn_ready_server() {
    let addr = std::env::var("POOLROUTER_ADMIN_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:9090".to_owned());
    spawn_ready_server_on(&addr);
}

/// Bind the probe/metrics listener on `addr` (used by tests with an explicit address).
pub fn spawn_ready_server_on(addr: &str) {
    let addr = addr.to_owned();
    tokio::spawn(async move {
        let Ok(listener) = TcpListener::bind(&addr).await else { return };
        let metrics = PrometheusBuilder::new().build_recorder().handle();
        let metrics_handle = metrics.clone();
        let app = Router::new()
            .route("/healthz", get(|| async { "ok" }))
            .route("/readyz", get(|| async { "ok" }))
            .route(
                "/metrics",
                get(move || {
                    let m = metrics_handle.clone();
                    async move {
                        let mut res = m.render().into_response();
                        res.headers_mut().insert(
                            header::CONTENT_TYPE,
                            HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
                        );
                        res
                    }
                }),
            );
        let _ = axum::serve(listener, app).await;
    });
}

/// Skeleton entry point used by the binaries until the real services land.
///
/// With `--check` it initialises logging and returns immediately (smoke test).
/// Otherwise it runs until SIGINT/SIGTERM.
pub async fn run_skeleton(name: &'static str, args: &[String]) {
    init_tracing();
    tracing::info!(component = name, "started");
    if args.iter().any(|a| a == "--check") {
        tracing::info!(component = name, "check ok");
        return;
    }
    let token = CancellationToken::new();
    tokio::spawn(cancel_on_signal(token.clone()));
    token.cancelled().await;
    tracing::info!(component = name, "stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::ExitCode;
    use std::time::Duration;

    #[tokio::test]
    async fn healthcheck_at_fails_without_listener() {
        let code = healthcheck_at("127.0.0.1:19191").await;
        assert_eq!(code, ExitCode::FAILURE);
    }

    #[tokio::test]
    async fn healthcheck_at_succeeds_when_readyz_serves_ok() {
        let addr = "127.0.0.1:19192";
        spawn_ready_server_on(addr);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let code = healthcheck_at(addr).await;
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[tokio::test]
    async fn check_flag_returns_immediately() {
        let args = vec!["--check".to_string()];
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_skeleton("test", &args),
        )
        .await
        .expect("--check must not block");
    }

    #[tokio::test]
    async fn without_check_it_waits_for_shutdown() {
        let res = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            run_skeleton("test", &[]),
        )
        .await;
        assert!(res.is_err(), "must keep running until a signal arrives");
    }
}
