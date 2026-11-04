//! Webserver runtime: API listener, admin listener (probes and metrics),
//! dependencies, graceful shutdown (ADR-0001 rev 15 2, 5a.9; SHAA-198).
//!
//! Addresses: `POOLROUTER_API_ADDR` (default `127.0.0.1:8081`) and
//! `POOLROUTER_ADMIN_ADDR` (default `127.0.0.1:9090`). Compose sets `0.0.0.0` on
//! the internal network and publishes no host port for the admin listener.

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::MatchedPath;
use axum::http::{Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::post;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use poolrouter_core::config::{Config, DbRole};
use poolrouter_core::kuota::pemulihan::{Kesiapan, periksa_kesiapan};
use redis::IntoConnectionInfo;
use secrecy::ExposeSecret;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use tokio_util::sync::CancellationToken;
use tower_http::limit::RequestBodyLimitLayer;

use super::auth::Authenticator;
use super::credential::{CachedStore, PgCredentialStore};
use super::jwt::JwtKeys;
use super::layers::{ApiError, no_store};
use super::probe::{self, ADMIN_ADDR_VAR, DEFAULT_ADMIN_ADDR, Readiness};
use super::token::{LocalThrottle, TokenThrottle, issue_token};
use crate::api::problem::ProblemKodeDown;

pub const API_ADDR_VAR: &str = "POOLROUTER_API_ADDR";
pub const DEFAULT_API_ADDR: &str = "127.0.0.1:8081";
const VERIFY_KEY_FILE_VAR: &str = "POOLROUTER_JWT_VERIFY_KEY_FILE";
/// Token requests are tiny forms; anything larger is refused before parsing.
const MAX_BODY_BYTES: usize = 16 * 1024;
/// Redis command budget (ADR-0001 5a.4).
const REDIS_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub struct AppState {
    pub auth: Arc<Authenticator>,
    pub throttle: Arc<dyn TokenThrottle>,
}

async fn tidak_ditemukan() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, ProblemKodeDown::TidakDitemukan)
}

/// Request count and latency by route template, method and status
/// (`poolrouter_http_request_duration_seconds`, ADR-0001 5a.9). The route is
/// the matched template, never the raw path, so cardinality stays closed.
async fn http_metrics(req: Request<axum::body::Body>, next: Next) -> Response {
    let method = req.method().to_string();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "tidak_dikenal".to_owned(), |m| m.as_str().to_owned());
    let started = Instant::now();
    let res = next.run(req).await;
    metrics::histogram!(
        "poolrouter_http_request_duration_seconds",
        "route" => route,
        "method" => method,
        "http_status" => res.status().as_u16().to_string(),
    )
    .record(started.elapsed().as_secs_f64());
    res
}

/// The API router. Endpoints added by later parts of SHAA-198 join here.
pub fn api_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/oauth/token", post(issue_token))
        .fallback(tidak_ditemukan)
        .with_state(state)
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(middleware::from_fn(http_metrics))
        .layer(no_store())
}

/// PostgreSQL and Redis (`pr:siap` matches). Failure text is fixed: driver
/// messages can carry hosts, users or DSN fragments.
struct DependencyReadiness {
    pg: PgPool,
    redis: redis::Client,
}

impl Readiness for DependencyReadiness {
    fn check(&self) -> probe::BoxFut<'_> {
        Box::pin(async move {
            sqlx::query("SELECT 1")
                .execute(&self.pg)
                .await
                .map_err(|_| "postgres unavailable".to_owned())?;
            let check = async {
                let mut conn = self.redis.get_multiplexed_async_connection().await?;
                periksa_kesiapan(&mut conn).await.map_err(|e| match e {
                    poolrouter_core::kuota::pemulihan::GalatPemulihan::Redis(r) => r,
                    _ => redis::RedisError::from((redis::ErrorKind::Client, "readiness")),
                })
            };
            match tokio::time::timeout(REDIS_TIMEOUT * 4, check).await {
                Ok(Ok(Kesiapan::Siap { .. })) => Ok(()),
                Ok(Ok(Kesiapan::PerluRebuild { .. })) => Err("redis not ready".to_owned()),
                Ok(Err(_)) | Err(_) => Err("redis unavailable".to_owned()),
            }
        })
    }
}

fn addr_from_env(var: &str, default: &str) -> Result<SocketAddr, String> {
    let raw = std::env::var(var).unwrap_or_else(|_| default.to_owned());
    raw.parse()
        .map_err(|_| format!("{var} is not a socket address"))
}

