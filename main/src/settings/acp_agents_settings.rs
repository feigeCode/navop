//! 「ACP 智能体」设置页。
//!
//! 一行一个 agent，展示检测结论（名称/版本、
//! 解析到的路径或在 PATH 中查找），可启用/停用、可直接切换当前使用、展开后编辑启动
//! 参数与环境变量；右上角一个刷新按钮 + 「检查于 X 前」缓存标签。
//!
//! 检测结论走落盘缓存（[`ai_chat_view::AcpProbeCache`]），因此：
//! - 打开设置页不会每次都拉起 CLI 子进程；
//! - 改了参数/环境变量后启动指纹变化，旧结论自动失效并重新检测。

use std::time::{SystemTime, UNIX_EPOCH};

use ai_chat_view::{
    AcpAgentSource, AcpProbeCache, AcpProbeRecord, acp_probe_cache, emit_acp_agent_config_changed,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, WeakEntity, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState, Textarea, TextareaState},
    switch::Switch,
    v_flex,
};
use one_core::gpui_tokio::Tokio;
use one_core::settings::AppSettings;
use rust_i18n::t;

use crate::ai_chat_acp::{
    AcpAgentRow, AcpUserAgentSpec, probe_and_cache, remove_user_agent, reset_agent_override,
    save_agent_override, set_active_agent, set_agent_enabled, settings_rows, upsert_user_agent,
};

/// 检测结论的展示状态。
#[derive(Clone, Debug, PartialEq, Eq)]
enum RowStatus {
    Probing,
    /// 命令存在且作为 ACP agent 应答过。
    Ready,
    /// 命令存在但要求先登录。
    LoginRequired,
    Failed(String),
    /// 还没检测过（或缓存已随配置变更失效）。
    Unknown,
}

impl RowStatus {
    fn from_record(record: Option<&AcpProbeRecord>, probing: bool) -> Self {
        if probing {
            return Self::Probing;
        }
        let Some(record) = record else {
            return Self::Unknown;
        };
        if let Some(error) = record.probe.error.as_deref() {
            return Self::Failed(error.to_string());
        }
        if record.probe.models.is_empty() && !record.probe.auth_methods.is_empty() {
            return Self::LoginRequired;
        }
        Self::Ready
    }
}

/// 展开中的编辑区。同一时刻只会有一个，因此输入框就挂在视图字段上。
struct AgentDraft {
    /// 目标 agent id；新增时是用户填的 id。
    id: String,
    /// 新增（而不是编辑既有的）。
    is_new: bool,
    /// 用户自定义的 agent（可改命令/名字），还是扩展提供的（只能覆盖参数）。
    is_user_defined: bool,
    name: Entity<InputState>,
    command: Entity<InputState>,
    args: Entity<TextareaState>,
    env: Entity<TextareaState>,
}

pub struct AcpAgentsView {
    rows: Vec<AcpAgentRow>,
    load_error: Option<String>,
    save_error: Option<String>,
    probing: Vec<String>,
    draft: Option<AgentDraft>,
    _probe_generation: u64,
}

