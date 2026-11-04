#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `webserver healthcheck`: probe the local admin port for container health
    // checks (no shell, curl or nc in the image).
    if args.first().map(String::as_str) == Some("healthcheck") {
        return poolrouter_webserver::downstream::probe::healthcheck_exit_code().await;
    }
    poolrouter_core::runtime::start(
        "webserver",
        poolrouter_core::config::Component::Webserver,
        &args,
    )
    .await
}