fn ssl_mode(raw: &str) -> Result<PgSslMode, String> {
    raw.parse()
        .map_err(|_| "POOLROUTER_DB_SSLMODE is not a PostgreSQL ssl mode".to_owned())
}

fn pg_pool(cfg: &Config) -> Result<PgPool, String> {
    let role = DbRole::Api;
    let password = cfg
        .postgres
        .password(role)
        .ok_or_else(|| "missing password for the api role".to_owned())?;
    let opts = PgConnectOptions::new()
        .host(&cfg.postgres.host)
        .port(cfg.postgres.port)
        .database(&cfg.postgres.name)
        .username(role.role_name())
        .password(password)
        .ssl_mode(ssl_mode(&cfg.postgres.sslmode)?);
    // Lazy: the process starts and `/readyz` reports the outage, instead of
    // exiting and restarting in a loop.
    Ok(PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy_with(opts))
}

fn redis_client(cfg: &Config) -> Result<redis::Client, String> {
    let r = cfg.redis.as_ref().ok_or("redis configuration missing")?;
    let info = redis::ConnectionAddr::Tcp(r.host.clone(), r.port)
        .into_connection_info()
        .map_err(|_| "redis configuration is not valid".to_owned())?
        .set_redis_settings(
            redis::RedisConnectionInfo::default().set_password(r.password.expose_secret()),
        );
    redis::Client::open(info).map_err(|_| "redis configuration is not valid".to_owned())
}

fn jwt_keys(cfg: &Config) -> Result<JwtKeys, String> {
    let issuer = cfg
        .token_issuer
        .as_deref()
        .ok_or("POOLROUTER_TOKEN_ISSUER missing")?;
    let signing = cfg
        .secrets
        .get("POOLROUTER_JWT_SIGNING_KEY")
        .ok_or("POOLROUTER_JWT_SIGNING_KEY_FILE missing")?;
    let verify = match std::env::var(VERIFY_KEY_FILE_VAR) {
        Ok(path) if !path.trim().is_empty() => Some(
            std::fs::read_to_string(path.trim())
                .map_err(|_| format!("{VERIFY_KEY_FILE_VAR} cannot be read"))?,
        ),
        _ => None,
    };
    JwtKeys::from_pem(issuer, signing.expose_secret(), verify.as_deref())
        .map_err(|e| format!("JWT key rejected: {e}"))
}

async fn serve_until(
    name: &'static str,
    addr: SocketAddr,
    app: Router,
    shutdown: CancellationToken,
) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|_| format!("{name}: cannot bind {addr}"))?;
    tracing::info!(listener = name, %addr, "listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move { shutdown.cancelled().await })
    .await
    .map_err(|_| format!("{name}: server error"))
}

/// Start the webserver and run until SIGINT or SIGTERM. Exit 2 for a bad
/// configuration or key (the message names the variable, never its value).
pub async fn run(cfg: Config) -> ExitCode {
    poolrouter_core::runtime::init_tracing();
    match start(cfg).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            tracing::error!(error = %msg, "webserver stopped");
            ExitCode::from(2)
        }
    }
}

