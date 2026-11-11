//! Titik masuk `migrate`: menerapkan `backend/migrations/` sebagai `poolrouter_owner`
//! (ADR-0001 §7.2). Dijalankan sekali oleh service Compose `migrate` (profile `migrate`),
//! bukan oleh aplikasi. Kredensial owner hanya ada di service itu.

use std::process::ExitCode;
use std::time::Duration;

use poolrouter_cli::migrasi::{MIGRATOR, terapkan};
use poolrouter_core::config::{Component, Config, DbRole};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};

#[tokio::main]
async fn main() -> ExitCode {
    let cfg = match Config::from_env(Component::Migrate) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("migrate: {e}");
            return ExitCode::from(2);
        }
    };
    let Some(password) = cfg.postgres.password(DbRole::Owner) else {
        eprintln!("migrate: password role poolrouter_owner tidak tersedia");
        return ExitCode::from(2);
    };
    let Ok(ssl) = cfg.postgres.sslmode.parse::<PgSslMode>() else {
        eprintln!("migrate: POOLROUTER_DB_SSLMODE bukan mode ssl PostgreSQL");
        return ExitCode::from(2);
    };
    let opsi = PgConnectOptions::new()
        .host(&cfg.postgres.host)
        .port(cfg.postgres.port)
        .database(&cfg.postgres.name)
        .username(DbRole::Owner.role_name())
        .password(password)
        .ssl_mode(ssl);
    let pool = match PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(opsi)
        .await
    {
        Ok(pool) => pool,
        Err(e) => {
            eprintln!("migrate: tidak bisa tersambung ke basis data: {e}");
            return ExitCode::FAILURE;
        }
    };
    match terapkan(&pool).await {
        Ok(()) => {
            println!(
                "migrate: skema mutakhir ({} migrasi dikenal)",
                MIGRATOR.iter().count()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("migrate: gagal: {e}");
            ExitCode::FAILURE
        }
    }
}
