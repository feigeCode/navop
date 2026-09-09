mod fields;
mod render;
mod schema;
mod storage;

use std::collections::HashMap;

use connection_form::declarative::DeclarativeForm;
use gpui::{App, AppContext, Context, Entity, FocusHandle, Window};
use gpui_component::{
    input::InputState,
    select::{SelectItem, SelectState},
};
use one_core::{
    cloud_sync::TeamOption,
    connection_notifier::emit_connection_event,
    storage::{ConnectionRepository, GlobalStorageState, StoredConnection, Workspace},
};

use self::{
    fields::{create_input, create_name_input, create_workspace_select, optional_input_text},
    schema::declarative_config,
    storage::{ExtensionConnectionDraft, build_connection, persist_connection},
};
use crate::universal_plugins::GlobalUniversalPluginService;

pub(crate) struct ExtensionConnectionFormConfig {
    pub contribution: extension_runtime::RegisteredResourceConnectionContribution,
    pub editing_connection: Option<StoredConnection>,
    pub workspaces: Vec<Workspace>,
    pub teams: Vec<TeamOption>,
}

#[derive(Clone)]
pub(super) struct WorkspaceItem {
    id: Option<i64>,
    label: String,
}

impl SelectItem for WorkspaceItem {
    type Value = Option<i64>;

    fn title(&self) -> gpui::SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

pub(crate) struct ExtensionConnectionForm {
    pub(super) contribution: extension_runtime::RegisteredResourceConnectionContribution,
    pub(super) editing_connection: Option<StoredConnection>,
    pub(super) name: Entity<InputState>,
    pub(super) fields: Entity<DeclarativeForm>,
    pub(super) workspace: Entity<SelectState<Vec<WorkspaceItem>>>,
    pub(super) team: Entity<SelectState<Vec<connection_form::team::TeamSelectItem>>>,
    pub(super) remark: Entity<InputState>,
    pub(super) sync_enabled: Entity<bool>,
    pub(super) test_result: Entity<Option<Result<(), String>>>,
    pub(super) is_testing: Entity<bool>,
    pub(super) focus_handle: FocusHandle,
}

impl ExtensionConnectionForm {
    pub(crate) fn new(
        config: ExtensionConnectionFormConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_params = config
            .editing_connection
            .as_ref()
            .and_then(|connection| connection.to_extension_params().ok());
        let initial_config = initial_params
            .as_ref()
            .map(|params| params.config.clone())
            .unwrap_or_default();
        let name_value = config
            .editing_connection
            .as_ref()
            .map(|connection| connection.name.clone())
            .unwrap_or_else(|| config.contribution.label.clone());
        let name = create_name_input(name_value, window, cx);
        let form_config = declarative_config(&config.contribution.form);
        let fields = cx.new(|cx| DeclarativeForm::new(form_config, &initial_config, window, cx));
        let selected_workspace = config
            .editing_connection
            .as_ref()
            .and_then(|connection| connection.workspace_id);
        let workspace = create_workspace_select(&config.workspaces, selected_workspace, window, cx);
        let team = connection_form::team::create_team_select(
            &config.teams,
            config
                .editing_connection
                .as_ref()
                .and_then(|connection| connection.team_id.as_deref()),
            window,
            cx,
        );
        let remark = create_input(
            config
                .editing_connection
                .as_ref()
                .and_then(|connection| connection.remark.clone())
                .unwrap_or_default(),
            "Optional note",
            window,
            cx,
        );
        let sync_enabled = config
            .editing_connection
            .as_ref()
            .map(|connection| connection.sync_enabled)
            .unwrap_or(true);
        Self {
            contribution: config.contribution,
            editing_connection: config.editing_connection,
            name,
            fields,
            workspace,
            team,
            remark,
            sync_enabled: cx.new(|_| sync_enabled),
            test_result: cx.new(|_| None),
            is_testing: cx.new(|_| false),
            focus_handle: cx.focus_handle(),
        }
    }

    fn draft(
        &self,
        cx: &App,
    ) -> Result<
        (
            serde_json::Map<String, serde_json::Value>,
            HashMap<String, String>,
        ),
        String,
    > {
        let existing = self
            .editing_connection
            .as_ref()
            .and_then(|connection| connection.to_extension_params().ok())
            .map(|params| params.secrets)
            .unwrap_or_default();
        let cleared = self.fields.read(cx).cleared_secret_ids();
        let preserved = existing
            .keys()
            .filter(|field| !cleared.contains(*field))
            .cloned()
            .collect();
        self.fields
            .read(cx)
            .collect_with_preserved_secrets(cx, &preserved)
    }

