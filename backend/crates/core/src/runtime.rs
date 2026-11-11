//! Process bootstrap shared by every binary: logging and graceful shutdown.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use redis::IntoConnectionInfo;
use secrecy::ExposeSecret;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::config::{Component, Config, RabbitmqConfig};

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
    let addr =
        std::env::var("POOLROUTER_ADMIN_ADDR").unwrap_or_else(|_| "127.0.0.1:9090".to_owned());
    healthcheck_at(&addr).await
}

/// Probe `/readyz` on `addr` (used by tests with an explicit address).
pub async fn healthcheck_at(addr: &str) -> std::process::ExitCode {
    probe_at(addr, "readyz").await
}

/// Probe `/healthz` for the container liveness check (ARC-226). Unlike
/// [`healthcheck`], a down dependency does not fail it: only a process that
/// reports itself dead does (worker consumer loops, ADR-0004 section 4.3 W3).
pub async fn liveness() -> std::process::ExitCode {
    let addr =
        std::env::var("POOLROUTER_ADMIN_ADDR").unwrap_or_else(|_| "127.0.0.1:9090".to_owned());
    liveness_at(&addr).await
}

/// Probe `/healthz` on `addr` (used by tests with an explicit address).
pub async fn liveness_at(addr: &str) -> std::process::ExitCode {
    probe_at(addr, "healthz").await
}

async fn probe_at(addr: &str, path: &str) -> std::process::ExitCode {
    let url = format!("http://{addr}/{path}");
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        reqwest::Client::new().get(url).send(),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .is_some_and(|response| response.status().is_success());
    if result {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

/// Whole-check budget for `/readyz` dependency checks.
const READY_BUDGET: Duration = Duration::from_secs(2);

pub type BoxFut<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// Dependency readiness for the admin `/readyz` endpoint. The error text is
/// logged, never returned to the caller: driver messages can carry hosts,
/// users or DSN fragments.
pub trait Readiness: Send + Sync + 'static {
    fn check(&self) -> BoxFut<'_>;
}

/// PostgreSQL, Redis and broker readiness shared by controller and worker.
pub struct DependencyReadiness {
    pg: sqlx::PgPool,
    redis: redis::Client,
    broker_uri: String,
}

impl DependencyReadiness {
    /// Build the probe clients from the component configuration. Connections
    /// are lazy, so a dead dependency shows up in `/readyz`, not at startup.
    pub fn from_config(component: Component, cfg: &Config) -> Result<Self, String> {
        let role = *component
            .db_roles()
            .first()
            .ok_or_else(|| "component has no database role".to_owned())?;
        let password = cfg
            .postgres
            .password(role)
            .ok_or_else(|| format!("missing password for the {} role", role.role_name()))?;
        let sslmode = cfg
            .postgres
            .sslmode
            .parse()
            .map_err(|_| "POOLROUTER_DB_SSLMODE is not a PostgreSQL ssl mode".to_owned())?;
        let pg = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(READY_BUDGET)
            .connect_lazy_with(
                sqlx::postgres::PgConnectOptions::new()
                    .host(&cfg.postgres.host)
                    .port(cfg.postgres.port)
                    .database(&cfg.postgres.name)
                    .username(role.role_name())
                    .password(password)
                    .ssl_mode(sslmode),
            );
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
        let redis =
            redis::Client::open(info).map_err(|_| "redis configuration is not valid".to_owned())?;
        let broker = cfg
            .rabbitmq
            .as_ref()
            .ok_or_else(|| "rabbitmq configuration missing".to_owned())?;
        let broker_uri = amqp_uri(broker);
        Ok(Self {
            pg,
            redis,
            broker_uri,
        })
    }
}

impl Readiness for DependencyReadiness {
    fn check(&self) -> BoxFut<'_> {
        Box::pin(async move {
            sqlx::query("SELECT 1")
                .execute(&self.pg)
                .await
                .map_err(|_| "postgres unavailable".to_owned())?;
            let redis = self.redis.clone();
            let ping = async move {
                let mut conn = redis.get_multiplexed_async_connection().await?;
                redis::cmd("PING").query_async::<String>(&mut conn).await?;
                Ok::<(), redis::RedisError>(())
            };
            match tokio::time::timeout(Duration::from_secs(1), ping).await {
                Ok(Ok(())) => {}
                _ => return Err("redis unavailable".to_owned()),
            }
            // Connect per check: a held connection would stay dead after a
            // broker restart and keep the container unhealthy until restarted.
            let opened = tokio::time::timeout(
                Duration::from_secs(1),
                lapin::Connection::connect(
                    &self.broker_uri,
                    lapin::ConnectionProperties::default(),
                ),
            )
            .await;
            match opened {
                Ok(Ok(connection)) => {
                    let _ = connection.close(200, "readiness".into()).await;
                    Ok(())
                }
                _ => Err("broker unavailable".to_owned()),
            }
        })
    }
}

