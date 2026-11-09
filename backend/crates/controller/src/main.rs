//! Controller process entry point (ARC-69, R2 T2 of `f0-controller-worker`).
//!
//! Scope of this ticket is the *frame*: leader election, the leader-gated
//! scheduler, the Redis rebuild from PostgreSQL and the `upstream_attempt_daily`
//! snapshot. Dispatcher (ARC-70) berjalan di task sendiri selama leader lock
//! yang sama dipegang. Penyapu (ARC-72) masuk penjadwal sebagai
//! [`poolrouter_controller::jadwal::Pekerjaan`]; reconciler menyusul.
//!
//! Steps, and why they are in this order:
//!
//! 1. Read the configuration. The controller also gets the `pr_webhook` role
//!    (ADR-0001 section 7.2), which only later tickets use.
//! 2. Build the PostgreSQL pool and the Redis client. Both are lazy so the
//!    process starts and reports the outage instead of crash-looping.
//! 3. Wait for the advisory lock. A standby sits here doing nothing, which is
//!    the intended state: AC-4 only asks that it take over within 30 s.
//! 4. Run the scheduler until SIGINT/SIGTERM.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use redis::IntoConnectionInfo;
use secrecy::ExposeSecret;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use tokio_util::sync::CancellationToken;

use poolrouter_controller::balikan::Balikan;
use poolrouter_controller::dispatcher::jalankan_loop_dispatcher;
use poolrouter_controller::jadwal::{INTERVAL_SIKLUS, Pekerjaan, SumberLeader, jalankan_loop};
use poolrouter_controller::leader::KotakFutur;
use poolrouter_controller::leader::{Leader, PemegangLeader};
use poolrouter_controller::penyapu::Penyapu;
use poolrouter_controller::rebuild::Rebuild;
use poolrouter_controller::snapshot::{Snapshot, SumberSnapshotPg};
use poolrouter_controller::webhook::Webhook;
use poolrouter_core::config::{Component, Config, DbRole};
use poolrouter_core::kuota::sumber_pg::SumberPostgres;
use poolrouter_core::queue::{JobPublisher, QueueError};
use poolrouter_core::secret::Brankas;
use poolrouter_queue_apalis::ApalisQueue;
use uuid::Uuid;

/// How long one acquire may wait before `/readyz` sees the database as down.
const TIMEOUT_AKUISISI: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("healthcheck") {
        return poolrouter_core::runtime::healthcheck().await;
    }
    if args.iter().any(|a| a == "--check") {
        return poolrouter_core::runtime::start("controller", Component::Controller, &args).await;
    }
    poolrouter_core::runtime::init_tracing();

    let cfg = match Config::from_env(Component::Controller) {
        Ok(cfg) => cfg,
        Err(err) => {
            tracing::error!(component = "controller", error = %err, "configuration rejected");
            return std::process::ExitCode::from(2);
        }
    };

    let readiness = match poolrouter_core::runtime::DependencyReadiness::from_config(
        Component::Controller,
        &cfg,
    ) {
        Ok(readiness) => readiness,
        Err(err) => {
            tracing::error!(component = "controller", error = %err, "readiness setup failed");
            return std::process::ExitCode::from(2);
        }
    };
    poolrouter_core::runtime::spawn_ready_server(std::sync::Arc::new(readiness));

    match jalankan(&cfg).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(component = "controller", error = %err, "controller stopped");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn jalankan(cfg: &Config) -> Result<(), String> {
    let pool = pg_pool(cfg)?;
    let redis = redis_client(cfg)?;

    let berhenti = CancellationToken::new();
    let token_sinyal = berhenti.clone();
    tokio::spawn(async move {
        poolrouter_core::runtime::cancel_on_signal(token_sinyal).await;
    });

    tracing::info!("controller menunggu leader lock");
    let Some(pemegang) = PemegangLeader::tunggu_jadi_leader(&pool, &berhenti).await else {
        return Ok(());
    };
    tracing::info!("controller menjadi leader");

    let sumber_rebuild = SumberPostgres::baru(pool.clone());
    // The webhook job has its own role (ADR-0001 §7.2) and opens the secret
    // envelope only when it is about to sign (AC-30).
    let kek = cfg
        .secrets
        .get("POOLROUTER_KEK")
        .ok_or("POOLROUTER_KEK_FILE missing")?;
    let brankas = Brankas::dari_isi_kek(kek.expose_secret()).map_err(|e| e.to_string())?;
    let antrean = ApalisQueue::connect(&rabbitmq_uri(cfg)?)
        .await
        .map_err(|e| e.to_string())?;
    let publisher = Arc::new(PublisherDeklarasi {
        antrean,
        sudah: tokio::sync::Mutex::new(HashSet::new()),
    });
    let pekerjaan: Vec<Box<dyn Pekerjaan>> = vec![
        Box::new(Rebuild::baru(redis.clone(), sumber_rebuild)),
        Box::new(Snapshot::baru(SumberSnapshotPg::baru(pool.clone()))),
        Box::new(Penyapu::baru(pool.clone(), redis.clone())),
        Box::new(Webhook::baru(pg_pool_webhook(cfg)?, brankas)),
        Box::new(Balikan::baru(pool.clone(), publisher.clone())),
    ];

    // The first leader was acquired above; the source hands it over once and
    // re-elects afterwards, when a cycle reports the lock lost.
    let sumber = SumberLeaderPg {
        pool,
        redis,
        publisher,
        berhenti: berhenti.clone(),
        awal: tokio::sync::Mutex::new(Some(pemegang)),
    };

    jalankan_loop(sumber, pekerjaan, INTERVAL_SIKLUS, berhenti, Utc::now).await;

    Ok(())
}

