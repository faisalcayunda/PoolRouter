#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("healthcheck") {
        return poolrouter_core::runtime::healthcheck().await;
    }
    poolrouter_core::runtime::spawn_ready_server();
    poolrouter_core::runtime::start("worker", poolrouter_core::config::Component::Worker, &args)
        .await
}