async fn start(cfg: Config) -> Result<(), String> {
    let api_addr = addr_from_env(API_ADDR_VAR, DEFAULT_API_ADDR)?;
    let admin_addr = addr_from_env(ADMIN_ADDR_VAR, DEFAULT_ADMIN_ADDR)?;
    let keys = Arc::new(jwt_keys(&cfg)?);
    let pg = pg_pool(&cfg)?;
    let redis = redis_client(&cfg)?;
    tracing::info!(kid = keys.signing_kid(), "signing key loaded");

    let metrics: PrometheusHandle = PrometheusBuilder::new()
        .install_recorder()
        .map_err(|_| "metrics recorder already installed".to_owned())?;

    let store = Arc::new(CachedStore::new(Arc::new(PgCredentialStore::new(
        pg.clone(),
    ))));
    let state = AppState {
        auth: Arc::new(Authenticator::new(store, keys)),
        throttle: Arc::new(LocalThrottle::new()),
    };
    let api = api_router(state);
    let admin = probe::admin_router(Arc::new(DependencyReadiness { pg, redis }), metrics);

    let shutdown = CancellationToken::new();
    tokio::spawn(poolrouter_core::runtime::cancel_on_signal(shutdown.clone()));
    let (a, b) = tokio::join!(
        serve_until("api", api_addr, api, shutdown.clone()),
        serve_until("admin", admin_addr, admin, shutdown.clone()),
    );
    // Whichever listener ends first (error or signal), stop the other one too.
    shutdown.cancel();
    a.and(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downstream::credential::{Jenis, Kredensial, fake::FakeStore};
    use argon2::{Algorithm, Argon2, Params, PasswordHasher, Version};
    use axum::body::{Body, to_bytes};
    use axum::extract::connect_info::MockConnectInfo;
    use axum::http::header;
    use poolrouter_core::kredensial::{base64url_encode, buat_api_key};
    use tower::ServiceExt;
    use uuid::Uuid;

    const ISS: &str = "https://poolrouter.example.test";

    fn phc(secret: &str) -> Vec<u8> {
        Argon2::new(
            Algorithm::Argon2id,
            Version::V0x13,
            Params::new(8, 1, 1, None).unwrap(),
        )
        .hash_password_with_salt(secret.as_bytes(), b"0123456789abcdef")
        .unwrap()
        .to_string()
        .into_bytes()
    }

    fn keys() -> Arc<JwtKeys> {
        use aws_lc_rs::rand::SystemRandom;
        use aws_lc_rs::signature::Ed25519KeyPair;
        let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let b = base64url_encode(doc.as_ref())
            .replace('-', "+")
            .replace('_', "/");
        let pem = format!("-----BEGIN PRIVATE KEY-----\n{b}\n-----END PRIVATE KEY-----\n");
        Arc::new(JwtKeys::from_pem(ISS, &pem, None).unwrap())
    }

    struct Harness {
        app: Router,
        fake: Arc<FakeStore>,
        client: Uuid,
    }

    fn harness_with(throttle: Arc<dyn TokenThrottle>) -> Harness {
        let fake = Arc::new(FakeStore::default());
        let client = Uuid::now_v7();
        fake.clients.lock().unwrap().insert(client, true);
        fake.kredensial.lock().unwrap().push(Kredensial {
            credential_id: Uuid::now_v7(),
            client_id: client,
            jenis: Jenis::ClientSecret,
            kunci_hash: phc("s3cret"),
            aktif: true,
            kedaluwarsa_at: None,
        });
        let state = AppState {
            auth: Arc::new(Authenticator::new(fake.clone(), keys())),
            throttle,
        };
        let app =
            api_router(state).layer(MockConnectInfo(SocketAddr::from(([203, 0, 113, 7], 5555))));
        Harness { app, fake, client }
    }

    fn harness() -> Harness {
        harness_with(Arc::new(LocalThrottle::new()))
    }

    fn form(body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/v1/oauth/token")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body.to_owned()))
            .unwrap()
    }

    async fn json(res: Response) -> serde_json::Value {
        serde_json::from_slice(&to_bytes(res.into_body(), 64 * 1024).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn token_endpoint_issues_a_bearer_token_from_form_credentials() {
        let h = harness();
        let body = format!(
            "grant_type=client_credentials&client_id={}&client_secret=s3cret",
            h.client
        );
        let res = h.app.clone().oneshot(form(&body)).await.unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");
        let v = json(res).await;
        assert_eq!(v["token_type"], "Bearer");
        assert_eq!(v["expires_in"], 900);
        assert!(v.get("refresh_token").is_none(), "no refresh token (R-05)");
        assert!(v["access_token"].as_str().unwrap().split('.').count() == 3);
    }

    #[tokio::test]
    async fn token_endpoint_accepts_http_basic() {
        let h = harness();
        let raw = format!("{}:s3cret", h.client);
        let std_b64 = base64url_encode(raw.as_bytes())
            .replace('-', "+")
            .replace('_', "/");
        let pad = "=".repeat((4 - std_b64.len() % 4) % 4);
        let mut req = form("grant_type=client_credentials");
        req.headers_mut().insert(
            header::AUTHORIZATION,
            format!("Basic {std_b64}{pad}").parse().unwrap(),
        );
        assert_eq!(h.app.clone().oneshot(req).await.unwrap().status(), 200);
    }

    #[tokio::test]
    async fn wrong_secret_unknown_client_and_bad_id_are_the_same_401() {
        let h = harness();
        let wrong = format!(
            "grant_type=client_credentials&client_id={}&client_secret=nope",
            h.client
        );
        let unknown = format!(
            "grant_type=client_credentials&client_id={}&client_secret=s3cret",
            Uuid::now_v7()
        );
        let bad_id = "grant_type=client_credentials&client_id=bgn-mpm-prod&client_secret=s3cret";
        let mut seen = vec![];
        for b in [wrong.as_str(), unknown.as_str(), bad_id] {
            let res = h.app.clone().oneshot(form(b)).await.unwrap();
            assert_eq!(res.status(), 401, "{b}");
            assert_eq!(res.headers()[header::WWW_AUTHENTICATE], "Basic");
            assert_eq!(
                res.headers()[header::CONTENT_TYPE],
                "application/problem+json"
            );
            seen.push(json(res).await);
        }
        assert_eq!(seen[0], seen[1]);
        assert_eq!(seen[1], seen[2]);
        assert_eq!(seen[0]["kode"], "tidak_terautentikasi");
        assert_eq!(seen[0]["error"], "invalid_client");
    }

    #[tokio::test]
    async fn non_uuid_client_id_does_not_query_the_database() {
        let h = harness();
        let res = h
            .app
            .clone()
            .oneshot(form(
                "grant_type=client_credentials&client_id=bgn-mpm&client_secret=x",
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), 401);
        assert_eq!(h.fake.calls(), 0);
    }

    #[tokio::test]
    async fn malformed_requests_are_400_problem_json() {
        let h = harness();
        let id = h.client;
        let cases = [
            (
                "grant_type=password&client_id=x&client_secret=y".to_owned(),
                "unsupported_grant_type",
            ),
            ("client_id=x&client_secret=y".to_owned(), "invalid_request"),
            (
                format!("grant_type=client_credentials&client_id={id}"),
                "invalid_request",
            ),
            (
                "grant_type=client_credentials".to_owned(),
                "invalid_request",
            ),
            (
                format!("grant_type=client_credentials&client_id={id}&client_secret=a&extra=1"),
                "invalid_request",
            ),
            (
                format!(
                    "grant_type=client_credentials&client_id={id}&client_id={id}&client_secret=a"
                ),
                "invalid_request",
            ),
        ];
        for (body, err) in cases {
            let res = h.app.clone().oneshot(form(&body)).await.unwrap();
            assert_eq!(res.status(), 400, "{body}");
            let v = json(res).await;
            assert_eq!(v["kode"], "validasi_gagal", "{body}");
            assert_eq!(v["error"], err, "{body}");
        }
        // JSON body is not a form.
        let mut req = form("{\"grant_type\":\"client_credentials\"}");
        req.headers_mut()
            .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        assert_eq!(h.app.clone().oneshot(req).await.unwrap().status(), 400);
        assert_eq!(h.fake.calls(), 0, "malformed input never reaches the store");
    }

    #[tokio::test]
    async fn per_client_throttle_returns_429_with_retry_after_and_no_error_field() {
        let h = harness();
        let body = format!(
            "grant_type=client_credentials&client_id={}&client_secret=nope",
            h.client
        );
        let mut last = None;
        for _ in 0..12 {
            last = Some(h.app.clone().oneshot(form(&body)).await.unwrap());
            if last.as_ref().unwrap().status() == 429 {
                break;
            }
        }
        let res = last.unwrap();
        assert_eq!(res.status(), 429, "burst of 10 exhausted");
        let secs: u64 = res.headers()[header::RETRY_AFTER]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!(secs >= 1);
        assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");
        let v = json(res).await;
        assert_eq!(v["kode"], "token_dibatasi");
        assert!(
            v.get("error").is_none(),
            "429 never carries an RFC 6749 error"
        );
    }

    #[tokio::test]
    async fn per_ip_throttle_applies_before_any_parsing() {
        struct Deny;
        impl TokenThrottle for Deny {
            fn ip(&self, _: std::net::IpAddr) -> Result<(), Duration> {
                Err(Duration::from_millis(1500))
            }
            fn client(&self, _: Uuid) -> Result<(), Duration> {
                Ok(())
            }
        }
        let h = harness_with(Arc::new(Deny));
        let res = h.app.clone().oneshot(form("garbage")).await.unwrap();
        assert_eq!(res.status(), 429);
        assert_eq!(res.headers()[header::RETRY_AFTER], "2");
        assert_eq!(h.fake.calls(), 0);
    }

    #[tokio::test]
    async fn store_outage_is_503_with_retry_after_5() {
        let h = harness();
        *h.fake.fail.lock().unwrap() = true;
        let body = format!(
            "grant_type=client_credentials&client_id={}&client_secret=s3cret",
            h.client
        );
        let res = h.app.clone().oneshot(form(&body)).await.unwrap();
        assert_eq!(res.status(), 503);
        assert_eq!(res.headers()[header::RETRY_AFTER], "5");
        assert_eq!(json(res).await["kode"], "layanan_tidak_tersedia");
    }

    #[tokio::test]
    async fn oversized_body_and_unknown_routes_are_refused_as_problem_json() {
        let h = harness();
        let res = h
            .app
            .clone()
            .oneshot(form(&format!(
                "grant_type={}",
                "a".repeat(MAX_BODY_BYTES + 1)
            )))
            .await
            .unwrap();
        assert_eq!(res.status(), 413);
        let res = h
            .app
            .clone()
            .oneshot(Request::builder().uri("/nope").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), 404);
        assert_eq!(
            res.headers()[header::CONTENT_TYPE],
            "application/problem+json"
        );
        assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");
    }

    #[tokio::test]
    async fn issued_token_authenticates_a_protected_handler_and_api_keys_too() {
        use crate::downstream::auth::AuthClient;
        let h = harness();
        let body = format!(
            "grant_type=client_credentials&client_id={}&client_secret=s3cret",
            h.client
        );
        let res = h.app.clone().oneshot(form(&body)).await.unwrap();
        let token = json(res).await["access_token"].as_str().unwrap().to_owned();

        // A protected route sharing the same state as the real router.
        let cred_id = Uuid::now_v7();
        let key = buat_api_key(cred_id).unwrap();
        h.fake.kredensial.lock().unwrap().push(Kredensial {
            credential_id: cred_id,
            client_id: h.client,
            jenis: Jenis::ApiKey,
            kunci_hash: key.kunci_hash.to_vec(),
            aktif: true,
            kedaluwarsa_at: None,
        });
        let state = AppState {
            auth: Arc::new(Authenticator::new(h.fake.clone(), keys())),
            throttle: Arc::new(LocalThrottle::new()),
        };
        // Different signing key => bearer from `h` must be refused here.
        let protected = Router::new()
            .route(
                "/whoami",
                axum::routing::get(|AuthClient(id): AuthClient| async move { id.to_string() }),
            )
            .with_state(state);
        let get = |hdr: Option<(&str, String)>| {
            let mut r = Request::builder().uri("/whoami");
            if let Some((k, v)) = hdr {
                r = r.header(k, v);
            }
            r.body(Body::empty()).unwrap()
        };
        let res = protected.clone().oneshot(get(None)).await.unwrap();
        assert_eq!(res.status(), 401);
        let res = protected
            .clone()
            .oneshot(get(Some(("x-api-key", key.kunci.clone()))))
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let res = protected
            .clone()
            .oneshot(get(Some(("authorization", format!("Bearer {token}")))))
            .await
            .unwrap();
        assert_eq!(res.status(), 401, "token signed by another key");
    }

    #[test]
    fn addresses_default_to_loopback_and_reject_garbage() {
        assert!(
            DEFAULT_API_ADDR
                .parse::<SocketAddr>()
                .unwrap()
                .ip()
                .is_loopback()
        );
        assert!(addr_from_env("POOLROUTER_TEST_UNSET_ADDR", "127.0.0.1:1").is_ok());
        assert!(
            DEFAULT_ADMIN_ADDR
                .parse::<SocketAddr>()
                .unwrap()
                .ip()
                .is_loopback()
        );
    }

    #[test]
    fn ssl_mode_values_are_validated() {
        assert!(ssl_mode("require").is_ok());
        assert!(ssl_mode("verify-full").is_ok());
        assert!(ssl_mode("bogus").is_err());
    }

    #[tokio::test]
    async fn readiness_errors_never_carry_connection_details() {
        // Port 1 on loopback: nothing listens, both dependencies fail.
        let opts = PgConnectOptions::new()
            .host("127.0.0.1")
            .port(1)
            .username("pr_api")
            .password("super-secret-password")
            .database("poolrouter");
        let pg = PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(300))
            .connect_lazy_with(opts);
        let redis = redis::Client::open("redis://:super-secret-password@127.0.0.1:1").unwrap();
        let r = DependencyReadiness { pg, redis };
        let err = r.check().await.unwrap_err();
        assert!(
            !err.contains("super-secret-password") && !err.contains("127.0.0.1"),
            "{err}"
        );
        assert_eq!(err, "postgres unavailable");
    }
}
