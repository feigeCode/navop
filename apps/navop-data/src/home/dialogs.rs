//! MCP 工具集开关弹窗：只控制 data 暴露给外部代理的工具集。

use gpui::{App, IntoElement, ParentElement, Styled, Window, div};
use gpui_component::WindowExt as _;
use gpui_component::checkbox::Checkbox;

/// 打开 MCP 工具集弹窗。
pub(super) fn open_mcp_tools_dialog(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, _window, cx| {
        dialog
            .title("MCP 工具集")
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(toolset_checkbox(
                        "mcp-toolset-database",
                        "Database",
                        mcp_host::McpToolset::Database,
                        cx,
                    ))
                    .child(toolset_checkbox(
                        "mcp-toolset-redis",
                        "Redis",
                        mcp_host::McpToolset::Redis,
                        cx,
                    ))
                    .child(toolset_checkbox(
                        "mcp-toolset-mongo",
                        "MongoDB",
                        mcp_host::McpToolset::Mongo,
                        cx,
                    ))
                    .child(toolset_checkbox(
                        "mcp-toolset-connections",
                        "连接管理",
                        mcp_host::McpToolset::Connections,
                        cx,
                    )),
            )
            .confirm()
    });
}

/// 单个工具集开关。
fn toolset_checkbox(
    id: &'static str,
    label: &'static str,
    toolset: mcp_host::McpToolset,
    cx: &mut App,
) -> impl IntoElement {
    Checkbox::new(id)
        .checked(mcp_host::current_toolset_enabled(cx, toolset))
        .label(label)
        .on_click(move |checked, _, cx| {
            let _ = mcp_host::set_toolset_enabled(cx, toolset, *checked);
        })
}
