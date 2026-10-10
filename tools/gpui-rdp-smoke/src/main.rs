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
    // The smoke tool exercises two presentation paths, selected by
    // `SMOKE_RDP_COMPOSE`:
    //
    //   off (default)  classic child HWND — needs GPUI's DirectComposition off
    //   early / late   the composition path Navop uses — needs it on
    //
    // GPUI's Windows platform reads `GPUI_DISABLE_DIRECT_COMPOSITION` exactly once,
    // while it builds its platform singleton, so the decision has to be made here,
    // before `windows_app::run` opens the window. Turning it off unconditionally —
    // as this tool used to — makes every compose mode silently fall back to the
    // classic child window, so the composition path could never be tested.
    let compose = windows_app::compose_mode();
    unsafe {
        if compose == windows_app::ComposeMode::Off {
            // Native child HWNDs must use the classic HWND composition path rather
            // than GPUI's DirectComposition swap-chain presentation.
            env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "1");
        }
        // Keep the native presentation stage traces so the smoke tool still
        // produces the diagnostic output its README workflow relies on.
        env::set_var("NAVOP_REMOTE_DESKTOP_DIAGNOSTICS", "1");
    }
    println!(
        "presentation: compose_mode={compose:?} direct_composition={}",
        if compose == windows_app::ComposeMode::Off {
            "disabled"
        } else {
            "enabled"
        }
    );
    windows_app::run(config);
    ExitCode::SUCCESS
}