impl AcpAgentsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            rows: Vec::new(),
            load_error: None,
            save_error: None,
            probing: Vec::new(),
            draft: None,
            _probe_generation: 0,
        };
        view.reload(cx);
        view
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        match settings_rows() {
            Ok(rows) => {
                self.rows = rows;
                self.load_error = None;
            }
            Err(error) => {
                self.load_error = Some(error.to_string());
            }
        }
        self.sync_probes(cx);
        cx.notify();
    }

    fn active_agent(&self, cx: &App) -> Option<String> {
        AppSettings::current(cx).ai_chat.last_acp_agent_id.clone()
    }

    // ---- 探测 -----------------------------------------------------------

    /// 只对「没有新鲜缓存结论」的条目起探测：命中缓存就完全不起子进程。
    fn sync_probes(&mut self, cx: &mut Context<Self>) {
        let targets: Vec<(String, String, Vec<String>, Vec<(String, String)>)> = {
            let cache = acp_probe_cache(cx);
            self.rows
                .iter()
                .filter(|row| row.enabled && row.diagnostic.is_none())
                .filter(|row| !row.command.is_empty())
                .filter(|row| !self.probing.contains(&row.id))
                .filter(|row| match (&row.fingerprint, cache.get_any(&row.id)) {
                    (Some(fingerprint), Some(record)) => &record.fingerprint != fingerprint,
                    (Some(_), None) => true,
                    _ => false,
                })
                .map(|row| {
                    (
                        row.id.clone(),
                        row.command.clone(),
                        row.args.clone(),
                        row.env.clone(),
                    )
                })
                .collect()
        };
        if targets.is_empty() {
            return;
        }
        for (id, _, _, _) in &targets {
            self.probing.push(id.clone());
        }
        self._probe_generation = self._probe_generation.wrapping_add(1);
        let handle = Tokio::handle(cx);
        let task = cx.background_spawn(async move {
            let mut results = Vec::with_capacity(targets.len());
            for (id, command, args, env) in targets {
                // 探测只需要启动命令本身，名字与 id 不影响结论。
                let config = ai_chat_view::AcpAgentConfig::new(id.clone(), id.clone(), command)
                    .with_args(args)
                    .with_env(env);
                results.push((id, probe_and_cache(&config, handle.clone())));
            }
            results
        });
        cx.spawn(
            async move |this: WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let results = task.await;
                let _ = this.update(cx, |this, cx| {
                    for (id, record) in results {
                        acp_probe_cache(cx).store(&id, record);
                        this.probing.retain(|probing| probing != &id);
                    }
                    cx.notify();
                });
            },
        )
        .detach();
    }

    fn refresh_all(&mut self, cx: &mut Context<Self>) {
        let stale: Vec<String> = self.rows.iter().map(|row| row.id.clone()).collect();
        acp_probe_cache(cx).invalidate(&stale);
        self.probing.clear();
        self.sync_probes(cx);
        cx.notify();
    }

    // ---- 写入 -----------------------------------------------------------

    fn toggle_enabled(&mut self, id: &str, enabled: bool, cx: &mut Context<Self>) {
        match set_agent_enabled(id, enabled) {
            Ok(()) => self.after_write(cx),
            Err(error) => self.fail(error, cx),
        }
    }

    fn activate(&mut self, id: &str, cx: &mut Context<Self>) {
        set_active_agent(cx, Some(id));
        emit_acp_agent_config_changed(cx);
        cx.notify();
    }

    fn reset_override(&mut self, id: &str, cx: &mut Context<Self>) {
        match reset_agent_override(id) {
            Ok(()) => {
                self.draft = None;
                self.after_write(cx);
            }
            Err(error) => self.fail(error, cx),
        }
    }

    fn delete_user_agent(&mut self, id: &str, cx: &mut Context<Self>) {
        match remove_user_agent(id) {
            Ok(()) => {
                if self.active_agent(cx).as_deref() == Some(id) {
                    set_active_agent(cx, None);
                }
                self.draft = None;
                self.after_write(cx);
            }
            Err(error) => self.fail(error, cx),
        }
    }

    fn after_write(&mut self, cx: &mut Context<Self>) {
        self.save_error = None;
        emit_acp_agent_config_changed(cx);
        self.reload(cx);
    }

    fn fail(&mut self, error: anyhow::Error, cx: &mut Context<Self>) {
        self.save_error = Some(error.to_string());
        cx.notify();
    }

    // ---- 编辑区 ---------------------------------------------------------

    fn open_draft_for(
        &mut self,
        row: Option<&AcpAgentRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_error = None;
        let (id, name, command, args, env, is_new, is_user_defined) = match row {
            Some(row) => (
                row.id.clone(),
                row.name.clone(),
                row.command.clone(),
                join_lines(&row.args),
                join_env(&row.env),
                false,
                row.source == AcpAgentSource::User,
            ),
            None => (
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                true,
                true,
            ),
        };
        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("Settings.AcpAgents.name_placeholder").to_string())
                .default_value(&name)
        });
        let command_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("Settings.AcpAgents.command_placeholder").to_string())
                .default_value(&command)
        });
        let args_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 8)
                .placeholder(t!("Settings.AcpAgents.args_placeholder").to_string())
                .default_value(&args)
        });
        let env_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 8)
                .placeholder(t!("Settings.AcpAgents.env_placeholder").to_string())
                .default_value(&env)
        });
        self.draft = Some(AgentDraft {
            id,
            is_new,
            is_user_defined,
            name: name_input,
            command: command_input,
            args: args_input,
            env: env_input,
        });
        cx.notify();
    }

    fn save_draft(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.as_ref() else {
            return;
        };
        let name = draft.name.read(cx).value().to_string();
        let command = draft.command.read(cx).value().to_string();
        let args = parse_lines(draft.args.read(cx).value().as_ref());
        let env = parse_env_lines(draft.env.read(cx).value().as_ref());
        let id = draft.id.clone();
        let result = if draft.is_user_defined {
            let id = if draft.is_new {
                command_slug(&name, &command)
            } else {
                id
            };
            upsert_user_agent(AcpUserAgentSpec {
                id,
                name,
                command,
                args,
                env,
            })
        } else {
            save_agent_override(&id, args, env)
        };
        match result {
            Ok(()) => {
                self.draft = None;
                self.after_write(cx);
            }
            Err(error) => self.fail(error, cx),
        }
    }

    // ---- 渲染 -----------------------------------------------------------

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let checked_label = self.checked_label(cx);
        div()
            .w_full()
            .px_4()
            .py_3()
            .rounded(cx.theme().radius)
            .bg(cx.theme().secondary)
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_medium()
                                    .child(t!("Settings.AcpAgents.group_title").to_string()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("Settings.AcpAgents.description").to_string()),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_none()
                            .items_end()
                            .gap_1()
                            .child(
                                Button::new("acp-agents-refresh")
                                    .small()
                                    .outline()
                                    .label(t!("Settings.AcpAgents.refresh").to_string())
                                    .on_click(cx.listener(|this, _, _, cx| this.refresh_all(cx))),
                            )
                            .when_some(checked_label, |element, label| {
                                element.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(label),
                                )
                            }),
                    ),
            )
    }

    /// 「检查于 X 前」：取所有条目里最新的一次检测时间。
    fn checked_label(&self, cx: &App) -> Option<SharedString> {
        let cache = cx.try_global::<AcpProbeCache>()?;
        let newest = self
            .rows
            .iter()
            .filter_map(|row| cache.get_any(&row.id))
            .map(|record| record.checked_at)
            .max()?;
        Some(SharedString::from(
            t!(
                "Settings.AcpAgents.checked_ago",
                age = format_age(now_unix() - newest)
            )
            .to_string(),
        ))
    }

    fn render_row(&self, row: &AcpAgentRow, cx: &mut Context<Self>) -> impl IntoElement {
        let cached = cx
            .try_global::<AcpProbeCache>()
            .and_then(|cache| cache.get_any(&row.id));
        // 指纹变了（用户改了参数/环境变量）时旧结论不再代表当前配置。
        let record = match (&row.fingerprint, cached) {
            (Some(fingerprint), Some(record)) if record.fingerprint == *fingerprint => Some(record),
            _ => None,
        };
        let status = RowStatus::from_record(record.as_ref(), self.probing.contains(&row.id));
        let is_active = self.active_agent(cx).as_deref() == Some(row.id.as_str());
        let editing = self
            .draft
            .as_ref()
            .is_some_and(|draft| !draft.is_new && draft.id == row.id);

        let detail = match (&row.diagnostic, &record) {
            (Some(diagnostic), _) => SharedString::from(diagnostic.clone()),
            (None, Some(record)) => match record.command_path.as_deref() {
                Some(path) => SharedString::from(
                    t!("Settings.AcpAgents.detected_at", path = path).to_string(),
                ),
                None => SharedString::from(
                    t!(
                        "Settings.AcpAgents.searches_path",
                        command = row.command.clone()
                    )
                    .to_string(),
                ),
            },
            (None, None) => SharedString::from(
                t!(
                    "Settings.AcpAgents.searches_path",
                    command = row.command.clone()
                )
                .to_string(),
            ),
        };
        let version = record
            .as_ref()
            .and_then(|record| record.probe.version.clone())
            .map(|version| SharedString::from(format!("v{version}")));
        let failure = match &status {
            RowStatus::Failed(error) => Some(SharedString::from(error.clone())),
            _ => None,
        };

        let mut header = h_flex()
            .w_full()
            .items_start()
            .gap_3()
            .child(status_dot(&status, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .items_baseline()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_medium()
                                    .child(SharedString::from(row.name.clone())),
                            )
                            .when_some(version, |element, version| {
                                element.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(version),
                                )
                            })
                            .when(is_active, |element| {
                                element.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().primary)
                                        .child(t!("Settings.AcpAgents.active").to_string()),
                                )
                            })
                            .when(row.source == AcpAgentSource::User, |element| {
                                element.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(t!("Settings.AcpAgents.custom").to_string()),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(detail),
                    )
                    .when_some(failure, |element, failure| {
                        element.child(div().text_xs().text_color(cx.theme().danger).child(failure))
                    }),
            )
            .child(
                Button::new(SharedString::from(format!("acp-agent-expand-{}", row.id)))
                    .small()
                    .ghost()
                    .label(if editing {
                        t!("Settings.AcpAgents.collapse").to_string()
                    } else {
                        t!("Settings.AcpAgents.expand").to_string()
                    })
                    .on_click(cx.listener({
                        let row = row.clone();
                        move |this, _, window, cx| {
                            if this
                                .draft
                                .as_ref()
                                .is_some_and(|draft| !draft.is_new && draft.id == row.id)
                            {
                                this.draft = None;
                                cx.notify();
                            } else {
                                this.open_draft_for(Some(&row), window, cx);
                            }
                        }
                    })),
            )
            .child(
                Switch::new(SharedString::from(format!("acp-agent-enabled-{}", row.id)))
                    .small()
                    .checked(row.enabled)
                    .on_click(cx.listener({
                        let id = row.id.clone();
                        move |this, checked, _, cx| this.toggle_enabled(&id, *checked, cx)
                    })),
            );

        if !is_active && row.enabled && row.diagnostic.is_none() {
            header = header.child(
                Button::new(SharedString::from(format!("acp-agent-use-{}", row.id)))
                    .small()
                    .outline()
                    .label(t!("Settings.AcpAgents.use").to_string())
                    .on_click(cx.listener({
                        let id = row.id.clone();
                        move |this, _, _, cx| this.activate(&id, cx)
                    })),
            );
        }

        v_flex()
            .w_full()
            .py_3()
            .gap_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(header)
            .when(editing, |element| element.child(self.render_draft(cx)))
    }

    /// 编辑区内容。`is_new` 时不渲染归属信息（还没归属）。
    fn render_draft(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(draft) = self.draft.as_ref() else {
            return div().into_any_element();
        };
        let id = draft.id.clone();
        let is_new = draft.is_new;
        let is_user_defined = draft.is_user_defined;
        let command_label = if is_user_defined {
            t!("Settings.AcpAgents.command").to_string()
        } else {
            t!("Settings.AcpAgents.command_from_extension").to_string()
        };

        let mut form = v_flex().w_full().gap_3().pl_6();

        if is_user_defined {
            form = form
                .child(field_label(t!("Settings.AcpAgents.name").to_string(), cx))
                .child(Input::new(&draft.name).small())
                .child(field_label(command_label, cx))
                .child(Input::new(&draft.command).small());
        } else {
            form = form.child(field_label(command_label, cx)).child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(extension_command_hint(&self.rows, &id))),
            );
        }

        form = form
            .child(field_label(t!("Settings.AcpAgents.args").to_string(), cx))
            .child(Textarea::new(&draft.args))
            .child(field_label(t!("Settings.AcpAgents.env").to_string(), cx))
            .child(Textarea::new(&draft.env))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        Button::new("acp-agent-draft-save")
                            .small()
                            .primary()
                            .label(t!("Settings.AcpAgents.save").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.save_draft(cx))),
                    )
                    .child(
                        Button::new("acp-agent-draft-cancel")
                            .small()
                            .outline()
                            .label(t!("Settings.AcpAgents.cancel").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.draft = None;
                                this.save_error = None;
                                cx.notify();
                            })),
                    )
                    .when(!is_new, |element| {
                        element.child(
                            Button::new("acp-agent-draft-reset")
                                .small()
                                .outline()
                                .label(if is_user_defined {
                                    t!("Settings.AcpAgents.remove").to_string()
                                } else {
                                    t!("Settings.AcpAgents.reset").to_string()
                                })
                                .on_click(cx.listener({
                                    let id = id.clone();
                                    move |this, _, _, cx| {
                                        if is_user_defined {
                                            this.delete_user_agent(&id, cx);
                                        } else {
                                            this.reset_override(&id, cx);
                                        }
                                    }
                                })),
                        )
                    }),
            );

        form.into_any_element()
    }
}

