//! Entry point proses worker (ARC-68).
//!
//! Urutan, dan alasannya:
//!
//! 1. Konfigurasi dan probe readiness, sama seperti komponen lain.
//! 2. Pool PostgreSQL peran `pr_worker` (ADR-0001 §7.2), store kuota Redis
//!    lewat `ConnectionManager` (tersambung ulang sendiri setelah Redis
//!    restart), brankas secret dari KEK, dan klien HTTP provider.
//! 3. Satu consumer per upstream pool (`nama_antrean_upstream`, ADR-0001 §6)
//!    yang menjalankan handler ARC-71 lewat [`konsumen::jalankan`]. Daftar
//!    pool dibaca ulang berkala sehingga pool baru ikut dikonsumsi tanpa
//!    restart.
//! 4. SIGINT/SIGTERM: consumer berhenti mengambil pesan, handler yang sedang
//!    berjalan diselesaikan dan di-ack, lalu proses keluar.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use poolrouter_core::config::{Component, Config, DbRole};
use poolrouter_core::kuota::metrik::Komponen;
use poolrouter_core::kuota::store::KuotaRedis;
use poolrouter_core::queue::nama_antrean_upstream;
use poolrouter_core::retry::AcakAwsLc;
use poolrouter_core::runtime::{self, DependencyReadiness};
use poolrouter_core::secret::Brankas;
use poolrouter_queue_apalis::{ApalisQueue, PREFETCH_BAWAAN};
use poolrouter_worker::handler::Handler;
use poolrouter_worker::konsumen;
use poolrouter_worker::upstream::PemanggilReqwest;
use redis::IntoConnectionInfo;
use secrecy::ExposeSecret;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Batas tunggu satu akuisisi koneksi PostgreSQL.
const TIMEOUT_AKUISISI: Duration = Duration::from_secs(5);

/// Interval baca ulang daftar upstream pool.
const INTERVAL_DAFTAR_POOL: Duration = Duration::from_secs(60);

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

    match jalankan(&cfg).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(component = "worker", error = %err, "worker stopped");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn jalankan(cfg: &Config) -> Result<(), String> {
    let berhenti = CancellationToken::new();
    let token_sinyal = berhenti.clone();
    tokio::spawn(async move {
        runtime::cancel_on_signal(token_sinyal).await;
    });

    let pool = pg_pool(cfg)?;
    let redis = redis_client(cfg)?;
    let koneksi_redis = redis::aio::ConnectionManager::new(redis)
        .await
        .map_err(|e| format!("redis: {e}"))?;
    let kuota = KuotaRedis::baru(koneksi_redis)
        .await
        .map_err(|e| format!("redis: {e}"))?
        .dengan_komponen(Komponen::Worker);
    let kek = cfg
        .secrets
        .get("POOLROUTER_KEK")
        .ok_or("POOLROUTER_KEK_FILE missing")?;
    let brankas = Brankas::dari_isi_kek(kek.expose_secret()).map_err(|e| e.to_string())?;
    let handler = Arc::new(Handler::baru(
        pool.clone(),
        kuota,
        PemanggilReqwest::baru()?,
        brankas,
        Box::new(AcakAwsLc::baru()),
        worker_id(),
    ));
    let antrean = Arc::new(
        ApalisQueue::connect(&rabbitmq_uri(cfg)?)
            .await
            .map_err(|e| e.to_string())?
            .dengan_penghenti(berhenti.clone())
            .dengan_prefetch(prefetch()),
    );

    let mut sudah: HashSet<Uuid> = HashSet::new();
    let mut consumer = tokio::task::JoinSet::new();
    loop {
        match daftar_pool(&pool).await {
            Ok(daftar) => {
                for id in daftar.into_iter().filter(|id| sudah.insert(*id)) {
                    let antrean = Arc::clone(&antrean);
                    let handler = Arc::clone(&handler);
                    consumer.spawn(async move {
                        let nama = nama_antrean_upstream(id);
                        tracing::info!(antrean = %nama, "worker: consumer mulai");
                        if let Err(galat) = konsumen::jalankan(&*antrean, &nama, handler).await {
                            tracing::error!(antrean = %nama, error = %galat, "worker: consumer berhenti");
                        }
                    });
                }
            }
            Err(galat) => {
                tracing::warn!(error = %galat, "worker: daftar upstream pool gagal dibaca")
            }
        }
        tokio::select! {
            _ = berhenti.cancelled() => break,
            _ = tokio::time::sleep(INTERVAL_DAFTAR_POOL) => {}
        }
    }
    while consumer.join_next().await.is_some() {}
    tracing::info!("worker berhenti");
    Ok(())
}

/// Semua upstream pool, aktif atau tidak: pesan untuk pool nonaktif tetap
/// dikonsumsi supaya gerbang dispatch (§3a.4) menutupnya.
async fn daftar_pool(pool: &sqlx::PgPool) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT upstream_pool_id FROM upstream_pool")
        .fetch_all(pool)
        .await
}

/// `transaction_log.worker_id`: hostname container bila ada.
fn worker_id() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| format!("worker-{}", Uuid::now_v7()))
}

/// `POOLROUTER_WORKER_PREFETCH`, bawaan [`PREFETCH_BAWAAN`].
fn prefetch() -> u16 {
    std::env::var("POOLROUTER_WORKER_PREFETCH")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|p| *p > 0)
        .unwrap_or(PREFETCH_BAWAAN)
}

fn pg_pool(cfg: &Config) -> Result<sqlx::PgPool, String> {
    let role = DbRole::Worker;
    let password = cfg
        .postgres
        .password(role)
        .ok_or_else(|| format!("missing password for the {} role", role.role_name()))?;
    let opts = PgConnectOptions::new()
        .host(&cfg.postgres.host)
        .port(cfg.postgres.port)
        .database(&cfg.postgres.name)
        .username(role.role_name())
        .password(password)
        .ssl_mode(ssl_mode(&cfg.postgres.sslmode)?);
    Ok(PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(TIMEOUT_AKUISISI)
        .connect_lazy_with(opts))
}

fn ssl_mode(raw: &str) -> Result<PgSslMode, String> {
    raw.parse()
        .map_err(|_| "POOLROUTER_DB_SSLMODE is not a PostgreSQL ssl mode".to_owned())
}

/// URI AMQP. Password hanya ada di `String` hasil, tidak pernah di-log.
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

fn redis_client(cfg: &Config) -> Result<redis::Client, String> {
    let r = cfg
        .redis
        .as_ref()
        .ok_or_else(|| "redis configuration missing".to_owned())?;
    let info = redis::ConnectionAddr::Tcp(r.host.clone(), r.port)
        .into_connection_info()
        .map_err(|_| "redis configuration is not valid".to_owned())?
        .set_redis_settings(
            redis::RedisConnectionInfo::default().set_password(r.password.expose_secret()),
        );
    redis::Client::open(info).map_err(|_| "redis configuration is not valid".to_owned())
}