/// [`SumberLeader`] backed by the PostgreSQL advisory lock.
///
/// Setiap kepemimpinan baru juga menyalakan loop dispatcher (ARC-70) yang
/// berbagi lock yang sama; loop itu berhenti sendiri saat lock hilang.
struct SumberLeaderPg {
    pool: sqlx::PgPool,
    redis: redis::Client,
    publisher: Arc<PublisherDeklarasi>,
    berhenti: CancellationToken,
    /// The leader already held when the process started.
    awal: tokio::sync::Mutex<Option<PemegangLeader>>,
}

impl SumberLeaderPg {
    fn mulai(&self, pemegang: PemegangLeader) -> Box<dyn Leader> {
        let bersama: Arc<dyn Leader> = Arc::new(pemegang);
        tokio::spawn(jalankan_loop_dispatcher(
            Arc::clone(&bersama),
            self.pool.clone(),
            self.redis.clone(),
            Arc::clone(&self.publisher),
            self.berhenti.clone(),
        ));
        Box::new(LeaderBersama(bersama))
    }
}

impl SumberLeader for SumberLeaderPg {
    fn ambil(&self) -> KotakFutur<'_, Option<Box<dyn Leader>>> {
        Box::pin(async move {
            let awal = self.awal.lock().await.take();
            let pemegang = match awal {
                Some(p) => p,
                None => PemegangLeader::tunggu_jadi_leader(&self.pool, &self.berhenti).await?,
            };
            Some(self.mulai(pemegang))
        })
    }
}

/// Leader yang dipegang bersama penjadwal dan loop dispatcher.
struct LeaderBersama(Arc<dyn Leader>);

impl Leader for LeaderBersama {
    fn masih_leader(&self) -> KotakFutur<'_, bool> {
        self.0.masih_leader()
    }
}

/// Publisher yang mendeklarasikan queue (idempoten) sebelum publish pertama
/// ke nama itu: queue per upstream pool baru diketahui saat dispatch, dan
/// publish `mandatory` ke queue yang belum ada ditolak broker.
struct PublisherDeklarasi {
    antrean: ApalisQueue,
    sudah: tokio::sync::Mutex<HashSet<String>>,
}

impl JobPublisher for PublisherDeklarasi {
    async fn publish(&self, queue: &str, request_id: Uuid) -> Result<(), QueueError> {
        {
            let mut sudah = self.sudah.lock().await;
            if !sudah.contains(queue) {
                self.antrean.declare(queue).await?;
                sudah.insert(queue.to_owned());
            }
        }
        self.antrean.publish(queue, request_id).await
    }
}

fn pg_pool_webhook(cfg: &Config) -> Result<sqlx::PgPool, String> {
    pg_pool_role(cfg, DbRole::Webhook)
}

fn pg_pool(cfg: &Config) -> Result<sqlx::PgPool, String> {
    pg_pool_role(cfg, DbRole::Controller)
}

fn pg_pool_role(cfg: &Config, role: DbRole) -> Result<sqlx::PgPool, String> {
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
        .max_connections(5)
        .acquire_timeout(TIMEOUT_AKUISISI)
        .connect_lazy_with(opts))
}

fn ssl_mode(raw: &str) -> Result<PgSslMode, String> {
    raw.parse()
        .map_err(|_| "POOLROUTER_DB_SSLMODE is not a PostgreSQL ssl mode".to_owned())
}

/// AMQP URI for [`ApalisQueue::connect`] (ADR-0001 §6, ADR-0004 §4.3). The
/// password is never logged: it only exists inside the returned `String`.
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
