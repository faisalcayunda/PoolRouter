#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    poolrouter_core::runtime::run_skeleton("controller", &args).await;
}
