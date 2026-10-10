#[tokio::main]
async fn main() {
    if let Err(e) = rtkk::cli::run_cli().await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