impl Render for AcpAgentsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = v_flex().w_full();
        for row in &self.rows {
            let element = self.render_row(row, cx);
            rows = rows.child(element);
        }
        if self.rows.is_empty() {
            rows = rows.child(field_label(t!("Settings.AcpAgents.empty").to_string(), cx));
        }
        // 先把「新增」编辑区渲染成拥有所有权的元素，避免它和后面的闭包同时借 `cx`。
        let adding: Option<gpui::AnyElement> =
            if self.draft.as_ref().is_some_and(|draft| draft.is_new) {
                Some(self.render_draft(cx).into_any_element())
            } else {
                None
            };

        v_flex()
            .w_full()
            .gap_3()
            .child(self.render_header(cx))
            .when_some(self.load_error.clone(), |element, error| {
                element.child(error_line(error, cx))
            })
            .when_some(self.save_error.clone(), |element, error| {
                element.child(error_line(error, cx))
            })
            .child(rows)
            .child(
                h_flex().w_full().pt_1().gap_2().child(
                    Button::new("acp-agent-add")
                        .small()
                        .outline()
                        .label(t!("Settings.AcpAgents.add").to_string())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_draft_for(None, window, cx);
                        })),
                ),
            )
            .when_some(adding, |element, form| {
                element
                    .child(field_label(t!("Settings.AcpAgents.add_hint").to_string(), cx))
                    .child(form)
            })
    }
}

