use gpui::{App, AsyncApp, Global};
use one_core::gpui_tokio::Tokio;
use public_mcp::discovery::{PublicMcpMode, public_mcp_discovery_path, remove_discovery};
use public_mcp::permissions::PermissionMode;
use public_mcp::runtime::PublicMcpRuntime;

pub struct GlobalPublicMcpRuntime(pub Option<PublicMcpRuntime>);

impl Global for GlobalPublicMcpRuntime {}

impl Drop for GlobalPublicMcpRuntime {
    fn drop(&mut self) {
        if self.0.is_some() {
            tracing::debug!("Public MCP runtime stopped");
        }
    }
}

pub fn init(cx: &mut App) {
    cx.set_global(GlobalPublicMcpRuntime(None));

    let discovery_path = public_mcp_discovery_path();
    let _ = remove_discovery(&discovery_path);

    if !public_mcp_enabled_from_env() {
        return;
    }

    let Some(registry) = terminal_view::public_mcp::registry(cx) else {
        tracing::warn!("Public MCP registry is not initialized");
        return;
    };
    let permission_mode = permission_mode_from_env();
    let task = Tokio::spawn_result(cx, async move {
        PublicMcpRuntime::start_terminal_mcp(
            registry,
            PublicMcpMode::Temporary,
            permission_mode,
        )
        .await
    });

    cx.spawn(async move |cx: &mut AsyncApp| {
        match task.await {
            Ok(runtime) => {
                let bind_addr = runtime.bind_addr();
                let discovery_path = runtime.discovery_path().clone();
                let _ = cx.update(move |cx| {
                    cx.set_global(GlobalPublicMcpRuntime(Some(runtime)));
                });
                tracing::info!(
                    bind_addr = %bind_addr,
                    discovery_path = %discovery_path.display(),
                    "Public MCP runtime started"
                );
            }
            Err(error) => {
                tracing::warn!(error = %error, "Failed to start Public MCP runtime");
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}

fn public_mcp_enabled_from_env() -> bool {
    std::env::var("ONETCLI_PUBLIC_MCP")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn permission_mode_from_env() -> PermissionMode {
    match std::env::var("ONETCLI_PUBLIC_MCP_PERMISSION")
        .unwrap_or_else(|_| "deny".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "allow" => PermissionMode::Allow,
        "ask" => PermissionMode::Ask,
        _ => PermissionMode::Deny,
    }
}
