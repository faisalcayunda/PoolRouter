//! Startup configuration from the environment.
//!
//! Follows "Kontrak env backend PoolRouter v1" (ADR-0001 Addendum A section 4,
//! rev 11 section 7.2):
//! - names are `POOLROUTER_<COMPONENT>_<KEY>`;
//! - secrets arrive only as `<NAME>_FILE` (a path). A secret passed by value
//!   makes startup fail;
//! - every required variable is checked at startup and errors name the
//!   variable, never its value; an empty secret file counts as missing;
//! - database role names are fixed and are not read from the environment.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::str::FromStr;

use secrecy::{ExposeSecret, SecretString};

/// Which binary is starting. Decides which variables and secrets are required
/// (contract section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    /// Migration job: the only one that holds the owner role.
    Migrate,
    Webserver,
    Controller,
    Worker,
    /// Admin CLI (seed, ADR-0003 section 3.6c).
    Cli,
}

/// Fixed database roles (ADR-0001 section 7.2). The mapping to names is code,
/// not configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DbRole {
    Owner,
    Api,
    AdminApi,
    Controller,
    Webhook,
    Worker,
}

impl DbRole {
    /// Role name in PostgreSQL, also the secret file name.
    pub const fn role_name(self) -> &'static str {
        match self {
            Self::Owner => "poolrouter_owner",
            Self::Api => "pr_api",
            Self::AdminApi => "pr_admin_api",
            Self::Controller => "pr_controller",
            Self::Webhook => "pr_webhook",
            Self::Worker => "pr_worker",
        }
    }

    const fn password_var(self) -> &'static str {
        match self {
            Self::Owner => "POOLROUTER_DB_OWNER_PASSWORD",
            Self::Api => "POOLROUTER_DB_API_PASSWORD",
            Self::AdminApi => "POOLROUTER_DB_ADMIN_API_PASSWORD",
            Self::Controller => "POOLROUTER_DB_CONTROLLER_PASSWORD",
            Self::Webhook => "POOLROUTER_DB_WEBHOOK_PASSWORD",
            Self::Worker => "POOLROUTER_DB_WORKER_PASSWORD",
        }
    }
}

impl Component {
    /// Pools this binary opens, one per role (contract section 3).
    pub const fn db_roles(self) -> &'static [DbRole] {
        match self {
            Self::Migrate => &[DbRole::Owner],
            Self::Webserver => &[DbRole::Api, DbRole::AdminApi],
            Self::Controller => &[DbRole::Controller, DbRole::Webhook],
            Self::Worker => &[DbRole::Worker],
            Self::Cli => &[DbRole::AdminApi],
        }
    }

    fn needs_redis_rabbitmq(self) -> bool {
        matches!(self, Self::Webserver | Self::Controller | Self::Worker)
    }

    fn app_secrets(self) -> &'static [&'static str] {
        match self {
            Self::Webserver => &[KEK, JWT, NIK_HMAC, IDEMPOTENCY_HMAC],
            Self::Controller | Self::Worker => &[KEK],
            Self::Migrate | Self::Cli => &[],
        }
    }
}

const ENV: &str = "POOLROUTER_ENV";
const TOKEN_ISSUER: &str = "POOLROUTER_TOKEN_ISSUER";
const UI_ORIGIN: &str = "POOLROUTER_UI_ORIGIN";
const KEK: &str = "POOLROUTER_KEK";
const JWT: &str = "POOLROUTER_JWT_SIGNING_KEY";
const NIK_HMAC: &str = "POOLROUTER_NIK_HMAC_KEY";
const IDEMPOTENCY_HMAC: &str = "POOLROUTER_IDEMPOTENCY_HMAC_KEY";
const SEED_PASSWORD: &str = "POOLROUTER_SEED_PASSWORD";
const REDIS_PASSWORD: &str = "POOLROUTER_REDIS_PASSWORD";
const RABBITMQ_PASSWORD: &str = "POOLROUTER_RABBITMQ_PASSWORD";

