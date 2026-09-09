mod fields;
mod render;
mod schema;
mod storage;

use std::collections::HashMap;

use connection_form::declarative::DeclarativeForm;
use connection_form::{SshTunnelForm, SshTunnelFormConfig};
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
use rust_i18n::t;

use self::{
    fields::{create_input, create_name_input, create_workspace_select, optional_input_text},
    schema::declarative_config,
    storage::{ExtensionConnectionDraft, build_connection, persist_connection},
};
use crate::shell_plugin_host::ssh_tunnel;
use crate::universal_plugins::GlobalUniversalPluginService;

pub struct ExtensionConnectionFormConfig {
    pub contribution: extension_runtime::RegisteredResourceConnectionContribution,
    pub editing_connection: Option<StoredConnection>,
    pub workspaces: Vec<Workspace>,
    pub teams: Vec<TeamOption>,
    pub ssh_connections: Vec<StoredConnection>,
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

pub struct ExtensionConnectionForm {
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
    /// SSH 隧道表单(清单声明 host/port 或 namesrv_addrs 时展示)
    pub(super) ssh_tunnel_form: Option<Entity<SshTunnelForm>>,
    /// 当前激活页签:0..manifest 页签数=清单页签,其后依次为 SSH 页签(若有)与备注页签
    pub(super) active_tab: usize,
    pub(super) focus_handle: FocusHandle,
}

impl ExtensionConnectionForm {
    pub fn new(
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
        // 嵌入模式:manifest 页签与宿主"SSH/备注"页签由本表单统一渲染切换
        fields.update(cx, |form, _| form.set_embedded(true));
        // SSH 隧道页签:仅清单声明可隧道地址字段(host/port 或 namesrv_addrs)时提供,
        // 与数据库连接窗口的 SSH 页签保持一致体验
        let ssh_tunnel_form =
            ssh_tunnel::form_supports_tunnel(&config.contribution.form).then(|| {
                let initial = initial_config
                    .get("ssh_tunnel")
                    .and_then(ssh_tunnel::form_value_from_stored_json);
                let (target_host_placeholder, target_port_placeholder) =
                    tunnel_placeholders(&initial_config, &config.contribution.form);
                cx.new(|cx| {
                    SshTunnelForm::new(
                        SshTunnelFormConfig::new(
                            "extension-ssh",
                            target_host_placeholder,
                            target_port_placeholder,
                            t!("ConnectionForm.ssh_timeout"),
                            "30",
                        ),
                        config.ssh_connections.clone(),
                        initial,
                        window,
                        cx,
                    )
                })
            });
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
            t!("ExtensionConnectionForm.remark_placeholder").to_string(),
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
            ssh_tunnel_form,
            active_tab: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    /// SSH 页签是否展示(清单声明可隧道地址字段时才有)
    pub(super) fn ssh_tab_visible(&self) -> bool {
        self.ssh_tunnel_form.is_some()
    }

    /// SSH 页签索引:紧跟 manifest 页签之后(无 SSH 页签时返回 None)
    pub(super) fn ssh_tab_index(&self, cx: &App) -> Option<usize> {
        self.ssh_tab_visible()
            .then(|| self.fields.read(cx).tab_count())
    }

    /// 采集 SSH 隧道表单值并应用到 config:启用时写入存储形状,禁用时删除该键
    fn apply_ssh_tunnel(&self, config: &mut serde_json::Map<String, serde_json::Value>, cx: &App) {
        let Some(form) = &self.ssh_tunnel_form else {
            return;
        };
        let value = form.read(cx).value(cx);
        match ssh_tunnel::tunnel_from_form_value(&value) {
            Some(tunnel) => {
                config.insert(
                    "ssh_tunnel".into(),
                    ssh_tunnel::tunnel_to_stored_json(&tunnel),
                );
            }
            None => {
                config.remove("ssh_tunnel");
            }
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
        let (mut config, secrets) = match self.test_draft(cx) {
            Ok(draft) => draft,
            Err(error) => {
                self.set_error(error, cx);
                return;
            }
        };
        // 与 DB 驱动一致的统一凭据解析:Auth 密码簿引用解析为运行时明文后测试。
        if let Some(repository) = cx
            .try_global::<GlobalStorageState>()
            .and_then(|state| state.storage.get::<ConnectionRepository>())
        {
            if let Err(error) = repository.resolve_extension_runtime_config(&mut config) {
                self.set_error(error.to_string(), cx);
                return;
            }
        }
        // SSH 隧道随测试生效:启用时写入 config,测试链路据此建隧道改写地址
        self.apply_ssh_tunnel(&mut config, cx);
        let contribution = self.contribution.clone();
        let service = cx.global::<GlobalUniversalPluginService>().service();
        let repository = cx
            .global::<GlobalStorageState>()
            .storage
            .get::<ConnectionRepository>();
        self.is_testing.update(cx, |testing, cx| {
            *testing = true;
            cx.notify();
        });
        let result = self.test_result.clone();
        let testing = self.is_testing.clone();
        let task = one_core::gpui_tokio::Tokio::spawn_result(cx, async move {
            service
                .test_extension_connection(contribution, config, secrets, repository)
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
            self.set_error(t!("ExtensionConnectionForm.name_required").to_string(), cx);
            return;
        }
        let (mut config, updates) = match self.draft(cx) {
            Ok(value) => value,
            Err(error) => {
                self.set_error(error, cx);
                return;
            }
        };
        // SSH 隧道:启用时以迁移惰性形状存入 config.ssh_tunnel,禁用时删除该键
        self.apply_ssh_tunnel(&mut config, cx);
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
            self.set_error(t!("ConnectionForm.repository_missing").to_string(), cx);
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

    pub(super) fn on_cancel(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }

    pub(super) fn on_clear_test_result(&mut self, cx: &mut Context<Self>) {
        self.test_result.update(cx, |result, cx| {
            *result = None;
            cx.notify();
        });
    }

    fn test_result_msg(&self, cx: &App) -> Option<String> {
        self.test_result
            .read(cx)
            .as_ref()
            .map(|result| match result {
                Ok(()) => format!("✓ {}", t!("ConnectionForm.test_success")),
                Err(error) => format!("✗ {error}"),
            })
    }
}

/// 隧道目标占位符:优先取当前 config 的地址字段作提示(MQTT host/port;
/// RocketMQ 取首个 namesrv 地址的 host/port),新建连接回退 manifest 默认值,
/// 仍无地址信息时回退 127.0.0.1
fn tunnel_placeholders(
    config: &serde_json::Map<String, serde_json::Value>,
    form: &extension_runtime::extension::manifest::ResourceConnectionForm,
) -> (String, String) {
    if let Some(namesrv) = config
        .get("namesrv_addrs")
        .and_then(|value| value.as_str())
        .or_else(|| manifest_field(form, "namesrv_addrs"))
    {
        if let Some(first) = ssh_tunnel::split_namesrv_addrs(namesrv).first() {
            if let Ok((host, port)) = ssh_tunnel::parse_host_port(first) {
                return (host, port.to_string());
            }
        }
    }
    let host = config
        .get("host")
        .and_then(|value| value.as_str())
        .filter(|host| !host.trim().is_empty())
        .or_else(|| manifest_field(form, "host"))
        .unwrap_or("127.0.0.1")
        .to_string();
    let port = config
        .get("port")
        .and_then(|value| value.as_u64())
        .map(|port| port.to_string())
        .or_else(|| manifest_field(form, "port").map(str::to_string))
        .unwrap_or_default();
    (host, port)
}

/// 读取 manifest 字段声明的默认值/占位符文本(取默认值优先,无则占位符)
fn manifest_field<'a>(
    form: &'a extension_runtime::extension::manifest::ResourceConnectionForm,
    id: &str,
) -> Option<&'a str> {
    let field = form
        .tabs
        .iter()
        .flat_map(|tab| &tab.fields)
        .find(|field| field.id == id)?;
    field
        .default_value
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or(field.placeholder.as_deref())
        .filter(|value| !value.trim().is_empty())
}
