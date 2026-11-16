// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Shell binary entry; all logic lives in the library for tests.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // §147: shell diagnostics to stderr — the daemon-side takeover chain is
    // invisible without it (no file sink yet; stderr is harmless when
    // launched by Finder/autostart and capturable when launched from CLI).
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    tetherly_shell::run()
}
