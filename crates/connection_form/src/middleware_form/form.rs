//! 通用中间件连接表单实体
//!
//! 镜像 `db_view::DbConnectionForm` 的通用机制(声明式字段状态机、标签页
//! 渲染、钥匙串/工作区/团队/云同步区块、SSH 隧道页、内联测试结果、保存
//! 流程),去掉数据库特有逻辑(插件测试、Oracle 双驱动、代理、URL 拼接)。
//! 中间件差异由 `MiddlewareFormAdapter` 承担。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, AsyncApp, Axis, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window, div, px,
};
use gpui_component::{
    ActiveTheme, IconName, Sizable, Size,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    form::{Field, field, v_form},
    h_flex,
    input::{Input, InputState, Textarea},
    radio::Radio,
    scroll::ScrollableElement,
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    tab::{Tab, TabBar},
    v_flex,
};
use one_core::cloud_sync::TeamOption;
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection, Workspace,
};
use rust_i18n::t;

use super::adapter::{FormSnapshot, MiddlewareFormAdapter};
use super::declarative::normalized_ssh_auth_type_or_default;
use crate::SshConnectionSelectItem;
use crate::declarative::{
    DeclarativeFieldType, DeclarativeForm, DeclarativeFormConfig, DeclarativeFormField,
    DeclarativeFormTab,
};
use crate::ssh_auth::SshAuthOption;
use crate::team::{
    TeamSelectItem, connection_sync_controls_visible_in, create_team_select, refresh_team_options,
    refresh_teams_tooltip, replace_team_options, resolve_team_assignment, selected_team_id,
    team_label, team_management_enabled,
};

/// 中间件表单配置
pub struct MiddlewareFormConfig {
    pub tab_groups: Vec<DeclarativeFormTab>,
}

/// 保存成功/失败事件
#[derive(Clone, Debug)]
pub enum MiddlewareFormEvent {
    Saved(Box<StoredConnection>),
    SaveError(String),
}

/// 工作区下拉项
#[derive(Clone, Debug)]
pub struct WorkspaceSelectItem {
    pub id: Option<i64>,
    pub name: String,
}

impl WorkspaceSelectItem {
    pub fn none() -> Self {
        Self {
            id: None,
            name: t!("Common.none").to_string(),
        }
    }

