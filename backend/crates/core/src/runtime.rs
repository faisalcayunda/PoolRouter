//! Process bootstrap shared by every binary: logging and graceful shutdown.

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
    if args.iter().any(|a| a == "healthcheck") {
        return healthcheck().await;
    }
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

/// Probe the component's local admin readiness endpoint.  This deliberately
/// runs before normal configuration loading: compose uses it to decide whether
/// the process is ready, and the endpoint itself performs dependency checks.
async fn healthcheck() -> std::process::ExitCode {
    let address =
        std::env::var("POOLROUTER_ADMIN_ADDR").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("POOLROUTER_ADMIN_PORT").unwrap_or_else(|_| "9090".to_string());
    healthcheck_at(&format!("{address}:{port}")).await
}

async fn healthcheck_at(address: &str) -> std::process::ExitCode {
    let url = format!("http://{address}/readyz");
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(_) => return std::process::ExitCode::from(1),
    };
    match client.get(url).send().await {
        Ok(response) if response.status().is_success() => std::process::ExitCode::SUCCESS,
        _ => std::process::ExitCode::from(1),
    }
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
    async fn healthcheck_uses_bounded_probe() {
        // A closed local port must fail promptly rather than hang.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            healthcheck_at("127.0.0.1:1"),
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), std::process::ExitCode::from(1));
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