/// Every secret name. Each one may only arrive as `<NAME>_FILE`.
const ALL_ROLES: [DbRole; 6] = [
    DbRole::Owner,
    DbRole::Api,
    DbRole::AdminApi,
    DbRole::Controller,
    DbRole::Webhook,
    DbRole::Worker,
];
const OTHER_SECRETS: [&str; 7] = [
    KEK,
    JWT,
    NIK_HMAC,
    IDEMPOTENCY_HMAC,
    SEED_PASSWORD,
    REDIS_PASSWORD,
    RABBITMQ_PASSWORD,
];

/// Deployment environment (ADR-0003 section 3.6c, R-12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Dev,
    Ci,
    Staging,
    Poc,
    Prod,
}

impl FromStr for Environment {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "dev" => Ok(Self::Dev),
            "ci" => Ok(Self::Ci),
            "staging" => Ok(Self::Staging),
            "poc" => Ok(Self::Poc),
            "prod" => Ok(Self::Prod),
            _ => Err(()),
        }
    }
}

impl Environment {
    /// Only dev and ci may carry a seed password.
    pub const fn allows_seed(self) -> bool {
        matches!(self, Self::Dev | Self::Ci)
    }
}

/// Where the environment and secret files come from. Tests use a map.
pub trait Source {
    fn var(&self, name: &str) -> Option<String>;
    fn read_file(&self, path: &str) -> io::Result<String>;
}

/// The real process environment and filesystem.
pub struct ProcessSource;

impl Source for ProcessSource {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
    fn read_file(&self, path: &str) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
}

/// One thing wrong with the configuration. Carries variable names only; a
/// value must never end up here (contract rule 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// Required variable is unset or empty.
    Missing(String),
    /// Variable is set but not a valid value (the value is not echoed).
    Invalid(String),
    /// A secret was passed by value instead of `<NAME>_FILE`.
    SecretByValue(String),
    /// The `_FILE` variable points at a file that cannot be read.
    SecretUnreadable(String),
    /// The `_FILE` variable points at an empty file.
    SecretEmpty(String),
    /// Variable must not be set for this component or environment.
    NotAllowed(String),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(v) => write!(f, "{v}: required but missing or empty"),
            Self::Invalid(v) => write!(f, "{v}: invalid value"),
            Self::SecretByValue(v) => {
                write!(f, "{v}: secret given by value, use {v}_FILE instead")
            }
            Self::SecretUnreadable(v) => write!(f, "{v}: file cannot be read"),
            Self::SecretEmpty(v) => write!(f, "{v}: file is empty"),
            Self::NotAllowed(v) => write!(f, "{v}: not allowed here"),
        }
    }
}

/// All problems found, so an operator fixes them in one round.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub struct ConfigError {
    pub problems: Vec<Problem>,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid configuration:")?;
        for p in &self.problems {
            write!(f, " [{p}]")?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub name: String,
    pub sslmode: String,
    pub pools: BTreeMap<DbRole, SecretString>,
}

#[derive(Debug)]
pub struct RedisConfig {
    pub host: String,
    pub port: u16,
    pub password: SecretString,
}

#[derive(Debug)]
pub struct RabbitmqConfig {
    pub host: String,
    pub port: u16,
    pub vhost: String,
    pub user: String,
    pub password: SecretString,
}

#[derive(Debug)]
pub struct Config {
    pub env: Environment,
    pub postgres: PostgresConfig,
    pub redis: Option<RedisConfig>,
    pub rabbitmq: Option<RabbitmqConfig>,
    /// Webserver only.
    pub token_issuer: Option<String>,
    pub ui_origin: Option<String>,
    /// Application secrets keyed by variable name without `_FILE`
    /// (for example `POOLROUTER_KEK`).
    pub secrets: BTreeMap<&'static str, SecretString>,
    /// Admin CLI seed password, dev and ci only.
    pub seed_password: Option<SecretString>,
}

impl Config {
    /// Load for `component` from the process environment.
    pub fn from_env(component: Component) -> Result<Self, ConfigError> {
        Self::load(component, &ProcessSource)
    }

