//! 按 kind id 构造各协议的连接表单视图。
//!
//! 各协议表单只接收自己的 config 结构体、不依赖具体产品的主页实现，因此
//! 独立产品可以直接复用同一套新建/编辑表单。

use super::kinds;
use domain_api::ConnectionFormRequest;
use gpui::{AnyView, App, AppContext as _, Window};
use one_core::cloud_sync::{TeamOption, get_cached_team_options};
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionType, RemoteDesktopProtocol, StoredConnection, Workspace};
use port_forwarding_view::{PortForwardingFormWindow, PortForwardingFormWindowConfig};
use remote_desktop_view::remote_desktop_form::{
    RemoteDesktopFormWindow, RemoteDesktopFormWindowConfig,
};
use terminal_view::{
    FtpFormWindow, FtpFormWindowConfig, SerialFormWindow, SerialFormWindowConfig, SshFormWindow,
    SshFormWindowConfig, TelnetFormWindow, TelnetFormWindowConfig,
};

/// 表单公共上下文：工作区列表与可选的团队，按 kind 只读取一次。
struct FormEnvironment {
    workspaces: Vec<Workspace>,
    teams: Vec<TeamOption>,
}

/// 表单视图构造器；把 `App`/`Window` 与公共上下文收拢，避免辅助函数参数膨胀。
struct FormBuilder<'a> {
    environment: FormEnvironment,
    window: &'a mut Window,
    cx: &'a mut App,
}

impl FormBuilder<'_> {
    /// SSH / SFTP 表单。
    fn ssh(self, request: ConnectionFormRequest) -> AnyView {
        let Self {
            environment,
            window,
            cx,
        } = self;
        let config = SshFormWindowConfig {
            editing_connection: request.editing_connection,
            initial_connection: request.initial_connection,
            initial_workspace_id: request.initial_workspace_id,
            // `SshFormSavedCallback` 比 `ConnectionSavedCallback` 多一个保存后置动作参数，
            // 无法直接转发；保存后的列表刷新由宿主通知器负责。
            on_saved: None,
            workspaces: environment.workspaces,
            teams: environment.teams,
        };
        cx.new(|cx| SshFormWindow::new(config, window, cx)).into()
    }

    /// FTP 表单。
    fn ftp(self, request: ConnectionFormRequest) -> AnyView {
        let Self {
            environment,
            window,
            cx,
        } = self;
        let config = FtpFormWindowConfig {
            editing_connection: request.editing_connection,
            initial_workspace_id: request.initial_workspace_id,
            workspaces: environment.workspaces,
            teams: environment.teams,
        };
        cx.new(|cx| FtpFormWindow::new(config, window, cx)).into()
    }

    /// 串口表单。
    fn serial(self, request: ConnectionFormRequest) -> AnyView {
        let Self {
            environment,
            window,
            cx,
        } = self;
        let config = SerialFormWindowConfig {
            editing_connection: request.editing_connection,
            initial_workspace_id: request.initial_workspace_id,
            workspaces: environment.workspaces,
            teams: environment.teams,
        };
        cx.new(|cx| SerialFormWindow::new(config, window, cx))
            .into()
    }

    /// Telnet 表单。
    fn telnet(self, request: ConnectionFormRequest) -> AnyView {
        let Self {
            environment,
            window,
            cx,
        } = self;
        let config = TelnetFormWindowConfig {
            editing_connection: request.editing_connection,
            initial_workspace_id: request.initial_workspace_id,
            workspaces: environment.workspaces,
            teams: environment.teams,
        };
        cx.new(|cx| TelnetFormWindow::new(config, window, cx))
            .into()
    }

    /// 端口转发表单；跳板机只能从已有 SSH 连接里选。
    fn port_forwarding(self, request: ConnectionFormRequest) -> AnyView {
        let Self {
            environment,
            window,
            cx,
        } = self;
        let config = PortForwardingFormWindowConfig {
            editing_connection: request.editing_connection,
            initial_workspace_id: request.initial_workspace_id,
            ssh_connections: ssh_connections(cx),
            workspaces: environment.workspaces,
            teams: environment.teams,
        };
        cx.new(|cx| PortForwardingFormWindow::new(config, window, cx))
            .into()
    }

    /// RDP / VNC 表单，协议由 kind 决定。
    fn remote_desktop(
        self,
        request: ConnectionFormRequest,
        protocol: RemoteDesktopProtocol,
    ) -> AnyView {
        let Self {
            environment,
            window,
            cx,
        } = self;
        let config = RemoteDesktopFormWindowConfig {
            protocol,
            editing_connection: request.editing_connection,
            initial_workspace_id: request.initial_workspace_id,
            workspaces: environment.workspaces,
            teams: environment.teams,
        };
        cx.new(|cx| RemoteDesktopFormWindow::new(config, window, cx))
            .into()
    }
}

/// 构造请求对应的表单；未知 kind 返回 `None`，由宿主提示。
///
/// 除 SSH 外的表单结构体都没有 `initial_connection` 字段，复制连接时无法预填内容，
/// 因此这里只转发 `editing_connection`（用复制源当编辑连接会在保存时覆盖原连接）。
pub(crate) fn build(
    request: ConnectionFormRequest,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyView> {
    let builder = FormBuilder {
        environment: FormEnvironment {
            workspaces: load_workspaces(cx),
            teams: get_cached_team_options(cx),
        },
        window,
        cx,
    };
    let kind_id = request.kind_id.clone();
    match kind_id.as_ref() {
        kinds::KIND_SSH => Some(builder.ssh(request)),
        kinds::KIND_FTP => Some(builder.ftp(request)),
        kinds::KIND_SERIAL => Some(builder.serial(request)),
        kinds::KIND_TELNET => Some(builder.telnet(request)),
        kinds::KIND_PORT_FORWARDING => Some(builder.port_forwarding(request)),
        kinds::KIND_RDP => Some(builder.remote_desktop(request, RemoteDesktopProtocol::Rdp)),
        kinds::KIND_VNC => Some(builder.remote_desktop(request, RemoteDesktopProtocol::Vnc)),
        _ => None,
    }
}

/// 读取工作区列表；仓库未初始化时退化为空列表。
fn load_workspaces(cx: &App) -> Vec<Workspace> {
    mcp_host::workspace_repository(cx)
        .and_then(|repository| repository.list().ok())
        .unwrap_or_default()
}

/// 端口转发可选的跳板机：已有连接里的 SSH/SFTP。
fn ssh_connections(cx: &App) -> Vec<StoredConnection> {
    let connections = mcp_host::connection_repository(cx)
        .and_then(|repository| repository.list().ok())
        .unwrap_or_default();
    connections
        .into_iter()
        .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
        .collect()
}
