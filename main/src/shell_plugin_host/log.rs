use gpui_shell::{HostError, HostModule, HostValue};

pub(super) fn log_module(
    contribution: &extension_runtime::RegisteredShellViewContribution,
) -> HostModule {
    let extension_id = contribution.extension_id.clone();
    let view_id = contribution.id.clone();
    HostModule::new("navop.log")
        .declarations(
            r#"
            export function debug(message: string): void;
            export function info(message: string): void;
            export function warn(message: string): void;
            export function error(message: string): void;
            "#,
        )
        .function(
            "debug",
            logger(extension_id.clone(), view_id.clone(), Level::Debug),
        )
        .function(
            "info",
            logger(extension_id.clone(), view_id.clone(), Level::Info),
        )
        .function(
            "warn",
            logger(extension_id.clone(), view_id.clone(), Level::Warn),
        )
        .function("error", logger(extension_id, view_id, Level::Error))
}

#[derive(Clone, Copy)]
enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

fn logger(
    extension_id: String,
    view_id: String,
    level: Level,
) -> impl Fn(&gpui_shell::HostArguments) -> Result<HostValue, HostError> {
    move |arguments| {
        let message = arguments.string(0)?;
        if message.len() > 16 * 1024 {
            return Err(HostError::new("log message exceeds 16 KiB"));
        }
        match level {
            Level::Debug => tracing::debug!(extension_id, view_id, "{message}"),
            Level::Info => tracing::info!(extension_id, view_id, "{message}"),
            Level::Warn => tracing::warn!(extension_id, view_id, "{message}"),
            Level::Error => tracing::error!(extension_id, view_id, "{message}"),
        }
        Ok(HostValue::Null)
    }
}
