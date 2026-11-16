//! `navop.terminal` 能力面（模块 `terminal`）——以 nsh 的 nativeModule 暴露。
//!
//! ```ts
//! const term = nativeModule<{ Terminal: NavopTerminal }>("navop").Terminal;
//! await term.list();                                   // { sessions: [...] }
//! await term.read({ sessionId?, lines? });             // { text, ... }
//! await term.write({ sessionId?, text });              // { sessionId, sent, bytes }
//! await term.wait({ sessionId?, previous?, lines?, timeoutMs? }); // { changed, text, ... }
//! ```
//!
//! 底层复用 Public MCP 的终端注册表（与 AI/MCP 工具同一套会话快照与
//! 输入通道）：每个活动终端在创建时注册了 read（`recent_text`）与
//! input（`write_keys`）句柄，这里只是把它们按扩展协议的形状转发。
//! `sessionId` 缺省取注册表里第一个已连接的本地/SSH 会话。
//!
//! 权限：清单声明 `modules: ["terminal"]`；向终端写内容是高权限动作，
//! 安装时按能力面提示。宿主主动推送（cwd-changed / command-finished）还没
//! 有：页面要“跟着输出走”用 `wait` 长轮询（不需要自己写 sleep 循环）。

use serde_json::{Value, json};

use public_mcp::registry::PublicMcpRegistry;
use public_mcp::terminal_read::TerminalReadRequest;
use public_mcp::terminal_write_keys::TerminalWriteKeysRequest;
/// 主线程上读终端：解析目标会话 → Public MCP read 句柄 → `recent_text`。
pub(crate) fn read_on_main(
    session_id: Option<&str>,
    lines: usize,
    cx: &mut gpui::App,
) -> Result<Value, String> {
    let Some(registry) = terminal_view::public_mcp::registry(cx) else {
        return Err("终端注册表未初始化".to_string());
    };
    let target = resolve_target(&registry, session_id)?;
    let request = TerminalReadRequest {
        target: target.clone(),
        lines,
    };
    let result = registry
        .terminal_read(&target, request)
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "sessionId": result.target,
        "text": result.text,
        "returnedLines": result.returned_lines,
        "availableLines": result.available_lines,
        "historySize": result.history_size,
        "screenLines": result.screen_lines,
        "columns": result.columns,
        "altScreen": result.alt_screen,
        "displayOffset": result.display_offset,
        "cursorRow": result.cursor_row,
        "cursorColumn": result.cursor_column,
    }))
}
/// 在快照上加 `changed` 标志；`text` 保持原样方便页面直接对比/渲染。
pub(crate) fn with_change_flag(mut snapshot: Value, changed: bool) -> Value {
    if let Some(object) = snapshot.as_object_mut() {
        object.insert("changed".to_string(), Value::Bool(changed));
    }
    snapshot
}

/// 主线程上写终端：write_keys 通道（与 MCP 工具同一条输入路径）。
pub(crate) fn write_on_main(
    session_id: Option<&str>,
    text: &str,
    cx: &mut gpui::App,
) -> Result<Value, String> {
    let Some(registry) = terminal_view::public_mcp::registry(cx) else {
        return Err("终端注册表未初始化".to_string());
    };
    let target = resolve_target(&registry, session_id)?;
    let bytes = text.as_bytes().to_vec();
    let request = TerminalWriteKeysRequest {
        target: target.clone(),
        bytes: bytes.clone(),
    };
    let result = registry
        .terminal_write_keys(&target, request)
        .map_err(|e| e.to_string())?;
    if !result.sent {
        return Err("终端当前不接受输入（会话未连接或已锁定）".to_string());
    }
    Ok(json!({ "sessionId": target, "sent": true, "bytes": bytes.len() }))
}

/// sessionId 缺省取第一个已连接的本地/SSH 会话。
pub(crate) fn resolve_target(
    registry: &PublicMcpRegistry,
    session_id: Option<&str>,
) -> Result<String, String> {
    if let Some(id) = session_id {
        return Ok(id.to_string());
    }
    registry
        .list_sessions()
        .into_iter()
        .find(|s| s.connected)
        .map(|s| s.session_id)
        .ok_or_else(|| "没有已连接的终端会话".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_snapshot_carries_the_changed_flag() {
        let value = with_change_flag(json!({ "text": "abc", "sessionId": "1" }), true);
        assert_eq!(value["changed"], json!(true));
        assert_eq!(value["text"], json!("abc"));

        let quiet = with_change_flag(json!({ "text": "abc" }), false);
        assert_eq!(quiet["changed"], json!(false));
    }
}
