use tracing::info;

mod application;
mod config;
mod runtime;
mod widgets;

fn main() {
    // Initialize logging
    init_logging();

    info!("Starting m'Queue v{}", env!("CARGO_PKG_VERSION"));

    // Run the GTK application
    let exit_code = application::run();
    std::process::exit(exit_code);
}

fn init_logging() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr);

    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer);

    // Optional journald logging (configured at runtime in later phases)
    // Optional file logging (configured at runtime in later phases)

    registry.init();
}
