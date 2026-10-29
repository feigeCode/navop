//! MCP 工具集开关弹窗：只控制 shell 暴露给外部代理的工具集。

use gpui::{App, ParentElement, Styled, Window, div};
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
                    .child(
                        Checkbox::new("mcp-toolset-terminal")
                            .checked(mcp_host::current_toolset_enabled(
                                cx,
                                mcp_host::McpToolset::Terminal,
                            ))
                            .label("终端")
                            .on_click(|checked, _, cx| {
                                let _ = mcp_host::set_toolset_enabled(
                                    cx,
                                    mcp_host::McpToolset::Terminal,
                                    *checked,
                                );
                            }),
                    )
                    .child(
                        Checkbox::new("mcp-toolset-sftp")
                            .checked(mcp_host::current_toolset_enabled(
                                cx,
                                mcp_host::McpToolset::Sftp,
                            ))
                            .label("SFTP")
                            .on_click(|checked, _, cx| {
                                let _ = mcp_host::set_toolset_enabled(
                                    cx,
                                    mcp_host::McpToolset::Sftp,
                                    *checked,
                                );
                            }),
                    )
                    .child(
                        Checkbox::new("mcp-toolset-connections")
                            .checked(mcp_host::current_toolset_enabled(
                                cx,
                                mcp_host::McpToolset::Connections,
                            ))
                            .label("连接管理")
                            .on_click(|checked, _, cx| {
                                let _ = mcp_host::set_toolset_enabled(
                                    cx,
                                    mcp_host::McpToolset::Connections,
                                    *checked,
                                );
                            }),
                    ),
            )
            .confirm()
    });
}
