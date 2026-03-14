/// Discover the GTK4 GSettings schema directory at build time via `pkg-config`
/// and embed it so the binary can add it to the search path at runtime.
///
/// This is needed on systems like NixOS where schemas are not in the
/// default `/usr/share/glib-2.0/schemas/` location and `cargo run` runs
/// unwrapped (without the environment variables that a proper package
/// wrapper would set).
fn main() {
    // Ask pkg-config where GTK4's data directory lives.
    if let Ok(output) = std::process::Command::new("pkg-config")
        .args(["--variable=prefix", "gtk4"])
        .output()
    {
        if output.status.success() {
            if let Ok(prefix) = std::string::String::from_utf8(output.stdout) {
                let prefix = prefix.trim();
                println!("cargo:rustc-env=MQ_GTK4_PREFIX={prefix}");
            }
        }
    }
}
