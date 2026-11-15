// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Shell binary entry; all logic lives in the library for tests.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tetherly_shell::run()
}