/// URI AMQP dari konfigurasi broker, dipakai readiness, worker, dan
/// controller supaya ketiganya mem-parse kredensial yang sama. User, password,
/// dan vhost di-percent-encode: secret hasil generate sering memuat
/// `/ @ : # ? %`, dan vhost bawaan `/` harus menjadi `%2F`.
pub fn amqp_uri(broker: &RabbitmqConfig) -> String {
    format!(
        "amqp://{}:{}@{}:{}/{}",
        uri_escape(&broker.user),
        uri_escape(broker.password.expose_secret()),
        broker.host,
        broker.port,
        uri_escape(&broker.vhost),
    )
}

/// AMQP URI requires user, password and vhost percent-encoded.
pub fn uri_escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Status hidup proses untuk `/healthz`. Hanya worker yang memasangnya
/// (ADR-0004 bagian 4.3 W3); controller dan webserver tetap memakai
/// liveness statis.
pub trait Liveness: Send + Sync + 'static {
    fn alive(&self) -> bool;
}

#[derive(Clone)]
struct ProbeState {
    ready: Arc<dyn Readiness>,
    live: Option<Arc<dyn Liveness>>,
}

fn admin_addr() -> String {
    std::env::var("POOLROUTER_ADMIN_ADDR").unwrap_or_else(|_| "0.0.0.0:9090".to_owned())
}

/// Expose the local readiness endpoint used by the container probe.
pub fn spawn_ready_server(ready: Arc<dyn Readiness>) {
    spawn_ready_server_on(&admin_addr(), ready);
}

/// Bind the probe/metrics listener on `addr` (used by tests with an explicit address).
pub fn spawn_ready_server_on(addr: &str, ready: Arc<dyn Readiness>) {
    spawn_probe_server(addr, ProbeState { ready, live: None });
}

/// Seperti [`spawn_ready_server`], tetapi `/healthz` menjawab 503 selama
/// `live` melaporkan proses tidak hidup.
pub fn spawn_ready_server_with_liveness(ready: Arc<dyn Readiness>, live: Arc<dyn Liveness>) {
    spawn_ready_server_with_liveness_on(&admin_addr(), ready, live);
}

/// [`spawn_ready_server_with_liveness`] pada `addr` eksplisit (untuk uji).
pub fn spawn_ready_server_with_liveness_on(
    addr: &str,
    ready: Arc<dyn Readiness>,
    live: Arc<dyn Liveness>,
) {
    spawn_probe_server(
        addr,
        ProbeState {
            ready,
            live: Some(live),
        },
    );
}

/// Handle `/metrics` untuk seluruh proses. Recorder Prometheus dipasang
/// global satu kali supaya `metrics::counter!`/`gauge!` dari kode aplikasi
/// benar-benar tercatat; `build_recorder()` saja membuat recorder yang tidak
/// pernah menerima apa pun. Bila proses sudah punya recorder global lain,
/// `/metrics` memakai recorder kosong seperti sebelumnya.
///
/// Recorder yang dipasang lewat `install_recorder` tidak punya upkeep
/// sendiri; histogram akan tumbuh tanpa batas bila `run_upkeep` tidak
/// dipanggil berkala. Harus dipanggil dari dalam runtime tokio.
fn handle_metrik() -> PrometheusHandle {
    static HANDLE: std::sync::OnceLock<PrometheusHandle> = std::sync::OnceLock::new();
    HANDLE
        .get_or_init(|| match PrometheusBuilder::new().install_recorder() {
            Ok(handle) => {
                let upkeep = handle.clone();
                tokio::spawn(async move {
                    let mut tik = tokio::time::interval(Duration::from_secs(5));
                    loop {
                        tik.tick().await;
                        upkeep.run_upkeep();
                    }
                });
                handle
            }
            Err(galat) => {
                tracing::warn!(error = %galat, "recorder metrik global tidak bisa dipasang");
                PrometheusBuilder::new().build_recorder().handle()
            }
        })
        .clone()
}