    pub fn load(component: Component, src: &dyn Source) -> Result<Self, ConfigError> {
        let mut r = Reader {
            src,
            problems: Vec::new(),
        };

        r.reject_secrets_by_value();

        let env = r.env();

        let (token_issuer, ui_origin) = if component == Component::Webserver {
            (r.text(TOKEN_ISSUER), r.text(UI_ORIGIN))
        } else {
            (None, None)
        };

        let postgres = r.postgres(component);

        let (redis, rabbitmq) = if component.needs_redis_rabbitmq() {
            (r.redis(), r.rabbitmq())
        } else {
            (None, None)
        };

        let mut secrets = BTreeMap::new();
        for name in component.app_secrets() {
            if let Some(s) = r.secret_required(name) {
                secrets.insert(*name, s);
            }
        }

        let seed_password = r.seed(component, env);

        if !r.problems.is_empty() {
            return Err(ConfigError {
                problems: r.problems,
            });
        }
        // No problems means every Option above was Some; the fallbacks keep
        // this panic-free if that ever changes.
        let (Some(env), Some(postgres)) = (env, postgres) else {
            return Err(ConfigError {
                problems: vec![Problem::Invalid("configuration".into())],
            });
        };
        Ok(Self {
            env,
            postgres,
            redis,
            rabbitmq,
            token_issuer,
            ui_origin,
            secrets,
            seed_password,
        })
    }
}

struct Reader<'a> {
    src: &'a dyn Source,
    problems: Vec<Problem>,
}

impl Reader<'_> {
    fn nonempty(&self, name: &str) -> Option<String> {
        self.src.var(name).filter(|v| !v.trim().is_empty())
    }

    /// Rule 2: a secret variable that carries a value is refused, whether or
    /// not this component needs that secret.
    fn reject_secrets_by_value(&mut self) {
        let names: BTreeSet<&str> = ALL_ROLES
            .iter()
            .map(|r| r.password_var())
            .chain(OTHER_SECRETS)
            .collect();
        for name in names {
            if self.nonempty(name).is_some() {
                self.problems.push(Problem::SecretByValue(name.into()));
            }
        }
    }

    fn text(&mut self, name: &str) -> Option<String> {
        let v = self.nonempty(name);
        if v.is_none() {
            self.problems.push(Problem::Missing(name.into()));
        }
        v
    }

    fn port(&mut self, name: &str) -> Option<u16> {
        let raw = self.text(name)?;
        match raw.trim().parse::<u16>() {
            Ok(p) if p != 0 => Some(p),
            _ => {
                self.problems.push(Problem::Invalid(name.into()));
                None
            }
        }
    }

    fn env(&mut self) -> Option<Environment> {
        let raw = self.text(ENV)?;
        match raw.parse() {
            Ok(e) => Some(e),
            Err(()) => {
                self.problems.push(Problem::Invalid(ENV.into()));
                None
            }
        }
    }

    /// Read secret `<name>_FILE`. Missing variable, unreadable file and empty
    /// file are separate problems, all naming only the variable.
    fn secret(&mut self, name: &str) -> Option<SecretString> {
        let file_var = format!("{name}_FILE");
        let path = self.nonempty(&file_var)?;
        let path = path.trim().to_string();
        match self.src.read_file(&path) {
            Ok(content) => {
                let content = content.trim_end_matches(['\r', '\n']);
                if content.trim().is_empty() {
                    self.problems.push(Problem::SecretEmpty(file_var));
                    None
                } else {
                    Some(SecretString::from(content.to_string()))
                }
            }
            Err(_) => {
                self.problems.push(Problem::SecretUnreadable(file_var));
                None
            }
        }
    }

    fn secret_required(&mut self, name: &str) -> Option<SecretString> {
        if self.nonempty(&format!("{name}_FILE")).is_none() {
            self.problems.push(Problem::Missing(format!("{name}_FILE")));
            return None;
        }
        self.secret(name)
    }

    fn postgres(&mut self, component: Component) -> Option<PostgresConfig> {
        let host = self.text("POOLROUTER_DB_HOST");
        let port = self.port("POOLROUTER_DB_PORT");
        let name = self.text("POOLROUTER_DB_NAME");
        let sslmode = self.text("POOLROUTER_DB_SSLMODE");
        let mut pools = BTreeMap::new();
        for role in component.db_roles() {
            if let Some(pw) = self.secret_required(role.password_var()) {
                pools.insert(*role, pw);
            }
        }
        Some(PostgresConfig {
            host: host?,
            port: port?,
            name: name?,
            sslmode: sslmode?,
            pools,
        })
    }

    fn redis(&mut self) -> Option<RedisConfig> {
        let host = self.text("POOLROUTER_REDIS_HOST");
        let port = self.port("POOLROUTER_REDIS_PORT");
        let password = self.secret_required(REDIS_PASSWORD);
        Some(RedisConfig {
            host: host?,
            port: port?,
            password: password?,
        })
    }

    fn rabbitmq(&mut self) -> Option<RabbitmqConfig> {
        let host = self.text("POOLROUTER_RABBITMQ_HOST");
        let port = self.port("POOLROUTER_RABBITMQ_PORT");
        let vhost = self.text("POOLROUTER_RABBITMQ_VHOST");
        let user = self.text("POOLROUTER_RABBITMQ_USER");
        let password = self.secret_required(RABBITMQ_PASSWORD);
        Some(RabbitmqConfig {
            host: host?,
            port: port?,
            vhost: vhost?,
            user: user?,
            password: password?,
        })
    }

    /// Seed password: admin CLI in dev and ci only. Optional there; refused
    /// everywhere else.
    fn seed(&mut self, component: Component, env: Option<Environment>) -> Option<SecretString> {
        let file_var = format!("{SEED_PASSWORD}_FILE");
        self.nonempty(&file_var)?;
        let allowed = component == Component::Cli && env.is_some_and(Environment::allows_seed);
        if !allowed {
            self.problems.push(Problem::NotAllowed(file_var));
            return None;
        }
        self.secret(SEED_PASSWORD)
    }
}