    pub fn from_workspace(workspace: &Workspace) -> Self {
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

/// 表单下拉项
#[derive(Clone, Debug)]
pub struct FormSelectItem {
    pub value: String,
    pub label: String,
}

impl FormSelectItem {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

impl SelectItem for FormSelectItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

fn ssh_auth_requires_private_key(auth_type: &str) -> bool {
    normalized_ssh_auth_type_or_default(auth_type) == SshAuthOption::PrivateKey.value()
}

fn ssh_auth_requires_private_key_content(auth_type: &str) -> bool {
    normalized_ssh_auth_type_or_default(auth_type) == SshAuthOption::PrivateKeyContent.value()
}

fn ssh_auth_requires_password(auth_type: &str) -> bool {
    normalized_ssh_auth_type_or_default(auth_type) == SshAuthOption::Password.value()
}

/// 启用 SSH 隧道时校验必填字段,返回缺失字段名
fn missing_ssh_tunnel_required_field(
    enabled: bool,
    ssh_host: &str,
    ssh_username: &str,
    auth_type: &str,
    ssh_private_key_path: &str,
    ssh_private_key_content: &str,
    ssh_password: &str,
) -> Option<&'static str> {
    if !enabled {
        return None;
    }

    if ssh_host.trim().is_empty() {
        return Some("ssh_host");
    }

    if ssh_username.trim().is_empty() {
        return Some("ssh_username");
    }

    if ssh_auth_requires_private_key(auth_type) && ssh_private_key_path.trim().is_empty() {
        return Some("ssh_private_key_path");
    }

    if ssh_auth_requires_private_key_content(auth_type) && ssh_private_key_content.trim().is_empty()
    {
        return Some("ssh_private_key_content");
    }

    if ssh_auth_requires_password(auth_type) && ssh_password.trim().is_empty() {
        return Some("ssh_password");
    }

    None
}

/// 判断是否启用自定义 SSH 标签页渲染(声明了约定 SSH 字段即启用)
fn should_use_custom_ssh_tab(fields: &[DeclarativeFormField]) -> bool {
    fields.iter().any(|field| field.id == "ssh_tunnel_enabled")
}

/// 通用中间件连接表单
pub struct MiddlewareConnectionForm {
    adapter: Arc<dyn MiddlewareFormAdapter>,
    config: MiddlewareFormConfig,
    focus_handle: FocusHandle,
    active_tab: usize,
    /// 统一声明式字段引擎:字段控件状态与收集均由此承担。
    declarative: Entity<DeclarativeForm>,
    is_testing: Entity<bool>,
    test_result: Entity<Option<Result<bool, String>>>,
    workspace_select: Entity<SelectState<Vec<WorkspaceSelectItem>>>,
    team_select: Entity<SelectState<Vec<TeamSelectItem>>>,
    ssh_connection_select: Entity<SelectState<SearchableVec<SshConnectionSelectItem>>>,
    selected_ssh_connection_id: Option<i64>,
    ssh_connections: Vec<StoredConnection>,
    editing_connection: Option<StoredConnection>,
    /// 编辑/预填时保留的透传字段(如 MQTT 协议版本)
    editing_extras: HashMap<String, String>,
    /// 是否启用云同步
    sync_enabled: Entity<bool>,
    _subscriptions: Vec<Subscription>,
}

impl MiddlewareConnectionForm {
    pub fn new(
        adapter: Arc<dyn MiddlewareFormAdapter>,
        config: MiddlewareFormConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();

        let declarative = cx.new(|cx| {
            DeclarativeForm::new(
                DeclarativeFormConfig {
                    tabs: config.tab_groups.clone(),
                },
                &serde_json::Map::new(),
                window,
                cx,
            )
        });

        let is_testing = cx.new(|_| false);
        let test_result = cx.new(|_| None);

        let workspace_items = vec![WorkspaceSelectItem::none()];
        let workspace_select =
            cx.new(|cx| SelectState::new(workspace_items, Some(Default::default()), window, cx));

        let team_select = create_team_select(&[], None, window, cx);

        let ssh_connection_items = SearchableVec::new(vec![SshConnectionSelectItem::none()]);
        let ssh_connection_select = cx.new(|cx| {
            SelectState::new(ssh_connection_items, Some(Default::default()), window, cx)
                .searchable(true)
        });
        cx.subscribe_in(
            &ssh_connection_select,
            window,
            move |form,
                  _select,
                  event: &SelectEvent<SearchableVec<SshConnectionSelectItem>>,
                  window,
                  cx| {
                let SelectEvent::Confirm(selected_value) = event;
                let selected_id = selected_value.as_ref().copied().flatten();
                form.selected_ssh_connection_id = selected_id;
                let value = selected_id.map(|id| id.to_string()).unwrap_or_default();
                form.set_field_value("ssh_connection_id", &value, window, cx);
            },
        )
        .detach();

        // 默认启用云同步,与数据库表单保持一致
        let sync_enabled = cx.new(|_| true);

        Self {
            adapter,
            config,
            focus_handle,
            active_tab: 0,
            declarative,
            is_testing,
            test_result,
            workspace_select,
            team_select,
            ssh_connection_select,
            selected_ssh_connection_id: None,
            ssh_connections: Vec::new(),
            editing_connection: None,
            editing_extras: HashMap::new(),
            sync_enabled,
            _subscriptions: Vec::new(),
        }
    }

    pub fn set_workspaces(
        &mut self,
        workspaces: Vec<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut items = vec![WorkspaceSelectItem::none()];
        items.extend(workspaces.iter().map(WorkspaceSelectItem::from_workspace));

        self.workspace_select.update(cx, |select, cx| {
            select.set_items(items, window, cx);
        });
        cx.notify();
    }

    pub fn set_teams(
        &mut self,
        teams: Vec<TeamOption>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        replace_team_options(&self.team_select, &teams, window, cx);
        cx.notify();
    }

    pub fn set_ssh_connections(
        &mut self,
        connections: Vec<StoredConnection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_ssh_connection_id = self.current_ssh_connection_id(cx);
        self.ssh_connections = connections
            .into_iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .collect();
        let mut items = vec![SshConnectionSelectItem::none()];
        items.extend(
            self.ssh_connections
                .iter()
                .map(SshConnectionSelectItem::from_connection),
        );

        self.ssh_connection_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(items), window, cx);
        });
        self.sync_ssh_connection_selection(window, cx);
        cx.notify();
    }

    /// 编辑模式回填
    pub fn load_connection(
        &mut self,
        connection: &StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_connection = Some(connection.clone());
        self.selected_ssh_connection_id = None;
        self.set_field_value("ssh_connection_id", "", window, cx);
        self.set_field_value("name", &connection.name, window, cx);

        self.sync_enabled.update(cx, |sync, cx| {
            *sync = connection.sync_enabled;
            cx.notify();
        });

        match self.adapter.load_fields(connection) {
            Ok(snapshot) => {
                // 钥匙串引用 + 手动用户名密码回填到 auth 字段
                if let Some(auth_name) = self.find_auth_field().map(|field| field.id.clone()) {
                    self.declarative.update(cx, |form, cx| {
                        form.set_auth_reference(
                            &auth_name,
                            snapshot.credential_reference.clone(),
                            window,
                            cx,
                        );
                    });
                    if let Some(username) = snapshot.fields.get("username") {
                        self.set_field_value(
                            &format!("{auth_name}.username"),
                            username,
                            window,
                            cx,
                        );
                    }
                    if let Some(password) = snapshot.fields.get("password") {
                        self.set_field_value(
                            &format!("{auth_name}.password"),
                            password,
                            window,
                            cx,
                        );
                    }
                }
                for (key, value) in &snapshot.fields {
                    if key == "username" || key == "password" {
                        continue;
                    }
                    if self.find_field(key).is_some() {
                        self.set_field_value(key, value, window, cx);
                    } else if key != "name" && key != "remark" {
                        // 未声明的键进入透传字段
                        self.editing_extras.insert(key.clone(), value.clone());
                    }
                }
                if let Some(ssh_connection_id) = snapshot
                    .fields
                    .get("ssh_connection_id")
                    .and_then(|value| value.parse::<i64>().ok())
                {
                    self.selected_ssh_connection_id = Some(ssh_connection_id);
                }
                self.editing_extras.extend(snapshot.extras);
            }
            Err(error) => {
                // 回填失败不阻断表单,仅提示;用户可修改后重新保存
                self.test_result.update(cx, |result, cx| {
                    *result = Some(Err(error));
                    cx.notify();
                });
            }
        }

        if let Some(remark) = &connection.remark {
            self.set_field_value("remark", remark, window, cx);
        }

        if let Some(ws_id) = connection.workspace_id {
            self.workspace_select.update(cx, |select, cx| {
                select.set_selected_value(&Some(ws_id), window, cx);
            });
        } else {
            self.workspace_select.update(cx, |select, cx| {
                select.set_selected_value(&None, window, cx);
            });
        }

        if let Some(ref team_id) = connection.team_id {
            self.team_select.update(cx, |select, cx| {
                select.set_selected_value(&Some(team_id.clone()), window, cx);
            });
        } else {
            self.team_select.update(cx, |select, cx| {
                select.set_selected_value(&None, window, cx);
            });
        }

        self.sync_ssh_connection_selection(window, cx);
    }

    /// 预填模式:回填但保持新增语义(不进入更新模式)
    pub fn load_initial_connection(
        &mut self,
        connection: &StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load_connection(connection, window, cx);
        self.editing_connection = None;
    }

    fn current_ssh_connection_id(&self, cx: &App) -> Option<i64> {
        self.selected_ssh_connection_id.or_else(|| {
            self.get_field_value("ssh_connection_id", cx)
                .and_then(|value| value.parse::<i64>().ok())
        })
    }

    fn sync_ssh_connection_selection(&self, window: &mut Window, cx: &mut App) {
        let selected_id = self.current_ssh_connection_id(cx);
        self.ssh_connection_select.update(cx, |select, cx| {
            select.set_selected_value(&selected_id, window, cx);
        });
    }

    fn set_field_value<C: AppContext>(
        &mut self,
        field_name: &str,
        value: &str,
        window: &mut Window,
        cx: &mut C,
    ) {
        let declarative = self.declarative.clone();
        declarative.update(cx, |form, cx| {
            form.set_field_value(field_name, value, window, cx)
        });
    }

    fn set_bool_field_value<C: AppContext>(
        &mut self,
        field_name: &str,
        value: bool,
        window: &mut Window,
        cx: &mut C,
    ) {
        self.set_field_value(field_name, if value { "true" } else { "false" }, window, cx);
    }

    fn get_field_value(&self, field_name: &str, cx: &App) -> Option<String> {
        self.find_field(field_name)
            .map(|_| self.declarative.read(cx).value(field_name, cx))
    }

    fn field_bool_value(&self, field_name: &str, cx: &App) -> bool {
        self.get_field_value(field_name, cx)
            .map(|value| value == "true" || value == "1")
            .unwrap_or(false)
    }

    fn field_label(&self, field_name: &str) -> String {
        self.find_field(field_name)
            .map(|field| field.label.clone())
            .unwrap_or_else(|| field_name.to_string())
    }

    fn find_field(&self, field_name: &str) -> Option<&DeclarativeFormField> {
        self.config
            .tab_groups
            .iter()
            .flat_map(|group| group.fields.iter())
            .find(|field| field.id == field_name)
    }

    fn find_auth_field(&self) -> Option<&DeclarativeFormField> {
        self.config
            .tab_groups
            .iter()
            .flat_map(|group| group.fields.iter())
            .find(|field| field.field_type == DeclarativeFieldType::Auth)
    }

    fn get_input_by_name(&self, field_name: &str, cx: &App) -> Option<Entity<InputState>> {
        self.declarative.read(cx).input_state(field_name)
    }

    fn get_textarea_by_name(
        &self,
        field_name: &str,
        cx: &App,
    ) -> Option<Entity<gpui_component::input::TextareaState>> {
        self.declarative.read(cx).textarea_state(field_name)
    }

    fn field_visible_from_values(&self, field: &DeclarativeFormField, cx: &App) -> bool {
        field
            .visible_when
            .iter()
            .all(|rule| rule.matches(self.get_field_value(&rule.field, cx).as_deref()))
    }

    fn resolve_referenced_ssh_connection(&self, cx: &App) -> Option<&StoredConnection> {
        let selected_id = self.current_ssh_connection_id(cx)?;

        self.ssh_connections
            .iter()
            .find(|connection| connection.id == Some(selected_id))
    }

    /// 构建表单快照(可见字段 + 透传字段 + 凭据引用)
    fn build_snapshot(&self, cx: &App) -> FormSnapshot {
        let auth_field = self.find_auth_field();
        let credential_reference =
            auth_field.and_then(|field| self.declarative.read(cx).auth_reference(&field.id, cx));
        let mut fields = HashMap::new();

        for field in self
            .config
            .tab_groups
            .iter()
            .flat_map(|group| &group.fields)
        {
            let field_name = &field.id;
            if !self.field_visible_from_values(field, cx) {
                continue;
            }
            if field.field_type == DeclarativeFieldType::Auth {
                // auth 字段:手动用户名/密码仅在未选钥匙串引用时进入快照
                if credential_reference.is_none() {
                    let declarative = self.declarative.read(cx);
                    fields.insert(
                        "username".to_string(),
                        declarative.auth_value(field_name, "username", cx),
                    );
                    fields.insert(
                        "password".to_string(),
                        declarative.auth_value(field_name, "password", cx),
                    );
                }
                continue;
            }
            fields.insert(
                field_name.clone(),
                self.declarative.read(cx).value(field_name, cx),
            );
        }

        FormSnapshot {
            fields,
            extras: self.editing_extras.clone(),
            credential_reference,
        }
    }

    /// 连接名称:显式填写优先,否则用适配器默认名
    fn connection_name(&self, snapshot: &FormSnapshot, cx: &App) -> String {
        let name = self.get_field_value("name", cx).unwrap_or_default();
        if name.trim().is_empty() {
            self.adapter.default_name(snapshot)
        } else {
            name
        }
    }

    fn validate(&self, cx: &App) -> Result<(), String> {
        let credential_selected = self
            .find_auth_field()
            .and_then(|field| self.declarative.read(cx).auth_reference(&field.id, cx))
            .is_some();

        for tab_group in &self.config.tab_groups {
            for field in &tab_group.fields {
                if !self.field_visible_from_values(field, cx) {
                    continue;
                }
                if field.field_type == DeclarativeFieldType::Auth {
                    if credential_selected {
                        continue;
                    }
                    if field.required {
                        let declarative = self.declarative.read(cx);
                        let username = declarative.auth_value(&field.id, "username", cx);
                        let password = declarative.auth_value(&field.id, "password", cx);
                        if username.trim().is_empty() && password.trim().is_empty() {
                            return Err(t!("ConnectionForm.field_required", label = field.label)
                                .to_string());
                        }
                    }
                    continue;
                }
                if field.required {
                    let value = self.get_field_value(&field.id, cx);
                    if value.is_none_or(|value| value.trim().is_empty()) {
                        return Err(
                            t!("ConnectionForm.field_required", label = field.label).to_string()
                        );
                    }
                }
            }
        }

        self.validate_ssh_tunnel(cx)?;
        Ok(())
    }

    fn validate_ssh_tunnel(&self, cx: &App) -> Result<(), String> {
        let enabled = self.field_bool_value("ssh_tunnel_enabled", cx);
        let auth_type = self
            .get_field_value("ssh_auth_type", cx)
            .unwrap_or_else(|| "password".to_string());
        if self.resolve_referenced_ssh_connection(cx).is_some() {
            return Ok(());
        }
        let missing_field = missing_ssh_tunnel_required_field(
            enabled,
            &self.get_field_value("ssh_host", cx).unwrap_or_default(),
            &self.get_field_value("ssh_username", cx).unwrap_or_default(),
            &auth_type,
            &self
                .get_field_value("ssh_private_key_path", cx)
                .unwrap_or_default(),
            &self
                .get_field_value("ssh_private_key_content", cx)
                .unwrap_or_default(),
            &self.get_field_value("ssh_password", cx).unwrap_or_default(),
        );

        if let Some(field) = missing_field {
            return Err(format!(
                "{}: {}",
                t!("ConnectionForm.ssh_tunnel_invalid"),
                t!("ConnectionForm.ssh_missing_required", field = field)
            ));
        }

        Ok(())
    }

    /// 由表单构建连接(不含团队/备注等引擎附加字段)
    fn build_connection_from_fields(&self, cx: &App) -> Result<StoredConnection, String> {
        self.validate(cx)?;

        let snapshot = self.build_snapshot(cx);
        let name = self.connection_name(&snapshot, cx);
        let workspace_id = self
            .workspace_select
            .read(cx)
            .selected_value()
            .cloned()
            .flatten();

        let mut connection = self
            .adapter
            .build_connection(&snapshot, name, workspace_id)?;
        // 防御:确保连接类型与适配器声明一致
        connection.connection_type = self.adapter.connection_type();
        connection.workspace_id = workspace_id;
        Ok(connection)
    }

    /// 构建待保存的存储连接(含团队/云同步元数据)
    pub fn build_stored_connection(&self, cx: &App) -> Result<(StoredConnection, bool), String> {
        let mut stored = self.build_connection_from_fields(cx)?;
        let remark = self.get_field_value("remark", cx);
        let is_update = self.editing_connection.is_some();
        let sync_enabled = *self.sync_enabled.read(cx);
        let team_id = selected_team_id(&self.team_select, cx);

        if let Some(conn) = &self.editing_connection {
            stored.id = conn.id;
            stored.cloud_id = conn.cloud_id.clone();
            stored.last_synced_at = conn.last_synced_at;
            stored.owner_id = conn.owner_id.clone();
        }

        stored.sync_enabled = sync_enabled;

        let assignment = resolve_team_assignment(team_id, is_update, stored.owner_id.clone(), cx)
            .map_err(|error| error.to_string())?;
        stored.team_id = assignment.team_id;
        stored.owner_id = assignment.owner_id;
        stored.remark = remark;
        Ok((stored, is_update))
    }

    /// 触发连接测试(内联展示结果)
    pub fn trigger_test_connection(&mut self, cx: &mut Context<Self>) {
        let connection = match self.build_connection_from_fields(cx) {
            Ok(connection) => connection,
            Err(error) => {
                self.test_result.update(cx, |result, cx| {
                    *result = Some(Err(error));
                    cx.notify();
                });
                return;
            }
        };

        self.is_testing.update(cx, |testing, cx| {
            *testing = true;
            cx.notify();
        });
        self.test_result.update(cx, |result, cx| {
            *result = None;
            cx.notify();
        });

        let task = self.adapter.test_connection(&connection, cx);
        let is_testing_handle = self.is_testing.clone();
        let test_result_handle = self.test_result.clone();

        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result: Result<bool, String> = task
                .await
                .map(|_| true)
                .map_err(|error| format!("{}: {}", t!("ConnectionForm.test_failed"), error));

            let _ = cx.update(|cx| {
                is_testing_handle.update(cx, |testing, cx| {
                    *testing = false;
                    cx.notify();
                });
                test_result_handle.update(cx, |result_state, cx| {
                    *result_state = Some(result);
                    cx.notify();
                });
            });
        })
        .detach();
    }

    /// 保存连接(insert/update),结果经事件上报
    pub fn save_connection(&mut self, cx: &mut Context<Self>) {
        let (stored, is_update) = match self.build_stored_connection(cx) {
            Ok(data) => data,
            Err(error) => {
                self.set_save_error(error.clone(), cx);
                cx.emit(MiddlewareFormEvent::SaveError(error));
                return;
            }
        };

        let storage = cx.global::<GlobalStorageState>().storage.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let Some(repo) = storage.get::<ConnectionRepository>() else {
                let error = t!("ConnectionForm.repository_missing").to_string();
                let _ = this.update(cx, |form, cx| {
                    form.set_save_error(error.clone(), cx);
                    cx.emit(MiddlewareFormEvent::SaveError(error));
                });
                return;
            };
            let result = if is_update {
                repo.update(&stored).map(|_| stored)
            } else {
                let mut stored = stored;
                repo.insert(&mut stored).map(|_| stored)
            };
            match result {
                Ok(saved) => {
                    let _ = this.update(cx, |form, cx| {
                        form.editing_connection = None;
                        cx.emit(MiddlewareFormEvent::Saved(Box::new(saved)));
                    });
                }
                Err(error) => {
                    let message = format!("{}: {}", t!("ConnectionForm.save_failed"), error);
                    let _ = this.update(cx, |form, cx| {
                        form.set_save_error(message.clone(), cx);
                        cx.emit(MiddlewareFormEvent::SaveError(message));
                    });
                }
            }
        })
        .detach();
    }

    pub fn set_save_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.test_result.update(cx, |result, cx| {
            *result = Some(Err(error));
            cx.notify();
        });
    }

    pub fn trigger_cancel(&mut self, _cx: &mut Context<Self>) {
        self.editing_connection = None;
    }

    pub fn is_testing(&self, cx: &App) -> bool {
        *self.is_testing.read(cx)
    }

    /// 测试结果展示文本(`None` 表示尚无结果)
    pub fn test_result_msg(&self, cx: &App) -> Option<String> {
        self.test_result.read(cx).as_ref().map(|r| match r {
            Ok(_) => format!("✓ {}", t!("ConnectionForm.test_success")),
            Err(e) => format!("✗ {e}"),
        })
    }

    pub fn clear_test_result(&mut self, cx: &mut Context<Self>) {
        self.test_result.update(cx, |test_result, cx| {
            *test_result = None;
            cx.notify();
        });
    }

    /// 渲染单个声明字段(SSH 自定义页用),控件复用统一引擎的底层输入态。
    fn render_field_by_name(&self, field_name: &str, cx: &mut Context<Self>) -> Field {
        let Some(info) = self.find_field(field_name) else {
            return field();
        };
        if !self.field_visible_from_values(info, cx) {
            return field();
        }
        let is_password = info.field_type == DeclarativeFieldType::Password;
        field()
            .label(info.label.clone())
            .required(info.required)
            .items_center()
            .child(
                h_flex().w_full().gap_2().child(match info.field_type {
                    DeclarativeFieldType::TextArea => self
                        .get_textarea_by_name(&info.id, cx)
                        .map(|state| Textarea::new(&state).w_full().into_any_element())
                        .unwrap_or_else(|| div().w_full().into_any_element()),
                    _ => self
                        .get_input_by_name(&info.id, cx)
                        .map(|state| {
                            let input = Input::new(&state).w_full();
                            if is_password {
                                input.mask_toggle().into_any_element()
                            } else {
                                input.into_any_element()
                            }
                        })
                        .unwrap_or_else(|| div().w_full().into_any_element()),
                }),
            )
    }

    /// SSH 标签页:启用开关 + 引用已有连接 + 手动字段
    fn render_ssh_tab_content(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let ssh_enabled = self.field_bool_value("ssh_tunnel_enabled", cx);
        let using_ssh_reference = self.resolve_referenced_ssh_connection(cx).is_some();
        let ssh_auth_type = self
            .get_field_value("ssh_auth_type", cx)
            .unwrap_or_else(|| "password".to_string());
        let ssh_auth_type = normalized_ssh_auth_type_or_default(&ssh_auth_type).to_string();

        v_form()
            .layout(Axis::Horizontal)
            .with_size(Size::Medium)
            .columns(1)
            .label_width(px(120.))
            .child(
                field()
                    .label(self.field_label("ssh_tunnel_enabled"))
                    .items_center()
                    .child(
                        Checkbox::new("middleware-ssh-tunnel-enabled")
                            .checked(ssh_enabled)
                            .on_click(cx.listener(|this, _, window, cx| {
                                let next_enabled = !this.field_bool_value("ssh_tunnel_enabled", cx);
                                this.set_bool_field_value(
                                    "ssh_tunnel_enabled",
                                    next_enabled,
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .when(ssh_enabled, |form| {
                form.child(
                    field()
                        .label(t!("ConnectionForm.ssh_connection_id").to_string())
                        .items_center()
                        .child(
                            Select::new(&self.ssh_connection_select)
                                .placeholder(t!("ConnectionForm.ssh_connection_manual"))
                                .w_full(),
                        ),
                )
                .when(!using_ssh_reference, |form| {
                    form.child(self.render_field_by_name("ssh_host", cx))
                        .child(self.render_field_by_name("ssh_port", cx))
                        .child(self.render_field_by_name("ssh_username", cx))
                        .child(
                            field()
                                .label(self.field_label("ssh_auth_type"))
                                .items_center()
                                .child(h_flex().w_full().flex_wrap().gap_4().children(
                                    SshAuthOption::ALL.iter().copied().map(|option| {
                                        Radio::new(format!(
                                            "middleware-ssh-auth-{}",
                                            option.value()
                                        ))
                                        .label(option.label())
                                        .checked(ssh_auth_type == option.value())
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.set_field_value(
                                                    "ssh_auth_type",
                                                    option.value(),
                                                    window,
                                                    cx,
                                                );
                                            }),
                                        )
                                    }),
                                )),
                        )
                        .when(ssh_auth_type == SshAuthOption::Password.value(), |form| {
                            form.child(self.render_field_by_name("ssh_password", cx))
                        })
                        .when(ssh_auth_type == SshAuthOption::PrivateKey.value(), |form| {
                            form.child(self.render_field_by_name("ssh_private_key_path", cx))
                                .child(self.render_field_by_name("ssh_private_key_passphrase", cx))
                        })
                        .when(
                            ssh_auth_type == SshAuthOption::PrivateKeyContent.value(),
                            |form| {
                                form.child(self.render_field_by_name("ssh_private_key_content", cx))
                                    .child(
                                        self.render_field_by_name("ssh_private_key_passphrase", cx),
                                    )
                            },
                        )
                })
                .child(self.render_field_by_name("ssh_target_host", cx))
                .child(self.render_field_by_name("ssh_target_port", cx))
            })
            .into_any_element()
    }

    /// 常规/普通标签页:统一引擎渲染声明字段(auth 字段自带钥匙串选择),
    /// 宿主负责 工作区/团队/云同步 区块(首个标签页)。
    fn host_current_tab(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let tab_index = self.active_tab;
        self.declarative.update(cx, |declarative, _| {
            declarative.host_content(tab_index, HashSet::new());
        });

        let mut content = v_flex().w_full().gap_4().child(self.declarative.clone());
        if tab_index == 0 {
            let chrome = self.render_general_chrome(cx);
            if let Some(chrome) = chrome {
                content = content.child(chrome);
            }
        }
        content.into_any_element()
    }

    /// 首个标签页底部的 工作区 / 团队 / 云同步 区块。
    fn render_general_chrome(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let mut children: Vec<Field> = Vec::new();
        children.push(
            field()
                .label(t!("ConnectionForm.workspace").to_string())
                .items_center()
                .child(Select::new(&self.workspace_select).w_full()),
        );
        if connection_sync_controls_visible_in(cx) && team_management_enabled(cx) {
            children.push(
                field().label(team_label()).items_center().child(
                    h_flex()
                        .gap_2()
                        .child(Select::new(&self.team_select).w_full())
                        .child(
                            Button::new("sync-middleware-teams")
                                .icon(IconName::Refresh)
                                .ghost()
                                .tooltip(refresh_teams_tooltip())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    refresh_team_options(&this.team_select, window, cx);
                                })),
                        ),
                ),
            );
        }
        if connection_sync_controls_visible_in(cx) {
            let sync_enabled = self.sync_enabled.clone();
            let is_sync_checked = *self.sync_enabled.read(cx);
            children.push(
                field()
                    .label(t!("ConnectionForm.cloud_sync").to_string())
                    .items_center()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Checkbox::new("middleware-sync-enabled")
                                    .checked(is_sync_checked)
                                    .on_click(move |_, _, cx| {
                                        sync_enabled.update(cx, |sync, cx| {
                                            *sync = !*sync;
                                            cx.notify();
                                        });
                                    }),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("ConnectionForm.cloud_sync_desc").to_string()),
                            ),
                    ),
            );
        }
        Some(
            v_form()
                .layout(Axis::Horizontal)
                .columns(1)
                .label_width(px(120.))
                .children(children)
                .into_any_element(),
        )
    }
}

