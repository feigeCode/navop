use crate::home_tab::HomePage;
use gpui::{Context, Window};
use one_core::storage::{ConnectionType, StoredConnection, Workspace};
use one_core::tab_container::TabOpenMode;
use remote_desktop::RemoteDesktopProtocol;

pub(crate) trait ConnectionOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    );
}

pub(crate) fn build_connection_open_strategy(
    connection: StoredConnection,
    workspace: Option<Workspace>,
) -> Box<dyn ConnectionOpenStrategy> {
    match connection.connection_type {
        ConnectionType::SshSftp => Box::new(SshOpenStrategy { connection }),
        ConnectionType::Database => Box::new(DatabaseOpenStrategy {
            connection,
            workspace,
        }),
        ConnectionType::Redis => Box::new(RedisOpenStrategy {
            connection,
            workspace,
        }),
        ConnectionType::MongoDB => Box::new(MongoOpenStrategy {
            connection,
            workspace,
        }),
        ConnectionType::Serial => Box::new(SerialOpenStrategy { connection }),
        ConnectionType::PortForwarding => Box::new(PortForwardingOpenStrategy { connection }),
        ConnectionType::Rdp => Box::new(RemoteDesktopOpenStrategy {
            connection,
            protocol: RemoteDesktopProtocol::Rdp,
        }),
        ConnectionType::Vnc => Box::new(RemoteDesktopOpenStrategy {
            connection,
            protocol: RemoteDesktopProtocol::Vnc,
        }),
        _ => Box::new(NoopOpenStrategy),
    }
}

struct SshOpenStrategy {
    connection: StoredConnection,
}

impl ConnectionOpenStrategy for SshOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        home.open_ssh_terminal_with_mode(self.connection, mode, window, cx);
    }
}

struct DatabaseOpenStrategy {
    connection: StoredConnection,
    workspace: Option<Workspace>,
}

impl ConnectionOpenStrategy for DatabaseOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        let DatabaseOpenStrategy {
            connection,
            workspace,
        } = *self;
        extension_runtime::database_driver_install::open_database_connection_with_driver_guard(
            home, connection, workspace, mode, window, cx,
        );
    }
}

impl extension_runtime::remote_desktop_provider_install::RemoteDesktopConnectionOpener
    for HomePage
{
    fn open_remote_desktop_connection(
        &mut self,
        connection: &StoredConnection,
        protocol: RemoteDesktopProtocol,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_remote_desktop_with_mode(connection.clone(), protocol, mode, window, cx);
    }
}

impl extension_runtime::database_driver_install::DatabaseDriverConnectionOpener for HomePage {
    fn open_database_connection(
        &mut self,
        connection: &StoredConnection,
        workspace: Option<Workspace>,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_item_to_tab_with_mode(connection, workspace, mode, window, cx);
    }
}

struct RedisOpenStrategy {
    connection: StoredConnection,
    workspace: Option<Workspace>,
}

impl ConnectionOpenStrategy for RedisOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        let RedisOpenStrategy {
            connection,
            workspace,
        } = *self;
        home.open_redis_tab_with_mode(connection, workspace, mode, window, cx);
    }
}

struct MongoOpenStrategy {
    connection: StoredConnection,
    workspace: Option<Workspace>,
}

impl ConnectionOpenStrategy for MongoOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        let MongoOpenStrategy {
            connection,
            workspace,
        } = *self;
        let connection_name = connection.name.clone();
        let requirement = extension_runtime::database_driver_install::required_native_driver(
            "mongodb",
            extension_runtime::database_driver_install::NativeDriverBackend::Ipc {
                driver_id: mongodb_runtime::DEFAULT_MONGODB_MODERN_DRIVER_ID.to_string(),
            },
        );
        extension_runtime::database_driver_install::open_native_driver_connection_with_guard(
            home,
            requirement,
            connection_name,
            window,
            cx,
            move |home, window, cx| {
                home.open_mongodb_tab_with_mode(connection, workspace, mode, window, cx);
            },
        );
    }
}

struct NoopOpenStrategy;

struct SerialOpenStrategy {
    connection: StoredConnection,
}

impl ConnectionOpenStrategy for SerialOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        home.open_serial_terminal_with_mode(self.connection, mode, window, cx);
    }
}

struct PortForwardingOpenStrategy {
    connection: StoredConnection,
}

impl ConnectionOpenStrategy for PortForwardingOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        _mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        home.open_port_forwarding_tab(self.connection, _mode, window, cx);
    }
}

struct RemoteDesktopOpenStrategy {
    connection: StoredConnection,
    protocol: RemoteDesktopProtocol,
}

impl ConnectionOpenStrategy for RemoteDesktopOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        let RemoteDesktopOpenStrategy {
            connection,
            protocol,
        } = *self;
        extension_runtime::remote_desktop_provider_install::open_remote_desktop_connection_with_provider_guard(
            home, connection, protocol, mode, window, cx,
        );
    }
}

impl ConnectionOpenStrategy for NoopOpenStrategy {
    fn open(
        self: Box<Self>,
        _home: &mut HomePage,
        _mode: TabOpenMode,
        _window: &mut Window,
        _cx: &mut Context<HomePage>,
    ) {
    }
}
