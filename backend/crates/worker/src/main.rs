//! Worker process entry point. The job loop is a later ticket; this ticket
//! owns the frame: configuration, readiness probes and graceful shutdown.

use std::sync::Arc;

use poolrouter_core::config::{Component, Config};
use poolrouter_core::runtime::{self, DependencyReadiness};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("healthcheck") {
        return runtime::healthcheck().await;
    }
    if args.iter().any(|a| a == "--check") {
        return runtime::start("worker", Component::Worker, &args).await;
    }

    runtime::init_tracing();
    let cfg = match Config::from_env(Component::Worker) {
        Ok(cfg) => cfg,
        Err(err) => {
            tracing::error!(component = "worker", error = %err, "configuration rejected");
            return std::process::ExitCode::from(2);
        }
    };
    let readiness = match DependencyReadiness::from_config(Component::Worker, &cfg) {
        Ok(readiness) => readiness,
        Err(err) => {
            tracing::error!(component = "worker", error = %err, "readiness setup failed");
            return std::process::ExitCode::from(2);
        }
    };
    runtime::spawn_ready_server(Arc::new(readiness));

    runtime::run_skeleton("worker", &args).await;
    std::process::ExitCode::SUCCESS
}
