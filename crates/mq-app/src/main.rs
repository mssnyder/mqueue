use tracing::info;

mod actions;
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

    let config = mq_core::config::AppConfig::load().unwrap_or_default();

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.logging.level));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr);

    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer);

    // Optional journald logging
    if config.logging.journald_enabled {
        match tracing_journald::layer() {
            Ok(journald_layer) => {
                registry.with(journald_layer).init();
                return;
            }
            Err(e) => {
                eprintln!("Warning: Failed to initialize journald logging: {e}");
            }
        }
    }

    // Optional file logging
    if config.logging.file_enabled {
        let log_dir = config
            .logging
            .file_path
            .clone()
            .unwrap_or_else(|| mq_core::config::AppConfig::data_dir().join("logs"));
        if let Ok(()) = std::fs::create_dir_all(&log_dir) {
            let file_appender = tracing_appender::rolling::daily(&log_dir, "mq-mail.log");
            let file_layer = tracing_subscriber::fmt::layer()
                .with_writer(file_appender)
                .with_ansi(false);
            registry.with(file_layer).init();
            return;
        }
    }

    registry.init();
}
