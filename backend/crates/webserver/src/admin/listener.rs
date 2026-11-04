//! The third listener: the admin API on `POOLROUTER_ADMIN_API_ADDR` (Shu,
//! SHAA-196 2026-11-04; ADR-0003 §3.9).
//!
//! It is separate from the downstream API listener (so a downstream client that
//! reaches `8081` cannot reach `/admin/v1`) and from the probe/metrics listener
//! (`POOLROUTER_ADMIN_ADDR`, whose "admin" means probes, not this).
//!
//! The admin router is built from [`AdminState`], which holds the
//! `pr_admin_api` pool only; the downstream router is built from a different
//! state that holds the `pr_api` pool only. The separation is by type, not by
//! convention.

use std::net::SocketAddr;
use std::time::Duration;

use poolrouter_core::admin::AsalUi;
use poolrouter_core::config::{Config, DbRole};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};

use secrecy::ExposeSecret;

use super::pembatas::PembatasLogin;
use super::sesi::AdminState;

pub const ADMIN_API_ADDR_VAR: &str = "POOLROUTER_ADMIN_API_ADDR";
pub const DEFAULT_ADMIN_API_ADDR: &str = "127.0.0.1:8082";

/// Resolve the admin API address. `raw` is the variable's value, `None` when
/// unset (default applies). An empty value is an error, not a silent default:
/// a deployment that sets the variable meant to set it. The address may not
/// collide with the other two listeners.
pub fn alamat_listener(
    raw: Option<&str>,
    api: SocketAddr,
    probe: SocketAddr,
) -> Result<SocketAddr, String> {
    let teks = match raw {
        None => DEFAULT_ADMIN_API_ADDR,
        Some(v) if v.trim().is_empty() => {
            return Err(format!("{ADMIN_API_ADDR_VAR} is set but empty"));
        }
        Some(v) => v.trim(),
    };
    let alamat: SocketAddr = teks
        .parse()
        .map_err(|_| format!("{ADMIN_API_ADDR_VAR} is not a socket address"))?;
    for (nama, lain) in [
        ("POOLROUTER_API_ADDR", api),
        ("POOLROUTER_ADMIN_ADDR", probe),
    ] {
        if bentrok(alamat, lain) {
            return Err(format!("{ADMIN_API_ADDR_VAR} collides with {nama}"));
        }
    }
    Ok(alamat)
}

/// Two binds clash when the ports match and the hosts overlap (equal, or
/// either one is the unspecified address that covers the other).
fn bentrok(a: SocketAddr, b: SocketAddr) -> bool {
    a.port() == b.port()
        && (a.ip() == b.ip()
            || a.ip().is_unspecified() && a.is_ipv4() == b.is_ipv4()
            || b.ip().is_unspecified() && a.is_ipv4() == b.is_ipv4())
}

/// The `pr_admin_api` pool, separate from the `pr_api` pool. `statement_timeout`
/// for this role is a database setting per role, not set here (ADR-0001 §7.2).
pub fn pool_admin(cfg: &Config) -> Result<PgPool, String> {
    let role = DbRole::AdminApi;
    let password = cfg
        .postgres
        .password(role)
        .ok_or_else(|| "missing password for the admin api role".to_owned())?;
    let ssl: PgSslMode = cfg
        .postgres
        .sslmode
        .parse()
        .map_err(|_| "POOLROUTER_DB_SSLMODE is not a PostgreSQL ssl mode".to_owned())?;
    let opts = PgConnectOptions::new()
        .host(&cfg.postgres.host)
        .port(cfg.postgres.port)
        .database(&cfg.postgres.name)
        .username(role.role_name())
        .password(password)
        .ssl_mode(ssl);
    Ok(PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy_with(opts))
}

pub fn state_admin(cfg: &Config, pool: PgPool, redis: redis::Client) -> Result<AdminState, String> {
    let asal = cfg
        .ui_origin
        .as_deref()
        .ok_or("POOLROUTER_UI_ORIGIN missing")?;
    let asal =
        AsalUi::baru(asal).map_err(|_| "POOLROUTER_UI_ORIGIN is not an origin".to_owned())?;
    // The limiter key is an HMAC under a key the webserver already holds, with
    // its own label (see `pembatas`): no new secret.
    let kunci = cfg
        .secrets
        .get("POOLROUTER_IDEMPOTENCY_HMAC_KEY")
        .ok_or("POOLROUTER_IDEMPOTENCY_HMAC_KEY_FILE missing")?;
    let pembatas = PembatasLogin::baru(redis, kunci.expose_secret().as_bytes());
    Ok(AdminState::baru(pool, asal, pembatas))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn default_dipakai_bila_tidak_diset() {
        let r = alamat_listener(None, a("127.0.0.1:8081"), a("127.0.0.1:9090")).unwrap();
        assert_eq!(r, a(DEFAULT_ADMIN_API_ADDR));
    }

    #[test]
    fn nilai_kosong_atau_bukan_alamat_ditolak() {
        let (x, y) = (a("127.0.0.1:8081"), a("127.0.0.1:9090"));
        assert!(alamat_listener(Some(""), x, y).is_err());
        assert!(alamat_listener(Some("  "), x, y).is_err());
        assert!(alamat_listener(Some("bukan-alamat"), x, y).is_err());
    }

    #[test]
    fn bentrok_dengan_listener_lain_ditolak() {
        let (x, y) = (a("0.0.0.0:8081"), a("0.0.0.0:9090"));
        assert!(alamat_listener(Some("0.0.0.0:8081"), x, y).is_err());
        assert!(alamat_listener(Some("127.0.0.1:8081"), x, y).is_err()); // wildcard covers it
        assert!(alamat_listener(Some("0.0.0.0:9090"), x, y).is_err());
        assert!(alamat_listener(Some("0.0.0.0:8082"), x, y).is_ok());
    }

    #[test]
    fn host_berbeda_di_port_sama_tidak_bentrok() {
        let (x, y) = (a("127.0.0.1:8081"), a("127.0.0.1:9090"));
        assert!(alamat_listener(Some("127.0.0.2:8081"), x, y).is_ok());
    }
}
