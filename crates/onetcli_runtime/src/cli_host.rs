use serde_json::{Value, json};
use tool_runtime::{ToolAdapter, ToolContext, ToolDescriptor, ToolError, ToolRegistry};

mod domain;
pub(crate) use domain::{
    run_connection_command, run_db_command, run_sftp_command, run_ssh_command,
};

const AUTOMATION_ADAPTER: ToolAdapter = ToolAdapter::FunctionCalling;

pub fn handle_command<F>(registry: F) -> Option<i32>
where
    F: Fn() -> anyhow::Result<ToolRegistry>,
{
    let command = match onetcli_cli::parse_from(std::env::args_os()) {
        Ok(Some(command)) => command,
        Ok(None) => return None,
        Err(error) => return Some(onetcli_cli::print_error(error)),
    };

    match run_cli_command(command, registry) {
        Ok(output) => {
            println!("{output}");
            Some(0)
        }
        Err(error) => {
            eprintln!("{error}");
            Some(2)
        }
    }
}

fn run_cli_command<F>(command: onetcli_cli::OnetCliCommand, registry: F) -> anyhow::Result<String>
where
    F: Fn() -> anyhow::Result<ToolRegistry>,
{
    match command {
        onetcli_cli::OnetCliCommand::Tool(command) => run_tool_command(command, registry()?),
        onetcli_cli::OnetCliCommand::Connection(command) => {
            run_connection_command(command, registry()?)
        }
        onetcli_cli::OnetCliCommand::Db(command) => run_db_command(command, registry()?),
        onetcli_cli::OnetCliCommand::Ssh(command) => run_ssh_command(command, registry()?),
        onetcli_cli::OnetCliCommand::Sftp(command) => run_sftp_command(command, registry()?),
    }
}

pub fn run_tool_command(
    command: onetcli_cli::ToolCommand,
    registry: ToolRegistry,
) -> anyhow::Result<String> {
    match command {
        onetcli_cli::ToolCommand::List { format } => list_tools(registry, format),
        onetcli_cli::ToolCommand::Schema { tool_id, format } => {
            schema_tool(&tool_id, registry, format)
        }
        onetcli_cli::ToolCommand::Call {
            tool_id,
            input,
            positional_input,
            allow_write,
            format,
        } => call_tool(
            &tool_id,
            input.or(positional_input),
            allow_write,
            registry,
            format,
        ),
    }
}

pub(super) fn run_function_tool(
    tool_id: &str,
    input: Value,
    allow_write: bool,
    registry: ToolRegistry,
    format: onetcli_cli::OutputFormat,
) -> anyhow::Result<String> {
    call_tool(
        tool_id,
        Some(input.to_string()),
        allow_write,
        registry,
        format,
    )
}

fn list_tools(
    registry: ToolRegistry,
    _format: onetcli_cli::OutputFormat,
) -> anyhow::Result<String> {
    let tools = registry
        .list(AUTOMATION_ADAPTER)
        .into_iter()
        .map(tool_summary)
        .collect::<Vec<_>>();
    Ok(serde_json::to_string_pretty(&tools)?)
}

fn schema_tool(
    tool_id: &str,
    registry: ToolRegistry,
    _format: onetcli_cli::OutputFormat,
) -> anyhow::Result<String> {
    let descriptor = registry
        .get(tool_id, AUTOMATION_ADAPTER)
        .ok_or_else(|| json_error("unknown_tool", format!("unknown tool: {tool_id}")))?;
    Ok(serde_json::to_string_pretty(&descriptor)?)
}

pub(super) fn call_tool(
    tool_id: &str,
    input: Option<String>,
    allow_write: bool,
    registry: ToolRegistry,
    _format: onetcli_cli::OutputFormat,
) -> anyhow::Result<String> {
    let input = input
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|error| json_error("invalid_json", error.to_string()))?
        .unwrap_or_else(|| json!({}));
    reject_disallowed_write(tool_id, &input, allow_write, &registry)?;
    let result = futures::executor::block_on(registry.call(
        tool_id,
        input,
        ToolContext::for_adapter(AUTOMATION_ADAPTER),
    ))
    .map_err(tool_error)?;
    Ok(serde_json::to_string_pretty(&result.structured_content)?)
}

fn reject_disallowed_write(
    tool_id: &str,
    input: &Value,
    allow_write: bool,
    registry: &ToolRegistry,
) -> anyhow::Result<()> {
    if allow_write {
        return Ok(());
    }
    let Some(annotations) = registry.call_annotations(tool_id, AUTOMATION_ADAPTER, input) else {
        return Ok(());
    };
    if annotations.read_only && !annotations.destructive {
        return Ok(());
    }
    Err(json_error(
        "write_not_allowed",
        format!("tool `{tool_id}` may write; rerun with --allow-write to permit it"),
    ))
}

fn tool_summary(tool: ToolDescriptor) -> Value {
    json!({
        "id": tool.id,
        "title": tool.title,
        "description": tool.description,
        "read_only": tool.annotations.read_only,
        "mode": tool.mode,
        "permissions": tool.permissions,
        "annotations": tool.annotations,
        "input_schema": tool.input_schema,
        "output_schema": tool.output_schema
    })
}

fn tool_error(error: ToolError) -> anyhow::Error {
    match error {
        ToolError::UnknownTool { id } => json_error("unknown_tool", format!("unknown tool: {id}")),
        ToolError::UnsupportedAdapter { id, adapter } => json_error(
            "unsupported_adapter",
            format!("tool `{id}` is not exposed for adapter {adapter:?}"),
        ),
        ToolError::Failed { message } => json_error("tool_failed", message),
    }
}

fn json_error(code: &str, message: impl Into<String>) -> anyhow::Error {
    anyhow::anyhow!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "ok": false,
            "error": {
                "code": code,
                "message": message.into()
            }
        }))
        .unwrap_or_else(|_| "{\"ok\":false}".to_string())
    )
}

#[cfg(test)]
mod tests;
