use poolrouter_core::config::{Component, Config};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `webserver healthcheck`: probe the local admin port for container health
    // checks (no shell, curl or nc in the image).
    if args.first().map(String::as_str) == Some("healthcheck") {
        return poolrouter_webserver::downstream::probe::healthcheck_exit_code().await;
    }
    // `--check` keeps the smoke-test behaviour of the shared bootstrap.
    if args.iter().any(|a| a == "--check") {
        return poolrouter_core::runtime::start("webserver", Component::Webserver, &args).await;
    }
    match Config::from_env(Component::Webserver) {
        Ok(cfg) => poolrouter_webserver::downstream::server::run(cfg).await,
        Err(err) => {
            poolrouter_core::runtime::init_tracing();
            tracing::error!(component = "webserver", error = %err, "configuration rejected");
            std::process::ExitCode::from(2)
        }
    }
}
