#[tokio::main]
async fn main() {
    // Guard must stay in scope to keep the log file writer alive
    let _guard = alloy::tui::logging::init();
    tracing::info!("Starting alloy {}", env!("CARGO_PKG_VERSION"));
    if let Err(e) = color_eyre::install() {
        eprintln!("Failed to install color-eyre: {}", e);
        tracing::warn!("Failed to install color-eyre handler: {}", e);
    }

    if let Err(e) = alloy::tui::show().await {
        tracing::error!("TUI error: {}", e);
    }
}
