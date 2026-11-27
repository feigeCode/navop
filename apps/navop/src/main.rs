#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! `navop`: Full, now running the nsh shell.

fn main() {
    navop_nsh_app::run(env!("CARGO_PKG_VERSION"));
}
