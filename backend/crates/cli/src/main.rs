//! Titik masuk `backend-cli` (ADR-0003 §3.6c). Dijalankan lewat `podman exec` di container backend,
//! memakai role basis data `pr_admin_api`, bukan `poolrouter_owner`.

use std::process::ExitCode;
use std::time::Duration;

use poolrouter_cli::{BANTUAN, Konteks, PasswordTerminal, Perintah, jalankan, urai};
use poolrouter_core::config::{Component, Config, DbRole};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let perintah = match urai(&args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("backend-cli: {e}");
            return ExitCode::from(2);
        }
    };
    if perintah == Perintah::Bantuan {
        println!("{BANTUAN}");
        return ExitCode::SUCCESS;
    }

    // Konfigurasi yang sama dengan komponen lain: secret hanya lewat `<NAMA>_FILE`, dan berkas
    // password seed ditolak di luar dev/ci.
    let cfg = match Config::from_env(Component::Cli) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("backend-cli: {e}");
            return ExitCode::from(2);
        }
    };
    let Some(password_db) = cfg.postgres.password(DbRole::AdminApi) else {
        eprintln!("backend-cli: password role pr_admin_api tidak tersedia");
        return ExitCode::from(2);
    };
    let Ok(ssl) = cfg.postgres.sslmode.parse::<PgSslMode>() else {
        eprintln!("backend-cli: POOLROUTER_DB_SSLMODE bukan mode ssl PostgreSQL");
        return ExitCode::from(2);
    };
    let opsi = PgConnectOptions::new()
        .host(&cfg.postgres.host)
        .port(cfg.postgres.port)
        .database(&cfg.postgres.name)
        .username(DbRole::AdminApi.role_name())
        .password(password_db)
        .ssl_mode(ssl);
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .connect_lazy_with(opsi);

    let konteks = Konteks {
        pool,
        env: cfg.env,
        seed_password: cfg.seed_password,
        sekarang: chrono::Utc::now(),
    };
    let mut keluaran = std::io::stdout();
    match jalankan(perintah, &konteks, &mut PasswordTerminal, &mut keluaran).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("backend-cli: {e}");
            ExitCode::from(e.kode_keluar())
        }
    }
}
