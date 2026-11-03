#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    poolrouter_core::runtime::start("worker", poolrouter_core::config::Component::Worker, &args)
        .await
}