    fn test_draft(
        &self,
        cx: &App,
    ) -> Result<
        (
            serde_json::Map<String, serde_json::Value>,
            HashMap<String, String>,
        ),
        String,
    > {
        let (config, mut secrets) = self.draft(cx)?;
        let visible = self.fields.read(cx).visible_secret_ids(cx);
        let cleared = self.fields.read(cx).cleared_secret_ids();
        let existing = self
            .editing_connection
            .as_ref()
            .and_then(|connection| connection.to_extension_params().ok())
            .map(|params| params.secrets)
            .unwrap_or_default();
        for (field, value) in existing {
            if visible.contains(&field) && !cleared.contains(&field) {
                secrets.entry(field).or_insert(value);
            }
        }
        Ok((config, secrets))
    }

    pub(super) fn on_test(&mut self, cx: &mut Context<Self>) {
        let (config, secrets) = match self.test_draft(cx) {
            Ok(draft) => draft,
            Err(error) => {
                self.set_error(error, cx);
                return;
            }
        };
        let contribution = self.contribution.clone();
        let service = cx.global::<GlobalUniversalPluginService>().service();
        self.is_testing.update(cx, |testing, cx| {
            *testing = true;
            cx.notify();
        });
        let result = self.test_result.clone();
        let testing = self.is_testing.clone();
        let task = one_core::gpui_tokio::Tokio::spawn_result(cx, async move {
            service
                .test_extension_connection(contribution, config, secrets)
                .await
        });
        cx.spawn(async move |_, cx| {
            let outcome = task.await.map_err(|error| error.to_string());
            let _ = cx.update(|cx| {
                testing.update(cx, |testing, cx| {
                    *testing = false;
                    cx.notify();
                });
                result.update(cx, |result, cx| {
                    *result = Some(outcome);
                    cx.notify();
                });
            });
        })
        .detach();
    }

    pub(super) fn on_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).text().to_string().trim().to_string();
        if name.is_empty() {
            self.set_error("Connection name is required", cx);
            return;
        }
        let (config, updates) = match self.draft(cx) {
            Ok(value) => value,
            Err(error) => {
                self.set_error(error, cx);
                return;
            }
        };
        let declared = self.fields.read(cx).visible_secret_ids(cx);
        let cleared = self.fields.read(cx).cleared_secret_ids();
        let workspace_id = self.workspace.read(cx).selected_value().cloned().flatten();
        let team_id = connection_form::team::selected_team_id(&self.team, cx);
        let assignment = match connection_form::team::resolve_team_assignment(
            team_id,
            self.editing_connection.is_some(),
            self.editing_connection
                .as_ref()
                .and_then(|connection| connection.owner_id.clone()),
            cx,
        ) {
            Ok(assignment) => assignment,
            Err(error) => {
                self.set_error(error.to_string(), cx);
                return;
            }
        };
        let mut connection = match build_connection(
            self.editing_connection.as_ref(),
            &self.contribution,
            ExtensionConnectionDraft {
                name,
                config,
                secret_updates: updates,
                visible_secrets: declared,
                cleared_secrets: cleared,
                workspace_id,
                team_id: assignment.team_id,
                owner_id: assignment.owner_id,
                remark: optional_input_text(&self.remark, cx),
                sync_enabled: *self.sync_enabled.read(cx),
            },
        ) {
            Ok(connection) => connection,
            Err(error) => {
                self.set_error(error.to_string(), cx);
                return;
            }
        };
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let Some(repository) = storage.get::<ConnectionRepository>() else {
            self.set_error("Connection repository is unavailable", cx);
            return;
        };
        let outcome = persist_connection(&repository, &mut connection);
        match outcome {
            Ok(event) => {
                emit_connection_event(event, cx);
                window.remove_window();
            }
            Err(error) => self.set_error(error.to_string(), cx),
        }
    }

    fn set_error(&self, error: impl Into<String>, cx: &mut Context<Self>) {
        self.test_result.update(cx, |result, cx| {
            *result = Some(Err(error.into()));
            cx.notify();
        });
    }
}
