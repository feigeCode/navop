use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Text,
    Empty,
    AuthRequired,
    PromptError,
    PromptHang,
    Permission,
    ExitAfterInitialize,
}

fn main() -> anyhow::Result<()> {
    let mode = parse_mode(std::env::args().nth(1).as_deref())?;
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut pending_prompt = None;
    for line in stdin.lock().lines() {
        let message: Value = serde_json::from_str(&line?)?;
        handle_message(mode, &message, &mut pending_prompt, &mut stdout)?;
        if mode == Mode::ExitAfterInitialize && message["method"] == "initialize" {
            break;
        }
    }
    Ok(())
}

fn handle_message(
    mode: Mode,
    message: &Value,
    pending_prompt: &mut Option<Value>,
    stdout: &mut impl Write,
) -> anyhow::Result<()> {
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    if mode == Mode::Permission && method.is_empty() && message["id"] == "permission-1" {
        return respond_permission_outcome(message, pending_prompt, stdout);
    }
    match method {
        "initialize" => respond_initialize(mode, message, stdout),
        "authenticate" => respond_result(message, json!({}), stdout),
        "session/new" => respond_result(message, json!({"sessionId": "fake-session"}), stdout),
        "session/prompt" => respond_prompt(mode, message, pending_prompt, stdout),
        "session/cancel" => respond_cancel(pending_prompt, stdout),
        _ => Ok(()),
    }
}

fn respond_initialize(mode: Mode, message: &Value, stdout: &mut impl Write) -> anyhow::Result<()> {
    let auth_methods = if mode == Mode::AuthRequired {
        json!([{"id": "fake-login", "name": "Fake Login"}])
    } else {
        json!([])
    };
    respond_result(
        message,
        json!({
            "protocolVersion": 1,
            "agentCapabilities": {},
            "authMethods": auth_methods,
            "agentInfo": {"name": "fake-acp-agent", "version": "1"}
        }),
        stdout,
    )
}

fn respond_prompt(
    mode: Mode,
    message: &Value,
    pending_prompt: &mut Option<Value>,
    stdout: &mut impl Write,
) -> anyhow::Result<()> {
    match mode {
        Mode::Text | Mode::AuthRequired => {
            write_json(
                json!({
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {
                        "sessionId": "fake-session",
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": {"type": "text", "text": "fake response"}
                        }
                    }
                }),
                stdout,
            )?;
            respond_result(message, json!({"stopReason": "end_turn"}), stdout)
        }
        Mode::Empty => respond_result(message, json!({"stopReason": "end_turn"}), stdout),
        Mode::PromptError => respond_error(message, stdout),
        Mode::PromptHang => {
            *pending_prompt = message.get("id").cloned();
            Ok(())
        }
        Mode::Permission => {
            *pending_prompt = message.get("id").cloned();
            write_json(
                json!({
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {
                        "sessionId": "fake-session",
                        "update": {
                            "sessionUpdate": "tool_call",
                            "toolCallId": "fake-call",
                            "title": "Write file",
                            "status": "pending",
                            "rawInput": {"path": "/tmp/acp-permission.txt"}
                        }
                    }
                }),
                stdout,
            )?;
            write_json(
                json!({
                    "jsonrpc": "2.0",
                    "id": "permission-1",
                    "method": "session/request_permission",
                    "params": {
                        "sessionId": "fake-session",
                        "toolCall": {
                            "toolCallId": "fake-call",
                            "title": "Write file",
                            "status": "pending",
                            "rawInput": {"path": "/tmp/acp-permission.txt"}
                        },
                        "options": [
                            {"optionId": "reject-once", "name": "Reject", "kind": "reject_once"},
                            {"optionId": "allow-once", "name": "Allow once", "kind": "allow_once"}
                        ]
                    }
                }),
                stdout,
            )
        }
        Mode::ExitAfterInitialize => Ok(()),
    }
}

fn respond_permission_outcome(
    message: &Value,
    pending_prompt: &mut Option<Value>,
    stdout: &mut impl Write,
) -> anyhow::Result<()> {
    let option_id = message["result"]["outcome"]["optionId"]
        .as_str()
        .unwrap_or("cancelled");
    write_json(
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "fake-session",
                "update": {
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "fake-call",
                    "status": "completed",
                    "rawOutput": {"permissionOptionId": option_id}
                }
            }
        }),
        stdout,
    )?;
    write_json(
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "fake-session",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": format!("permission:{option_id}")}
                }
            }
        }),
        stdout,
    )?;
    let Some(prompt_id) = pending_prompt.take() else {
        return Ok(());
    };
    write_json(
        json!({
            "jsonrpc": "2.0",
            "id": prompt_id,
            "result": {"stopReason": "end_turn"}
        }),
        stdout,
    )
}

fn respond_error(message: &Value, stdout: &mut impl Write) -> anyhow::Result<()> {
    write_json(
        json!({
            "jsonrpc": "2.0",
            "id": message["id"],
            "error": {
                "code": -32603,
                "message": "Internal error",
                "data": {
                    "message": "Invalid API key",
                    "provider": {"httpStatusCode": 401}
                }
            }
        }),
        stdout,
    )
}

fn respond_cancel(
    pending_prompt: &mut Option<Value>,
    stdout: &mut impl Write,
) -> anyhow::Result<()> {
    let Some(id) = pending_prompt.take() else {
        return Ok(());
    };
    write_json(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"stopReason": "cancelled"}
        }),
        stdout,
    )
}

fn respond_result(message: &Value, result: Value, stdout: &mut impl Write) -> anyhow::Result<()> {
    write_json(
        json!({"jsonrpc": "2.0", "id": message["id"], "result": result}),
        stdout,
    )
}

fn write_json(value: Value, stdout: &mut impl Write) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *stdout, &value)?;
    writeln!(stdout)?;
    stdout.flush()?;
    Ok(())
}

fn parse_mode(value: Option<&str>) -> anyhow::Result<Mode> {
    match value {
        Some("text") => Ok(Mode::Text),
        Some("empty") => Ok(Mode::Empty),
        Some("auth-required") => Ok(Mode::AuthRequired),
        Some("prompt-error") => Ok(Mode::PromptError),
        Some("prompt-hang") => Ok(Mode::PromptHang),
        Some("permission") => Ok(Mode::Permission),
        Some("exit-after-initialize") => Ok(Mode::ExitAfterInitialize),
        other => anyhow::bail!("unsupported fake ACP mode: {other:?}"),
    }
}