fn error_line(error: String, cx: &App) -> impl IntoElement {
    div()
        .text_xs()
        .text_color(cx.theme().danger)
        .child(SharedString::from(error))
}

fn field_label(label: String, cx: &App) -> impl IntoElement {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(SharedString::from(label))
}

fn status_dot(status: &RowStatus, cx: &App) -> impl IntoElement {
    let color = match status {
        RowStatus::Probing => cx.theme().muted_foreground,
        RowStatus::Ready => cx.theme().success,
        RowStatus::LoginRequired => cx.theme().warning,
        RowStatus::Failed(_) => cx.theme().danger,
        RowStatus::Unknown => cx.theme().border,
    };
    div()
        .flex_none()
        .pt_1()
        .child(div().size(px(8.)).rounded_full().bg(color))
}

/// 扩展 agent 的命令是扩展包内的相对路径，展示只读解析结果。
fn extension_command_hint(rows: &[AcpAgentRow], id: &str) -> String {
    rows.iter()
        .find(|row| row.id == id)
        .map(|row| row.command.clone())
        .unwrap_or_default()
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// 「X 前」文案：秒/分钟/小时/天。
fn format_age(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 60 {
        t!("Settings.AcpAgents.age_seconds", count = seconds).to_string()
    } else if seconds < 3600 {
        t!("Settings.AcpAgents.age_minutes", count = seconds / 60).to_string()
    } else if seconds < 86_400 {
        t!("Settings.AcpAgents.age_hours", count = seconds / 3600).to_string()
    } else {
        t!("Settings.AcpAgents.age_days", count = seconds / 86_400).to_string()
    }
}

/// 参数按行编辑：一行一个参数，去掉空行。
fn parse_lines(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// 环境变量按行编辑：一行一条 `KEY=VALUE`。
fn parse_env_lines(value: &str) -> Vec<(String, String)> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let (name, value) = line.split_once('=')?;
            let name = name.trim();
            (!name.is_empty()).then(|| (name.to_string(), value.trim().to_string()))
        })
        .collect()
}

