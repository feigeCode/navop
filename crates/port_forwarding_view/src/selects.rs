use gpui::SharedString;
use gpui_component::select::SelectItem;
use one_core::cloud_sync::TeamOption;
use one_core::storage::{PortForwardingKind, StoredConnection, Workspace};
use rust_i18n::t;

#[derive(Clone, Default, PartialEq)]
pub(super) struct WorkspaceSelectItem {
    id: Option<i64>,
    name: String,
}

impl WorkspaceSelectItem {
    pub(super) fn none() -> Self {
        Self {
            id: None,
            name: t!("Common.none").to_string(),
        }
    }

    pub(super) fn from_workspace(workspace: &Workspace) -> Self {
        Self {
            id: workspace.id,
            name: workspace.name.clone(),
        }
    }
}

impl SelectItem for WorkspaceSelectItem {
    type Value = Option<i64>;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

#[derive(Clone, Default, PartialEq)]
pub(super) struct TeamSelectItem {
    id: Option<String>,
    name: String,
}

impl TeamSelectItem {
    pub(super) fn personal() -> Self {
        Self {
            id: None,
            name: t!("TeamSync.personal").to_string(),
        }
    }

    pub(super) fn from_team(team: &TeamOption) -> Self {
        Self {
            id: Some(team.id.clone()),
            name: team.name.clone(),
        }
    }
}

impl SelectItem for TeamSelectItem {
    type Value = Option<String>;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

#[derive(Clone, PartialEq)]
pub(super) struct SshConnectionSelectItem {
    id: i64,
    name: String,
}

impl SshConnectionSelectItem {
    pub(super) fn from_connection(connection: &StoredConnection) -> Option<Self> {
        let id = connection.id?;
        let host = connection.to_ssh_params().ok().map(|params| params.host);
        let name = match host.as_deref().filter(|host| !host.trim().is_empty()) {
            Some(host) => format!("{} ({})", connection.name, host),
            None => connection.name.clone(),
        };
        Some(Self { id, name })
    }
}

impl SelectItem for SshConnectionSelectItem {
    type Value = i64;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

#[derive(Clone, PartialEq)]
pub(super) struct ForwardingKindSelectItem {
    kind: PortForwardingKind,
}

impl ForwardingKindSelectItem {
    pub(super) fn all() -> Vec<Self> {
        vec![
            Self {
                kind: PortForwardingKind::Local,
            },
            Self {
                kind: PortForwardingKind::Dynamic,
            },
        ]
    }
}

impl SelectItem for ForwardingKindSelectItem {
    type Value = PortForwardingKind;

    fn title(&self) -> SharedString {
        self.kind.label().into()
    }

    fn value(&self) -> &Self::Value {
        &self.kind
    }
}
