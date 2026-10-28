/// 数据库标签的打开模式，等价于原 `db_view::DatabaseOpenMode`，但不引用 `db_view`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatabaseTabMode {
    Single,
    Workspace,
}

/// 远程桌面协议，等价于原 `remote_desktop::RemoteDesktopProtocol`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteProtocol {
    Rdp,
    Vnc,
}

/// 本地终端 profile 的领域无关描述。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalProfileKind {
    System,
    PowerShell,
    Cmd,
    Wsl,
    GitBash,
    Zsh,
    Bash,
    Fish,
    Nushell,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalProfileDescriptor {
    pub kind: TerminalProfileKind,
    pub id: Option<String>,
}

/// 领域注册表能识别的“打开一个 Tab”意图。
///
/// 这里刻意不出现 `db_view::DatabaseOpenMode`、`remote_desktop::RemoteDesktopProtocol`
/// 等具体领域类型，避免契约层反向依赖实现层。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabIntent {
    Database {
        connection_id: Option<i64>,
        mode: DatabaseTabMode,
    },
    Redis {
        connection_id: i64,
    },
    Mongo {
        connection_id: i64,
    },
    Sftp {
        connection_id: i64,
    },
    Terminal {
        profile: Option<TerminalProfileDescriptor>,
    },
    RemoteDesktop {
        connection_id: i64,
        protocol: RemoteProtocol,
    },
    PortForwarding,
    KnownHosts,
}
