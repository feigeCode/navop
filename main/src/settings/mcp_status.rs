use crate::public_mcp_runtime::{PublicMcpRuntimeStatus, runtime_status};
use gpui::{App, Div, ParentElement, Styled, div};
use gpui_component::{
    ActiveTheme,
    setting::{SettingField, SettingItem},
    v_flex,
};
use public_mcp::discovery::PublicMcpMode;
use rust_i18n::t;

pub fn mcp_runtime_status_item() -> SettingItem {
    SettingItem::new(
        t!("Settings.General.Mcp.status"),
        SettingField::render(|_, _, cx| render_runtime_status(cx)),
    )
    .description(t!("Settings.General.Mcp.status_desc").to_string())
}

struct McpRuntimeStatusViewModel {
    state_key: &'static str,
    detail_lines: Vec<String>,
}

fn render_runtime_status(cx: &mut App) -> Div {
    let view = mcp_runtime_status_view_model(&runtime_status(cx));
    let detail_color = cx.theme().muted_foreground;
    v_flex()
        .gap_1()
        .child(div().text_sm().child(t!(view.state_key).to_string()))
        .children(
            view.detail_lines
                .into_iter()
                .map(|line| div().text_xs().text_color(detail_color).child(line)),
        )
}

fn mcp_runtime_status_view_model(status: &PublicMcpRuntimeStatus) -> McpRuntimeStatusViewModel {
    match status {
        PublicMcpRuntimeStatus::Disabled => McpRuntimeStatusViewModel {
            state_key: "Settings.General.Mcp.status_disabled",
            detail_lines: Vec::new(),
        },
        PublicMcpRuntimeStatus::Starting { .. } => McpRuntimeStatusViewModel {
            state_key: "Settings.General.Mcp.status_starting",
            detail_lines: Vec::new(),
        },
        PublicMcpRuntimeStatus::Running {
            bind_addr,
            mode,
            discovery_path,
            client_count,
            ..
        } => McpRuntimeStatusViewModel {
            state_key: "Settings.General.Mcp.status_running",
            detail_lines: vec![
                bind_addr.to_string(),
                public_mcp_mode_label(*mode).to_string(),
                format!("clients: {client_count}"),
                discovery_path.display().to_string(),
            ],
        },
        PublicMcpRuntimeStatus::Failed { message, .. } => McpRuntimeStatusViewModel {
            state_key: "Settings.General.Mcp.status_failed",
            detail_lines: vec![message.clone()],
        },
    }
}

fn public_mcp_mode_label(mode: PublicMcpMode) -> &'static str {
    match mode {
        PublicMcpMode::Temporary => "temporary",
        PublicMcpMode::Persistent => "persistent",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_mcp_runtime::PublicMcpRuntimeStatus;
    use public_mcp::discovery::PublicMcpMode;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::path::PathBuf;

    #[test]
    fn runtime_status_view_model_describes_disabled_running_and_failed_states() {
        let disabled = mcp_runtime_status_view_model(&PublicMcpRuntimeStatus::Disabled);
        assert_eq!("Settings.General.Mcp.status_disabled", disabled.state_key);
        assert!(disabled.detail_lines.is_empty());

        let running = mcp_runtime_status_view_model(&PublicMcpRuntimeStatus::Running {
            generation: 3,
            bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9234),
            mode: PublicMcpMode::Persistent,
            discovery_path: PathBuf::from("/tmp/public-mcp.json"),
            client_count: 2,
        });
        assert_eq!("Settings.General.Mcp.status_running", running.state_key);
        assert_eq!(
            vec![
                "127.0.0.1:9234".to_string(),
                "persistent".to_string(),
                "clients: 2".to_string(),
                "/tmp/public-mcp.json".to_string()
            ],
            running.detail_lines
        );

        let failed = mcp_runtime_status_view_model(&PublicMcpRuntimeStatus::Failed {
            generation: 4,
            message: "bind failed".to_string(),
        });
        assert_eq!("Settings.General.Mcp.status_failed", failed.state_key);
        assert_eq!(vec!["bind failed".to_string()], failed.detail_lines);
    }
}
