//! Binary entry point for the `rtkk` command-line tool.

#[tokio::main]
async fn main() {
    let result = rtkk::cli::run_cli().await;
    rtkk::logging::shutdown_logging();
    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
