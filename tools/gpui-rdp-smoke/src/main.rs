use std::env;
use std::process::ExitCode;

mod cli;
#[cfg(target_os = "windows")]
mod native_overlay;
#[cfg(target_os = "windows")]
mod native_overlay_ffi;
#[cfg(target_os = "windows")]
mod windows_app;

use cli::{Config, ParseOutcome, parse_args, usage};

fn main() -> ExitCode {
    let password = env::var("NAVOP_RDP_PASSWORD").ok();
    let outcome = match parse_args(env::args().skip(1), password) {
        Ok(outcome) => outcome,
        Err(error) => {
            eprintln!("argument error: {error}\n");
            eprintln!("{}", usage());
            return ExitCode::from(2);
        }
    };

    match outcome {
        ParseOutcome::Help => {
            println!("{}", usage());
            ExitCode::SUCCESS
        }
        ParseOutcome::Run(config) => run(config),
    }
}

#[cfg(not(target_os = "windows"))]
fn run(_config: Config) -> ExitCode {
    eprintln!("gpui-rdp-smoke is only supported on Windows");
    ExitCode::from(2)
}

#[cfg(target_os = "windows")]
fn run(config: Config) -> ExitCode {
    // Native child HWNDs must use the classic HWND composition path rather
    // than GPUI's DirectComposition swap-chain presentation.
    unsafe {
        env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "1");
    }
    println!("presentation: GPUI DirectComposition disabled for native child HWND hosting");
    windows_app::run(config);
    ExitCode::SUCCESS
}