impl EventEmitter<MiddlewareFormEvent> for MiddlewareConnectionForm {}

impl Focusable for MiddlewareConnectionForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MiddlewareConnectionForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current_tab_group = &self.config.tab_groups[self.active_tab];
        let current_tab_fields = &current_tab_group.fields;
        let current_tab_name = current_tab_group.id.as_str();
        let tab_content =
            if current_tab_name == "ssh" && should_use_custom_ssh_tab(current_tab_fields) {
                self.render_ssh_tab_content(window, cx)
            } else {
                self.host_current_tab(cx)
            };

        v_flex()
            .gap_4()
            .size_full()
            .child(
                div().flex().justify_center().child(
                    TabBar::new("middleware-connection-tabs")
                        .with_size(Size::Large)
                        .underline()
                        .selected_index(self.active_tab)
                        .on_click(cx.listener(|this, ix: &usize, window, cx| {
                            this.active_tab = *ix;
                            if this
                                .config
                                .tab_groups
                                .get(*ix)
                                .is_some_and(|tab| tab.id == "ssh")
                            {
                                this.sync_ssh_connection_selection(window, cx);
                            }
                            cx.notify();
                        }))
                        .children(
                            self.config
                                .tab_groups
                                .iter()
                                .map(|tab| Tab::new().label(tab.label.clone())),
                        ),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(250.))
                    .overflow_y_scrollbar()
                    .child(tab_content),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarative::DeclarativeVisibilityRule;
    use gpui::{Task, TestAppContext, VisualTestContext, WindowOptions};
    use one_core::settings::AppSettings;
    use one_core::storage::{MqttParams, SshAuthMethod, SshParams};

    /// 测试用标签页:常规 + 条件字段扩展页 + 共享 SSH/备注页
    fn mock_tab_groups() -> Vec<DeclarativeFormTab> {
        vec![
            DeclarativeFormTab::new("general", "常规").fields(vec![
                DeclarativeFormField::new("host", "主机", DeclarativeFieldType::Text)
                    .default("127.0.0.1"),
                DeclarativeFormField::new("port", "端口", DeclarativeFieldType::Number)
                    .default("1883"),
                DeclarativeFormField::new("auth", "认证", DeclarativeFieldType::Auth).optional(),
            ]),
            DeclarativeFormTab::new("extra", "扩展").fields(vec![
                DeclarativeFormField::new("use_flag", "开关", DeclarativeFieldType::Checkbox)
                    .optional()
                    .default("false"),
                DeclarativeFormField::new("token", "令牌", DeclarativeFieldType::Text)
                    .optional()
                    .visible_when(DeclarativeVisibilityRule::field_equals("use_flag", "true")),
            ]),
            super::super::declarative::ssh_tab_group(),
            super::super::declarative::notes_tab_group(),
        ]
    }

    /// mock 适配器:params 使用 `k=v;k2=v2` 编码,`x_` 前缀键进入透传字段
    struct MockAdapter;

    impl MockAdapter {
        fn encode(map: &HashMap<String, String>) -> String {
            map.iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(";")
        }

        fn decode(params: &str) -> HashMap<String, String> {
            params
                .split(';')
                .filter_map(|pair| pair.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        }

        fn stored(params: &str, id: Option<i64>) -> StoredConnection {
            let mut stored =
                StoredConnection::new_mqtt("mock".to_string(), MqttParams::default(), None);
            stored.params = params.to_string();
            stored.id = id;
            stored
        }
    }

    impl MiddlewareFormAdapter for MockAdapter {
        fn connection_type(&self) -> ConnectionType {
            ConnectionType::Mqtt
        }

        fn load_fields(&self, connection: &StoredConnection) -> Result<FormSnapshot, String> {
            let mut fields = HashMap::new();
            let mut extras = HashMap::new();
            for (key, value) in Self::decode(&connection.params) {
                if let Some(extra_key) = key.strip_prefix("x_") {
                    extras.insert(extra_key.to_string(), value);
                } else {
                    fields.insert(key, value);
                }
            }
            Ok(FormSnapshot {
                fields,
                extras,
                credential_reference: None,
            })
        }

        fn build_connection(
            &self,
            snapshot: &FormSnapshot,
            name: String,
            workspace_id: Option<i64>,
        ) -> Result<StoredConnection, String> {
            if snapshot
                .fields
                .get("host")
                .is_none_or(|host| host.trim().is_empty())
            {
                return Err("host required".to_string());
            }
            let mut map = snapshot.fields.clone();
            for (key, value) in &snapshot.extras {
                map.insert(format!("x_{key}"), value.clone());
            }
            let mut stored = StoredConnection::new_mqtt(name, MqttParams::default(), workspace_id);
            stored.params = Self::encode(&map);
            Ok(stored)
        }

        fn default_name(&self, snapshot: &FormSnapshot) -> String {
            let host = snapshot
                .fields
                .get("host")
                .map(String::as_str)
                .unwrap_or_default();
            let port = snapshot
                .fields
                .get("port")
                .map(String::as_str)
                .unwrap_or("1883");
            format!("{host}:{port}")
        }

        fn test_connection(
            &self,
            _connection: &StoredConnection,
            _cx: &mut App,
        ) -> Task<Result<(), String>> {
            unimplemented!("mock 适配器不执行真实连接测试")
        }
    }

    fn stored_ssh_connection(id: i64, name: &str, host: &str) -> StoredConnection {
        let mut connection = StoredConnection::new_ssh(
            name.to_string(),
            SshParams {
                sftp_default_directory: None,
                disabled_jump_server: None,
                sftp_account: None,
                host: host.to_string(),
                port: 22,
                username: "root".to_string(),
                auth_method: SshAuthMethod::Agent,
                credential_reference: None,
                prompt_username: None,
                prompt_password: None,
                keyboard_interactive: None,
                terminal_encoding: Default::default(),
                terminal_type: Default::default(),
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                x11_forwarding: None,
                allow_legacy_algorithms: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
                icon_file_path: None,
                account_expect: Default::default(),
            },
            None,
        );
        connection.id = Some(id);
        connection
    }

    fn open_form(
        cx: &mut TestAppContext,
        setup: impl FnOnce(
            &mut MiddlewareConnectionForm,
            &mut Window,
            &mut gpui::Context<MiddlewareConnectionForm>,
        ),
    ) -> (
        gpui::WindowHandle<MiddlewareConnectionForm>,
        VisualTestContext,
    ) {
        let window = cx
            .update(|cx| {
                cx.set_global(AppSettings::default());
                gpui_component::init(cx);
                cx.open_window(WindowOptions::default(), |window, cx| {
                    cx.new(|cx| {
                        let mut form = MiddlewareConnectionForm::new(
                            std::sync::Arc::new(MockAdapter),
                            MiddlewareFormConfig {
                                tab_groups: mock_tab_groups(),
                            },
                            window,
                            cx,
                        );
                        setup(&mut form, window, cx);
                        form
                    })
                })
            })
            .expect("表单窗口应可打开");
        let cx = VisualTestContext::from_window(window.into(), cx);
        (window, cx)
    }

    #[gpui::test]
    fn defaults_populate_and_build_new_connection(cx: &mut TestAppContext) {
        let (window, mut cx) = open_form(cx, |_form, _window, _cx| {});
        cx.run_until_parked();

        let form = window.root(&mut cx).expect("表单应已挂载");
        form.read_with(&cx, |form, cx| {
            // 声明式默认值生效
            assert_eq!(
                Some("127.0.0.1".to_string()),
                form.get_field_value("host", cx)
            );
            assert_eq!(Some("1883".to_string()), form.get_field_value("port", cx));
            // 条件字段在开关关闭时不进入快照
            let snapshot = form.build_snapshot(cx);
            assert!(!snapshot.fields.contains_key("token"));
            assert!(snapshot.fields.contains_key("use_flag"));
        });

        // 未填名称时使用适配器默认名;保存构建走新增分支
        let (stored, is_update) = form
            .read_with(&cx, |form, cx| form.build_stored_connection(cx))
            .expect("默认值应通过校验");
        assert!(!is_update);
        assert_eq!(stored.name, "127.0.0.1:1883");
        assert!(stored.params.contains("host=127.0.0.1"));
    }

    #[gpui::test]
    fn load_connection_restores_fields_and_extras_for_update(cx: &mut TestAppContext) {
        let existing = MockAdapter::stored("host=broker;port=8883;x_ver=5.0", Some(42));
        let (window, mut cx) = open_form(cx, |form, window, cx| {
            form.load_connection(&existing, window, cx);
        });
        cx.run_until_parked();

        let form = window.root(&mut cx).expect("表单应已挂载");
        form.read_with(&cx, |form, cx| {
            assert_eq!(Some("broker".to_string()), form.get_field_value("host", cx));
            assert_eq!(Some("8883".to_string()), form.get_field_value("port", cx));
            // 透传字段保留在快照中(mock 编解码会剥离 x_ 前缀)
            assert_eq!(
                Some(&"5.0".to_string()),
                form.build_snapshot(cx).extras.get("ver")
            );
        });

        let (stored, is_update) = form
            .read_with(&cx, |form, cx| form.build_stored_connection(cx))
            .expect("回填后应通过校验");
        assert!(is_update);
        assert_eq!(stored.id, Some(42));
        // 透传字段在保存时原样回传
        assert!(stored.params.contains("x_ver=5.0"));
    }

    #[gpui::test]
    fn credential_reference_suppresses_manual_credentials(cx: &mut TestAppContext) {
        // load_fields 返回钥匙串引用 + 手动用户名,引擎应只保留引用
        struct CredAdapter;
        impl MiddlewareFormAdapter for CredAdapter {
            fn connection_type(&self) -> ConnectionType {
                ConnectionType::Mqtt
            }
            fn load_fields(&self, _connection: &StoredConnection) -> Result<FormSnapshot, String> {
                let mut fields = HashMap::new();
                fields.insert("host".to_string(), "broker".to_string());
                fields.insert("username".to_string(), "手动输入".to_string());
                Ok(FormSnapshot {
                    fields,
                    extras: HashMap::new(),
                    credential_reference: Some(one_core::storage::CredentialReference::new(9)),
                })
            }
            fn build_connection(
                &self,
                _snapshot: &FormSnapshot,
                _name: String,
                _workspace_id: Option<i64>,
            ) -> Result<StoredConnection, String> {
                unimplemented!("本测试不触达保存构建")
            }
            fn default_name(&self, _snapshot: &FormSnapshot) -> String {
                "cred".to_string()
            }
            fn test_connection(
                &self,
                _connection: &StoredConnection,
                _cx: &mut App,
            ) -> Task<Result<(), String>> {
                unimplemented!()
            }
        }

        let window = cx
            .update(|cx| {
                cx.set_global(AppSettings::default());
                gpui_component::init(cx);
                cx.open_window(WindowOptions::default(), |window, cx| {
                    cx.new(|cx| {
                        let mut form = MiddlewareConnectionForm::new(
                            std::sync::Arc::new(CredAdapter),
                            MiddlewareFormConfig {
                                tab_groups: mock_tab_groups(),
                            },
                            window,
                            cx,
                        );
                        let connection = StoredConnection::new_mqtt(
                            "cred".to_string(),
                            MqttParams::default(),
                            None,
                        );
                        form.load_connection(&connection, window, cx);
                        form
                    })
                })
            })
            .expect("表单窗口应可打开");
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();

        let form = window.root(&mut cx).expect("表单应已挂载");
        form.read_with(&cx, |form, cx| {
            let snapshot = form.build_snapshot(cx);
            // 选择钥匙串引用后,手动账号/密码不进入快照
            assert!(!snapshot.fields.contains_key("username"));
            assert!(!snapshot.fields.contains_key("password"));
            assert!(snapshot.credential_reference.is_some());
            assert_eq!(
                Some("broker".to_string()),
                snapshot.fields.get("host").cloned()
            );
        });
    }

    #[gpui::test]
    fn ssh_tunnel_validation_requires_manual_fields(cx: &mut TestAppContext) {
        // 启用隧道但未填写手动字段 -> 校验失败
        let (window, mut cx) = open_form(cx, |form, window, cx| {
            form.set_field_value("ssh_tunnel_enabled", "true", window, cx);
        });
        cx.run_until_parked();

        let form = window.root(&mut cx).expect("表单应已挂载");
        let error = form
            .read_with(&cx, |form, cx| form.build_stored_connection(cx))
            .unwrap_err();
        assert!(error.contains("ssh"), "错误信息应指向 SSH 字段: {error}");
    }

    #[gpui::test]
    fn ssh_tunnel_reference_skips_manual_validation(cx: &mut TestAppContext) {
        // 引用已有 SSH 连接时跳过手动必填校验
        let (window, mut cx) = open_form(cx, |form, window, cx| {
            form.set_field_value("ssh_tunnel_enabled", "true", window, cx);
            form.set_ssh_connections(
                vec![stored_ssh_connection(7, "跳板", "10.0.0.5")],
                window,
                cx,
            );
            form.set_field_value("ssh_connection_id", "7", window, cx);
        });
        cx.run_until_parked();

        let form = window.root(&mut cx).expect("表单应已挂载");
        form.read_with(&cx, |form, cx| {
            assert!(
                form.build_stored_connection(cx).is_ok(),
                "引用已有 SSH 连接时应通过校验"
            );
            // 快照中包含引用 id,适配器可据此构建 connection_id 隧道
            assert_eq!(
                Some(&"7".to_string()),
                form.build_snapshot(cx).fields.get("ssh_connection_id")
            );
        });
    }

    #[gpui::test]
    fn visibility_rule_exposes_conditional_field(cx: &mut TestAppContext) {
        let (window, mut cx) = open_form(cx, |form, window, cx| {
            form.set_field_value("use_flag", "true", window, cx);
            form.set_field_value("token", "secret", window, cx);
        });
        cx.run_until_parked();

        let form = window.root(&mut cx).expect("表单应已挂载");
        form.read_with(&cx, |form, cx| {
            let snapshot = form.build_snapshot(cx);
            // 开关打开后条件字段进入快照
            assert_eq!(Some(&"secret".to_string()), snapshot.fields.get("token"));
        });
    }
}
