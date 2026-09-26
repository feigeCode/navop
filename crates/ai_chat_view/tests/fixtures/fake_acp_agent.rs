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
    /// 声明 `session/load` 能力，并把 `session/new` 生成的 id 按进程区分。
    ///
    /// 用来验证「重连复用上次的 ACP 会话」：走 load 时 id 不变，走 new 时必然换一个。
    SessionReuse,
    /// 只声明 `session/resume`（不声明 `session/load`）：能接上会话，但不回放历史。
    ///
    /// 用来验证「接上了但屏幕上看不到」这一档必须被如实标记出来。
    SessionResumeOnly,
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mode = parse_mode(args.next().as_deref())?;
    // 可选：把收到的每个请求方法名追加到这个文件，供测试断言真实走的是 load 还是 new。
    let method_log = args.next();
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut pending_prompt = None;
    for line in stdin.lock().lines() {
        let message: Value = serde_json::from_str(&line?)?;
        record_method(method_log.as_deref(), &message)?;
        handle_message(mode, &message, &mut pending_prompt, &mut stdout)?;
        if mode == Mode::ExitAfterInitialize && message["method"] == "initialize" {
            break;
        }
    }
    Ok(())
}

fn record_method(log: Option<&str>, message: &Value) -> anyhow::Result<()> {
    let Some(path) = log else {
        return Ok(());
    };
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return Ok(());
    };
    // 每次 append 都开合一次：进程被 kill 时不会有半截缓冲丢掉，测试断言才可靠。
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{method}")?;
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
        "session/new" => {
            respond_result(message, json!({"sessionId": new_session_id(mode)}), stdout)
        }
        "session/load" | "session/resume" => respond_reopen_session(mode, message, stdout),
        "session/prompt" => respond_prompt(mode, message, pending_prompt, stdout),
        "session/cancel" => respond_cancel(pending_prompt, stdout),
        _ => Ok(()),
    }
}

/// 每个 `session-reuse` 进程给出不同的新会话 id：于是「走了 new」在测试里一定看得见。
///
/// 其它模式沿用固定的 `fake-session`：那些用例断言的是通知里的会话 id，
/// 换成进程相关值只会平白弄坏它们。
fn new_session_id(mode: Mode) -> String {
    match mode {
        Mode::SessionReuse => format!("fake-new-{}", std::process::id()),
        _ => "fake-session".to_string(),
    }
}

/// `session/load` / `session/resume`。
///
/// `session-reuse` 模式下：目标 id 以 `fake-missing` 开头就当它不存在，回错误——
/// 用来验证客户端会降级到新建，而不是把整次连接判死。
///
/// `session-resume-only` 模式复用同一段应答：那个模式只声明 `session/resume`，
/// 客户端也不会发 `session/load`，所以「能被复用」的语义是一样的。
fn respond_reopen_session(
    mode: Mode,
    message: &Value,
    stdout: &mut impl Write,
) -> anyhow::Result<()> {
    if !matches!(mode, Mode::SessionReuse | Mode::SessionResumeOnly) {
        return Ok(());
    }
    let target = message["params"]["sessionId"].as_str().unwrap_or("");
    if target.starts_with("fake-missing") {
        return write_json(
            json!({
                "jsonrpc": "2.0",
                "id": message["id"],
                "error": {"code": -32602, "message": "unknown session", "data": target}
            }),
            stdout,
        );
    }
    respond_result(message, json!({}), stdout)
}

fn respond_initialize(mode: Mode, message: &Value, stdout: &mut impl Write) -> anyhow::Result<()> {
    let auth_methods = if mode == Mode::AuthRequired {
        json!([{"id": "fake-login", "name": "Fake Login"}])
    } else {
        json!([])
    };
    let capabilities = match mode {
        Mode::SessionReuse => json!({"loadSession": true}),
        Mode::SessionResumeOnly => json!({"sessionCapabilities": {"resume": {}}}),
        _ => json!({}),
    };
    respond_result(
        message,
        json!({
            "protocolVersion": 1,
            "agentCapabilities": capabilities,
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
        // 会话复用用例只关心握手，不关心轮次内容。
        Mode::SessionReuse | Mode::SessionResumeOnly => {
            respond_result(message, json!({"stopReason": "end_turn"}), stdout)
        }
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
        Some("session-reuse") => Ok(Mode::SessionReuse),
        Some("session-resume-only") => Ok(Mode::SessionResumeOnly),
        other => anyhow::bail!("unsupported fake ACP mode: {other:?}"),
    }
}