fn spawn_probe_server(addr: &str, state: ProbeState) {
    let addr = addr.to_owned();
    // Dipasang sebelum task jalan: metrik yang dicatat segera setelah
    // `spawn_ready_server` kembali sudah tercatat.
    let metrics_handle = handle_metrik();
    tokio::spawn(async move {
        let Ok(listener) = TcpListener::bind(&addr).await else {
            return;
        };
        let app = Router::new()
            .route("/healthz", get(healthz))
            .route("/readyz", get(readyz))
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
            )
            .with_state(state);
        let _ = axum::serve(listener, app).await;
    });
}

/// Liveness: statis untuk controller dan webserver, supaya gangguan
/// dependensi tidak memicu restart berantai. Worker memasang [`Liveness`]
/// karena loop consumer yang tidak pernah tersambung hanya pulih lewat
/// restart (ADR-0004 bagian 4.3 W3).
async fn healthz(State(state): State<ProbeState>) -> Response {
    match &state.live {
        Some(live) if !live.alive() => {
            tracing::warn!("liveness failed");
            (StatusCode::SERVICE_UNAVAILABLE, "not alive").into_response()
        }
        _ => (StatusCode::OK, "ok").into_response(),
    }
}

/// Readiness: every dependency must answer.
async fn readyz(State(state): State<ProbeState>) -> Response {
    match tokio::time::timeout(READY_BUDGET, state.ready.check()).await {
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

    struct Ready;
    impl Readiness for Ready {
        fn check(&self) -> BoxFut<'_> {
            Box::pin(async { Ok(()) })
        }
    }

    struct NotReady;
    impl Readiness for NotReady {
        fn check(&self) -> BoxFut<'_> {
            Box::pin(async { Err("postgres unavailable".to_owned()) })
        }
    }

    #[tokio::test]
    async fn healthcheck_at_fails_without_listener() {
        let code = healthcheck_at("127.0.0.1:19191").await;
        assert_eq!(code, ExitCode::FAILURE);
    }

    #[tokio::test]
    async fn healthcheck_at_succeeds_when_readyz_serves_ok() {
        let addr = "127.0.0.1:19192";
        spawn_ready_server_on(addr, Arc::new(Ready));
        tokio::time::sleep(Duration::from_millis(200)).await;
        let code = healthcheck_at(addr).await;
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[tokio::test]
    async fn readyz_reports_down_dependency_but_healthz_stays_ok() {
        let addr = "127.0.0.1:19193";
        spawn_ready_server_on(addr, Arc::new(NotReady));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(healthcheck_at(addr).await, ExitCode::FAILURE);
        let healthz = reqwest::get(format!("http://{addr}/healthz"))
            .await
            .unwrap();
        assert!(healthz.status().is_success());
    }

    struct Mati;
    impl Liveness for Mati {
        fn alive(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn healthz_gagal_bila_liveness_terpasang_melaporkan_mati() {
        let addr = "127.0.0.1:19194";
        spawn_ready_server_with_liveness_on(addr, Arc::new(Ready), Arc::new(Mati));
        tokio::time::sleep(Duration::from_millis(200)).await;
        let healthz = reqwest::get(format!("http://{addr}/healthz"))
            .await
            .unwrap();
        assert_eq!(healthz.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            healthcheck_at(addr).await,
            ExitCode::SUCCESS,
            "readyz terpisah"
        );

        // Status consumer tanpa loop terdaftar (belum ada pool) tetap hidup.
        let addr = "127.0.0.1:19195";
        let status = crate::queue::StatusKonsumen::baru(Duration::from_secs(90));
        spawn_ready_server_with_liveness_on(addr, Arc::new(Ready), status);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let healthz = reqwest::get(format!("http://{addr}/healthz"))
            .await
            .unwrap();
        assert!(healthz.status().is_success());
    }

    /// `metrics::gauge!` dari kode aplikasi harus muncul di `/metrics`:
    /// recorder Prometheus harus terpasang global, bukan hanya dibangun.
    #[tokio::test]
    async fn metrics_memuat_metrik_yang_dicatat_aplikasi() {
        let addr = "127.0.0.1:19199";
        spawn_ready_server_on(addr, Arc::new(Ready));
        metrics::gauge!("poolrouter_uji_runtime_metrics").set(42.0);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let badan = reqwest::get(format!("http://{addr}/metrics"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(
            badan.contains("poolrouter_uji_runtime_metrics 42"),
            "{badan}"
        );
    }

    #[tokio::test]
    async fn liveness_at_membaca_healthz_bukan_readyz() {
        // Worker mati tetapi dependensi siap: liveness gagal, readiness lolos.
        let addr = "127.0.0.1:19196";
        spawn_ready_server_with_liveness_on(addr, Arc::new(Ready), Arc::new(Mati));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(liveness_at(addr).await, ExitCode::FAILURE);
        assert_eq!(healthcheck_at(addr).await, ExitCode::SUCCESS);

        // Dependensi turun tetapi proses hidup: liveness tetap lolos.
        let addr = "127.0.0.1:19197";
        spawn_ready_server_on(addr, Arc::new(NotReady));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(liveness_at(addr).await, ExitCode::SUCCESS);
        assert_eq!(healthcheck_at(addr).await, ExitCode::FAILURE);

        assert_eq!(liveness_at("127.0.0.1:19198").await, ExitCode::FAILURE);
    }

    /// Temuan review ARC-68: password hasil generate dengan karakter khusus
    /// harus kembali utuh setelah URI di-parse klien AMQP.
    #[test]
    fn amqp_uri_mengembalikan_kredensial_semula() {
        use std::str::FromStr;
        let broker = RabbitmqConfig {
            host: "rabbitmq".to_owned(),
            port: 5672,
            vhost: "/".to_owned(),
            user: "pool router@x".to_owned(),
            password: secrecy::SecretString::from("a/b@c:d#e?f%g h".to_owned()),
        };
        let uri = amqp_uri(&broker);
        let parsed = lapin::uri::AMQPUri::from_str(&uri).expect("URI AMQP valid");
        assert_eq!(parsed.authority.userinfo.username, "pool router@x");
        assert_eq!(parsed.authority.userinfo.password, "a/b@c:d#e?f%g h");
        assert_eq!(parsed.authority.host, "rabbitmq");
        assert_eq!(parsed.authority.port, 5672);
        assert_eq!(parsed.vhost, "/");

        let broker = RabbitmqConfig {
            vhost: "pr/prod".to_owned(),
            ..broker
        };
        let parsed = lapin::uri::AMQPUri::from_str(&amqp_uri(&broker)).expect("URI AMQP valid");
        assert_eq!(parsed.vhost, "pr/prod");
    }

    #[tokio::test]
    async fn readiness_errors_never_carry_connection_details() {
        let opts = sqlx::postgres::PgConnectOptions::new()
            .host("127.0.0.1")
            .port(1)
            .username("pr_controller")
            .password("super-secret-password")
            .database("poolrouter");
        let readiness = DependencyReadiness {
            pg: sqlx::postgres::PgPoolOptions::new()
                .acquire_timeout(Duration::from_millis(300))
                .connect_lazy_with(opts),
            redis: redis::Client::open("redis://:super-secret-password@127.0.0.1:1").unwrap(),
            broker_uri: "amqp://poolrouter:super-secret-password@127.0.0.1:1/%2F".to_owned(),
        };
        let err = readiness.check().await.unwrap_err();
        assert_eq!(err, "postgres unavailable");
        assert!(!err.contains("super-secret-password") && !err.contains("127.0.0.1"));
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
