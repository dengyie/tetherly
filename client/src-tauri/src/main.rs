// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Shell binary entry; all logic lives in the library for tests.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // §147/§238: shell diagnostics. Default to a rotating file (Finder and
    // autostart launches discard stderr, which left a dead tray untraceable);
    // fall back to stderr when the log cannot be opened.
    let filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
    };
    match tetherly_shell::shelllog::open() {
        Some((_, file)) => tracing_subscriber::fmt()
            .with_env_filter(filter())
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .init(),
        None => tracing_subscriber::fmt()
            .with_env_filter(filter())
            .with_writer(std::io::stderr)
            .init(),
    }
    tetherly_shell::run()
}