impl PostgresConfig {
    /// Password for `role`, if this component opens that pool.
    pub fn password(&self, role: DbRole) -> Option<&str> {
        self.pools.get(&role).map(|s| s.expose_secret())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Fake {
        vars: HashMap<String, String>,
        files: HashMap<String, String>,
    }

    impl Fake {
        fn set(mut self, k: &str, v: &str) -> Self {
            self.vars.insert(k.into(), v.into());
            self
        }
        fn file(mut self, path: &str, content: &str) -> Self {
            self.files.insert(path.into(), content.into());
            self
        }
        fn unset(mut self, k: &str) -> Self {
            self.vars.remove(k);
            self
        }
    }

    impl Source for Fake {
        fn var(&self, name: &str) -> Option<String> {
            self.vars.get(name).cloned()
        }
        fn read_file(&self, path: &str) -> io::Result<String> {
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
        }
    }

    /// A complete, valid environment for the webserver. Secret values are
    /// obviously fake.
    fn webserver_ok() -> Fake {
        let mut f = Fake::default()
            .set(ENV, "dev")
            .set(TOKEN_ISSUER, "https://poolrouter.example.test")
            .set(UI_ORIGIN, "https://ui.example.test")
            .set("POOLROUTER_DB_HOST", "postgres")
            .set("POOLROUTER_DB_PORT", "5432")
            .set("POOLROUTER_DB_NAME", "poolrouter")
            .set("POOLROUTER_DB_SSLMODE", "disable")
            .set("POOLROUTER_REDIS_HOST", "redis")
            .set("POOLROUTER_REDIS_PORT", "6379")
            .set(
                "POOLROUTER_REDIS_PASSWORD_FILE",
                "/run/secrets/redis_password",
            )
            .file("/run/secrets/redis_password", "fake-redis\n")
            .set("POOLROUTER_RABBITMQ_HOST", "rabbitmq")
            .set("POOLROUTER_RABBITMQ_PORT", "5672")
            .set("POOLROUTER_RABBITMQ_VHOST", "/")
            .set("POOLROUTER_RABBITMQ_USER", "poolrouter")
            .set(
                "POOLROUTER_RABBITMQ_PASSWORD_FILE",
                "/run/secrets/rabbitmq_password",
            )
            .file("/run/secrets/rabbitmq_password", "fake-rabbit");
        for role in [DbRole::Api, DbRole::AdminApi] {
            let var = format!("{}_FILE", role.password_var());
            let path = format!("/run/secrets/{}", role.role_name());
            f = f.set(&var, &path).file(&path, "fake-db");
        }
        for (name, file) in [
            (KEK, "poolrouter_kek"),
            (JWT, "poolrouter_jwt_signing_key"),
            (NIK_HMAC, "poolrouter_nik_hmac_key"),
            (IDEMPOTENCY_HMAC, "poolrouter_idempotency_hmac_key"),
        ] {
            let path = format!("/run/secrets/{file}");
            f = f
                .set(&format!("{name}_FILE"), &path)
                .file(&path, "fake-secret");
        }
        f
    }

    fn problems(res: Result<Config, ConfigError>) -> Vec<Problem> {
        res.expect_err("config must be rejected").problems
    }

    #[test]
    fn valid_webserver_config_loads() {
        let cfg = Config::load(Component::Webserver, &webserver_ok()).expect("valid");
        assert_eq!(cfg.env, Environment::Dev);
        assert_eq!(cfg.postgres.port, 5432);
        assert_eq!(cfg.postgres.pools.len(), 2);
        assert_eq!(cfg.postgres.password(DbRole::Api), Some("fake-db"));
        // trailing newline of a secret file is not part of the secret
        assert_eq!(
            cfg.redis.as_ref().unwrap().password.expose_secret(),
            "fake-redis"
        );
        assert!(cfg.secrets.contains_key(KEK));
    }

    #[test]
    fn missing_required_variable_names_it_without_a_value() {
        let src = webserver_ok().unset("POOLROUTER_DB_HOST");
        let err = Config::load(Component::Webserver, &src).unwrap_err();
        assert_eq!(
            err.problems,
            vec![Problem::Missing("POOLROUTER_DB_HOST".into())]
        );
        let msg = err.to_string();
        assert!(msg.contains("POOLROUTER_DB_HOST"));
        assert!(!msg.contains("fake-"));
    }

    #[test]
    fn missing_secret_file_variable_is_reported_by_name() {
        let src = webserver_ok().unset("POOLROUTER_KEK_FILE");
        assert_eq!(
            problems(Config::load(Component::Webserver, &src)),
            vec![Problem::Missing("POOLROUTER_KEK_FILE".into())]
        );
    }

    #[test]
    fn secret_given_by_value_is_refused() {
        let src = webserver_ok()
            .set(KEK, "leaked-value")
            .set("POOLROUTER_DB_API_PASSWORD", "leaked-db");
        let err = Config::load(Component::Webserver, &src).unwrap_err();
        assert!(err.problems.contains(&Problem::SecretByValue(KEK.into())));
        assert!(
            err.problems
                .contains(&Problem::SecretByValue("POOLROUTER_DB_API_PASSWORD".into()))
        );
        assert!(!err.to_string().contains("leaked"));
    }

    #[test]
    fn secret_by_value_is_refused_even_if_the_component_does_not_use_it() {
        let src = webserver_ok().set(SEED_PASSWORD, "leaked-seed");
        assert!(
            problems(Config::load(Component::Webserver, &src))
                .contains(&Problem::SecretByValue(SEED_PASSWORD.into()))
        );
    }

    #[test]
    fn empty_secret_file_counts_as_missing() {
        let src = webserver_ok().file("/run/secrets/poolrouter_kek", "\n");
        assert_eq!(
            problems(Config::load(Component::Webserver, &src)),
            vec![Problem::SecretEmpty("POOLROUTER_KEK_FILE".into())]
        );
    }

    #[test]
    fn unreadable_secret_file_is_rejected_by_variable_name() {
        let src = webserver_ok().set("POOLROUTER_REDIS_PASSWORD_FILE", "/nope");
        let err = Config::load(Component::Webserver, &src).unwrap_err();
        assert_eq!(
            err.problems,
            vec![Problem::SecretUnreadable(
                "POOLROUTER_REDIS_PASSWORD_FILE".into()
            )]
        );
        assert!(!err.to_string().contains("/nope"));
    }

    #[test]
    fn invalid_port_and_env_do_not_echo_the_value() {
        let src = webserver_ok()
            .set("POOLROUTER_DB_PORT", "not-a-port")
            .set(ENV, "production");
        let err = Config::load(Component::Webserver, &src).unwrap_err();
        assert!(
            err.problems
                .contains(&Problem::Invalid("POOLROUTER_DB_PORT".into()))
        );
        assert!(err.problems.contains(&Problem::Invalid(ENV.into())));
        assert!(!err.to_string().contains("not-a-port"));
    }

    #[test]
    fn worker_needs_only_its_own_role_and_secrets() {
        // Webserver-only variables and the other roles' files are not needed.
        let mut src = webserver_ok()
            .unset(TOKEN_ISSUER)
            .unset(UI_ORIGIN)
            .unset("POOLROUTER_DB_API_PASSWORD_FILE")
            .unset("POOLROUTER_DB_ADMIN_API_PASSWORD_FILE")
            .unset("POOLROUTER_JWT_SIGNING_KEY_FILE")
            .unset("POOLROUTER_NIK_HMAC_KEY_FILE")
            .unset("POOLROUTER_IDEMPOTENCY_HMAC_KEY_FILE");
        src = src
            .set(
                "POOLROUTER_DB_WORKER_PASSWORD_FILE",
                "/run/secrets/pr_worker",
            )
            .file("/run/secrets/pr_worker", "fake-db");
        let cfg = Config::load(Component::Worker, &src).expect("worker config");
        assert_eq!(
            cfg.postgres.pools.keys().copied().collect::<Vec<_>>(),
            vec![DbRole::Worker]
        );
        assert_eq!(cfg.secrets.len(), 1);
    }

    #[test]
    fn migrate_uses_only_the_owner_role() {
        let src = Fake::default()
            .set(ENV, "ci")
            .set("POOLROUTER_DB_HOST", "postgres")
            .set("POOLROUTER_DB_PORT", "5432")
            .set("POOLROUTER_DB_NAME", "poolrouter")
            .set("POOLROUTER_DB_SSLMODE", "disable")
            .set(
                "POOLROUTER_DB_OWNER_PASSWORD_FILE",
                "/run/secrets/poolrouter_owner",
            )
            .file("/run/secrets/poolrouter_owner", "fake-owner");
        let cfg = Config::load(Component::Migrate, &src).expect("migrate config");
        assert!(cfg.redis.is_none() && cfg.rabbitmq.is_none());
        assert_eq!(cfg.postgres.password(DbRole::Owner), Some("fake-owner"));
    }

    #[test]
    fn seed_password_only_for_cli_in_dev_or_ci() {
        let base = |env: &str| {
            Fake::default()
                .set(ENV, env)
                .set("POOLROUTER_DB_HOST", "postgres")
                .set("POOLROUTER_DB_PORT", "5432")
                .set("POOLROUTER_DB_NAME", "poolrouter")
                .set("POOLROUTER_DB_SSLMODE", "disable")
                .set(
                    "POOLROUTER_DB_ADMIN_API_PASSWORD_FILE",
                    "/run/secrets/pr_admin_api",
                )
                .file("/run/secrets/pr_admin_api", "fake-db")
                .set("POOLROUTER_SEED_PASSWORD_FILE", "/run/secrets/seed")
                .file("/run/secrets/seed", "fake-seed")
        };
        let cfg = Config::load(Component::Cli, &base("dev")).expect("dev cli");
        assert!(cfg.seed_password.is_some());
        assert_eq!(
            problems(Config::load(Component::Cli, &base("prod"))),
            vec![Problem::NotAllowed("POOLROUTER_SEED_PASSWORD_FILE".into())]
        );
    }

    #[test]
    fn debug_output_never_shows_secrets() {
        let cfg = Config::load(Component::Webserver, &webserver_ok()).unwrap();
        let dump = format!("{cfg:?}");
        assert!(!dump.contains("fake-db") && !dump.contains("fake-secret"));
    }
}