fn join_lines(values: &[String]) -> String {
    values.join("\n")
}

fn join_env(values: &[(String, String)]) -> String {
    values
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 新增 agent 时缺 id：用命令名派生一个可读、稳定的 id。
fn command_slug(name: &str, command: &str) -> String {
    let source = if name.trim().is_empty() {
        command.trim()
    } else {
        name.trim()
    };
    let slug: String = source
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "acp-agent".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::{command_slug, format_age, parse_env_lines, parse_lines};

    #[test]
    fn args_are_edited_one_per_line() {
        assert_eq!(
            vec!["--stdio".to_string(), "--verbose".to_string()],
            parse_lines("--stdio\n\n  --verbose  \n")
        );
    }

    #[test]
    fn env_lines_skip_blanks_and_comments() {
        assert_eq!(
            vec![
                ("A".to_string(), "1".to_string()),
                ("B".to_string(), "two words".to_string()),
            ],
            parse_env_lines("# comment\nA=1\n\nB = two words\n")
        );
    }

    #[test]
    fn env_lines_drop_entries_without_a_name() {
        assert!(parse_env_lines("=value\n").is_empty());
    }

    #[test]
    fn slug_prefers_the_name_then_the_command() {
        assert_eq!("my-agent", command_slug("My Agent", "/usr/bin/foo"));
        assert_eq!("my-agent", command_slug("  ", "My-Agent"));
        assert_eq!("acp-agent", command_slug("", "///"));
    }

    #[test]
    fn age_labels_pick_the_coarsest_unit() {
        assert!(format_age(30).contains("30"));
        assert!(format_age(120).contains("2"));
        assert!(format_age(7_200).contains("2"));
        assert!(format_age(172_800).contains("2"));
    }
}
