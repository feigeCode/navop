use nsh_core::prelude::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Route {
    #[default]
    Home,
    Workspaces,
    Connections,
    Notes,
    Json,
    KnownHosts,
    Database,
    Redis,
    Mongo,
    Sftp,
    Terminal,
    PortForwarding,
    RemoteDesktop,
    Ai,
    Monitor,
    Team,
    Toolbox,
    SessionLogs,
    CredentialVault,
    Extensions,
    Settings,
}

impl Route {
    pub const ALL: [Self; 21] = [
        Self::Home,
        Self::Workspaces,
        Self::Connections,
        Self::Notes,
        Self::Json,
        Self::KnownHosts,
        Self::Database,
        Self::Redis,
        Self::Mongo,
        Self::Sftp,
        Self::Terminal,
        Self::PortForwarding,
        Self::RemoteDesktop,
        Self::Ai,
        Self::Monitor,
        Self::Team,
        Self::Toolbox,
        Self::SessionLogs,
        Self::CredentialVault,
        Self::Extensions,
        Self::Settings,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Home => "Workspace",
            Self::Workspaces => "Workspaces",
            Self::Connections => "Connections",
            Self::Notes => "Notes",
            Self::Json => "JSON",
            Self::KnownHosts => "Known Hosts",
            Self::Database => "Database",
            Self::Redis => "Redis",
            Self::Mongo => "MongoDB",
            Self::Sftp => "SFTP",
            Self::Terminal => "Terminal",
            Self::PortForwarding => "Port Forwarding",
            Self::RemoteDesktop => "Remote Desktop",
            Self::Ai => "AI",
            Self::Monitor => "Host Monitor",
            Self::Team => "Team",
            Self::Toolbox => "Toolbox",
            Self::SessionLogs => "Session Logs",
            Self::CredentialVault => "Credentials",
            Self::Extensions => "Extensions",
            Self::Settings => "Settings",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Home => "Overview of the nsh workbench and migration status",
            Self::Workspaces => "Organize saved connections into local workspaces",
            Self::Connections => "Saved database, SSH and middleware connections",
            Self::Notes => "Browse and edit the local notebook",
            Self::Json => "Format and inspect JSON without leaving Navop",
            Self::KnownHosts => "Review trusted SSH host keys",
            Self::Database => "Database workspace",
            Self::Redis => "Redis workspace",
            Self::Mongo => "MongoDB workspace",
            Self::Sftp => "SFTP workspace",
            Self::Terminal => "Terminal workspace",
            Self::PortForwarding => "Local, remote and dynamic SSH tunnels",
            Self::RemoteDesktop => "RDP and VNC sessions",
            Self::Ai => "AI conversations and tools",
            Self::Monitor => "Live CPU, memory, disk and network metrics for SSH hosts",
            Self::Team => "Cached team membership, roles and encryption keys",
            Self::Toolbox => "Shared tools and migrated workspaces",
            Self::SessionLogs => "Browse terminal session history",
            Self::CredentialVault => "Stored usernames, passwords and SSH keys",
            Self::Extensions => "Installed nsh extension apps",
            Self::Settings => "Application and nsh migration information",
        }
    }

    pub const fn group(self) -> Group {
        match self {
            Self::Home | Self::Workspaces | Self::Connections => Group::Workspace,
            Self::Notes | Self::Json | Self::KnownHosts | Self::Toolbox | Self::SessionLogs => {
                Group::Tools
            }
            Self::Database | Self::Redis | Self::Mongo => Group::Data,
            Self::Sftp
            | Self::Terminal
            | Self::PortForwarding
            | Self::RemoteDesktop
            | Self::Monitor => Group::Remote,
            Self::Ai | Self::Team => Group::Intelligence,
            Self::CredentialVault | Self::Extensions | Self::Settings => Group::System,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Workspace,
    Tools,
    Data,
    Remote,
    Intelligence,
    System,
}

impl Group {
    pub const ALL: [Self; 6] = [
        Self::Workspace,
        Self::Tools,
        Self::Data,
        Self::Remote,
        Self::Intelligence,
        Self::System,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Workspace => "WORKSPACE",
            Self::Tools => "TOOLS",
            Self::Data => "DATA",
            Self::Remote => "REMOTE",
            Self::Intelligence => "INTELLIGENCE",
            Self::System => "SYSTEM",
        }
    }

    pub fn routes(self) -> impl Iterator<Item = Route> {
        Route::ALL
            .into_iter()
            .filter(move |route| route.group() == self)
    }
}

fn current_route() -> Signal<Route> {
    global::<Route>()
}

#[derive(Clone, Default, PartialEq, Eq)]
struct ConnectionRequest {
    route: Option<Route>,
    connection_id: Option<i64>,
}

fn connection_request() -> Signal<ConnectionRequest> {
    global::<ConnectionRequest>()
}

#[derive(Clone, Default, PartialEq, Eq)]
struct ExtensionRequest {
    extension_id: Option<String>,
    route: Option<String>,
}

fn extension_request() -> Signal<ExtensionRequest> {
    global::<ExtensionRequest>()
}

pub(crate) fn current() -> Route {
    current_route().with(|route| *route)
}

pub(crate) fn navigate(route: Route) {
    connection_request().set(ConnectionRequest::default());
    extension_request().set(ExtensionRequest::default());
    current_route().set(route);
}

pub(crate) fn open_connection(route: Route, connection_id: i64) {
    connection_request().set(ConnectionRequest {
        route: Some(route),
        connection_id: Some(connection_id),
    });
    current_route().set(route);
}

pub(crate) fn take_connection_request(route: Route) -> Option<i64> {
    let request = connection_request().get_untracked();
    if request.route != Some(route) {
        return None;
    }
    connection_request().set(ConnectionRequest::default());
    request.connection_id
}

pub(crate) fn open_extension(extension_id: impl Into<String>, route: Option<&str>) {
    extension_request().set(ExtensionRequest {
        extension_id: Some(extension_id.into()),
        route: route.map(str::to_owned),
    });
    current_route().set(Route::Extensions);
}

pub(crate) fn take_extension_request() -> Option<(String, Option<String>)> {
    let request = extension_request().get_untracked();
    if request.extension_id.is_none() {
        return None;
    }
    extension_request().set(ExtensionRequest::default());
    request
        .extension_id
        .map(|extension_id| (extension_id, request.route))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_has_one_group_and_a_unique_label() {
        let mut labels = std::collections::HashSet::new();
        for route in Route::ALL {
            assert!(labels.insert(route.label()));
            assert!(
                Group::ALL
                    .into_iter()
                    .any(|group| group.routes().any(|candidate| candidate == route))
            );
        }
    }
}
