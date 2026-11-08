//! Worker process entry point. Konsumen untuk `pr.jobs.balikan` (ARC-198).

mod consumer;

use std::sync::Arc;

use secrecy::ExposeSecret;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use std::time::Duration;

use poolrouter_core::config::{Component, Config, DbRole};
use poolrouter_core::runtime::{self, DependencyReadiness};
use poolrouter_queue_apalis::ApalisQueue;
use consumer::proses_balikan;

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

    if let Err(e) = jalankan(&cfg).await {
        tracing::error!(component = "worker", error = %e, "worker stopped");
        return std::process::ExitCode::FAILURE;
    }

    runtime::run_skeleton("worker", &args).await;
    std::process::ExitCode::SUCCESS
}

async fn jalankan(cfg: &Config) -> Result<(), String> {
    let pool = pg_pool(cfg)?;
    let pengirim = poolrouter_adapter_provider::PengirimHttp::baru()?;
    let antrean = ApalisQueue::connect(&rabbitmq_uri(cfg)?)
        .await
        .map_err(|e| e.to_string())?;
    antrean
        .declare("pr.jobs.balikan")
        .await
        .map_err(|e| e.to_string())?;

    antrean
        .run("pr.jobs.balikan", |provider_id_bytes| async move {
            let provider_id_str = String::from_utf8(provider_id_bytes.to_vec())
                .map_err(|e| poolrouter_core::queue::QueueError::Handler(e.to_string()))?;
            proses_balikan(&pool, &provider_id_str, &pengirim).await
        })
        .await
}

fn pg_pool(cfg: &Config) -> Result<sqlx::PgPool, String> {
    let password = cfg
        .postgres
        .password(DbRole::Worker)
        .ok_or("missing password for the worker role")?;
    let opts = PgConnectOptions::new()
        .host(&cfg.postgres.host)
        .port(cfg.postgres.port)
        .database(&cfg.postgres.name)
        .username("pr_worker")
        .password(password)
        .ssl_mode(ssl_mode(&cfg.postgres.sslmode)?);
    Ok(PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy_with(opts))
}

fn ssl_mode(raw: &str) -> Result<sqlx::postgres::PgSslMode, String> {
    raw.parse()
        .map_err(|_| "POOLROUTER_DB_SSLMODE is not a PostgreSQL ssl mode".to_owned())
}

fn rabbitmq_uri(cfg: &Config) -> Result<String, String> {
    let r = cfg
        .rabbitmq
        .as_ref()
        .ok_or_else(|| "rabbitmq configuration missing".to_owned())?;
    Ok(format!(
        "amqp://{}:{}@{}:{}/{}",
        r.user,
        r.password.expose_secret(),
        r.host,
        r.port,
        r.vhost
    ))
}
