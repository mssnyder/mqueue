use tracing::info;

mod actions;
mod application;
mod config;
mod runtime;
mod widgets;

fn main() {
    // Ensure GSettings schemas (e.g. EmojiChooser) are discoverable.
    // On systems like NixOS, schemas live outside the default search path.
    // The build.rs records the GTK4 prefix via pkg-config so we can add it.
    ensure_gsettings_schemas();

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

/// Add the GTK4 schema directory to `XDG_DATA_DIRS` if it isn't already
/// reachable. This prevents a fatal GLib error when GTK widgets try to
/// access schemas (e.g. EmojiChooser) that aren't in the default path.
fn ensure_gsettings_schemas() {
    // If the user (or a wrapper like wrapGAppsHook4) already set the schema
    // dir, trust it.
    if std::env::var_os("GSETTINGS_SCHEMA_DIR").is_some() {
        return;
    }

    // build.rs embeds the GTK4 prefix discovered at compile time.
    let Some(prefix) = option_env!("MQ_GTK4_PREFIX") else {
        return;
    };

    // On NixOS the schemas live under a `gsettings-schemas/<name>/` subtree,
    // while on FHS systems they are directly under `<prefix>/share/`.
    // Check both layouts and add whichever exists to XDG_DATA_DIRS.
    let share = format!("{prefix}/share");
    let candidates = [
        // NixOS layout: <prefix>/share/gsettings-schemas/*/glib-2.0/schemas/
        find_nix_schema_datadir(&share),
        // FHS layout: <prefix>/share/glib-2.0/schemas/
        Some(share.clone()),
    ];

    for candidate in candidates.into_iter().flatten() {
        let schema_dir = format!("{candidate}/glib-2.0/schemas");
        if std::path::Path::new(&schema_dir).join("gschemas.compiled").exists() {
            prepend_xdg_data_dirs(&candidate);
            return;
        }
    }
}

/// On NixOS, schemas are at `<share>/gsettings-schemas/<name>/`.
/// Return the `<share>/gsettings-schemas/<name>` path if found.
fn find_nix_schema_datadir(share: &str) -> Option<String> {
    let schemas_dir = format!("{share}/gsettings-schemas");
    let dir = std::fs::read_dir(&schemas_dir).ok()?;
    for entry in dir.flatten() {
        let path = entry.path();
        if path.join("glib-2.0/schemas/gschemas.compiled").exists() {
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}

fn prepend_xdg_data_dirs(datadir: &str) {
    let current = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    if current.split(':').any(|p| p == datadir) {
        return; // Already present
    }
    let new_val = if current.is_empty() {
        // Preserve the default /usr/share fallback per XDG spec
        format!("{datadir}:/usr/local/share:/usr/share")
    } else {
        format!("{datadir}:{current}")
    };
    unsafe { std::env::set_var("XDG_DATA_DIRS", new_val) };
}
