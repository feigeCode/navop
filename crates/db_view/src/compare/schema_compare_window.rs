use std::collections::HashSet;
use std::sync::Arc;

use db::{DbNode, GlobalDbState};
use extension_component::DbSelectorKind;
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement,
    Render, Styled, Subscription, Task, Window, div, prelude::FluentBuilder,
};
use gpui_component::{
    ActiveTheme, Disableable, IconName, StyledExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::InputState,
    select::{SearchableVec, SelectEvent, SelectState},
    v_flex,
};
use rust_i18n::t;
use tokio::sync::mpsc;

use crate::compare::sync_statement_picker::{
    SyncStatementListState, default_selected_statement_ids, refresh_sync_statement_list,
    selected_sync_sql_text_for_ids, sync_statement_list_state,
};
use crate::compare::target_picker::{
    StringSelect, selected_string, set_connection_select, set_string_select, string_select_state,
};
use crate::compare::window_params::{SchemaCompareSelection, schema_compare_params};
use crate::compare::window_ui::{
    ConnectionSelectItem, close_button, connection_select_state, register_connection_for_compare,
    selected_connection_id, sql_editor_panel, start_sync_sql_execution, sync_sql_editor_state,
};
use crate::compare::{
    CompareProgress, CompareTargetScope, SchemaCompareParams, execute_schema_compare,
    generate_schema_sync_plan_for_target,
};
use crate::db_object_selector::{
    DbObjectSelectorPolicy, db_object_selector_panel, effective_database_schema,
    policy_for_connection,
};
use db::compare::{SchemaCompareResult, SyncPlan};

/// 结构比较弹出窗口
pub struct SchemaCompareWindow {
    pub(super) source_connection_id: Entity<InputState>,
    pub(super) source_connection_select: Entity<SelectState<SearchableVec<ConnectionSelectItem>>>,
    pub(super) source_database: Entity<InputState>,
    pub(super) source_database_select: StringSelect,
    pub(super) source_schema: Entity<InputState>,
    pub(super) source_schema_select: StringSelect,
    pub(super) target_connection_id: Entity<InputState>,
    pub(super) target_connection_select: Entity<SelectState<SearchableVec<ConnectionSelectItem>>>,
    pub(super) target_database: Entity<InputState>,
    pub(super) target_database_select: StringSelect,
    pub(super) target_schema: Entity<InputState>,
    pub(super) target_schema_select: StringSelect,
    pub(super) result: Entity<Option<SchemaCompareResult>>,
    pub(super) sync_plan: Entity<Option<SyncPlan>>,
    pub(super) selected_statement_ids: Entity<HashSet<String>>,
    pub(super) sync_statement_list: SyncStatementListState,
    pub(super) sync_sql_editor: Entity<InputState>,
    pub(super) progress: Entity<Option<CompareProgress>>,
    compare_target: Entity<Option<CompareTargetScope>>,
    pub(super) status: Entity<String>,
    is_running: Entity<bool>,
    is_executing: Entity<bool>,
    compare_task: Option<Task<()>>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl SchemaCompareWindow {
    pub fn new(source_node: DbNode, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let default_database = source_node.get_database_name().unwrap_or_default();
        let default_schema = source_node.get_schema_name().unwrap_or_default();
        let default_policy = db_object_policy_for_source(&source_node, cx);
        let default_database = if default_policy.schema_as_database {
            default_schema.clone()
        } else {
            default_database
        };
        let default_schema = if default_policy.show_schema {
            default_schema
        } else {
            String::new()
        };

        let source_connection_id = cx
            .new(|cx| InputState::new(window, cx).default_value(source_node.connection_id.clone()));
        let source_connection_select =
            connection_select_state(&source_node.connection_id, window, cx);
        let source_database =
            cx.new(|cx| InputState::new(window, cx).default_value(default_database.clone()));
        let source_database_select = string_select_state(default_database.clone(), window, cx);
        let source_schema =
            cx.new(|cx| InputState::new(window, cx).default_value(default_schema.clone()));
        let source_schema_select = string_select_state(default_schema.clone(), window, cx);
        let target_connection_id = cx
            .new(|cx| InputState::new(window, cx).default_value(source_node.connection_id.clone()));
        let target_connection_select =
            connection_select_state(&source_node.connection_id, window, cx);
        let target_database =
            cx.new(|cx| InputState::new(window, cx).default_value(default_database.clone()));
        let target_database_select = string_select_state(default_database.clone(), window, cx);
        let target_schema =
            cx.new(|cx| InputState::new(window, cx).default_value(default_schema.clone()));
        let target_schema_select = string_select_state(default_schema.clone(), window, cx);
        let sync_sql_editor = sync_sql_editor_state(window, cx);

        let view = cx.new(|cx: &mut Context<Self>| {
            let selected_statement_ids = cx.new(|_| HashSet::new());
            let sync_statement_list =
                sync_statement_list_state(selected_statement_ids.clone(), window, cx);
            let mut window_state = Self {
                source_connection_id,
                source_connection_select,
                source_database,
                source_database_select,
                source_schema,
                source_schema_select,
                target_connection_id,
                target_connection_select,
                target_database,
                target_database_select,
                target_schema,
                target_schema_select,
                sync_sql_editor,
                result: cx.new(|_| None),
                sync_plan: cx.new(|_| None),
                selected_statement_ids,
                sync_statement_list,
                progress: cx.new(|_| None),
                compare_target: cx.new(|_| None),
                status: cx.new(|_| t!("Compare.ready").to_string()),
                is_running: cx.new(|_| false),
                is_executing: cx.new(|_| false),
                compare_task: None,
                focus_handle: cx.focus_handle(),
                _subscriptions: Vec::new(),
            };
            // 选中语句变化时(比较完成、勾选、批量选择)刷新 SQL 编辑器内容
            let sub = cx.observe_in(
                &window_state.selected_statement_ids,
                window,
                |this, _, window, cx| {
                    this.refresh_sync_editor(window, cx);
                    this.sync_statement_list.update(cx, |_, cx| cx.notify());
                },
            );
            window_state._subscriptions.push(sub);
            // 源级联:连接 → 数据库 → Schema
            window_state._subscriptions.push(cx.subscribe(
                &window_state.source_connection_select,
                |this, _, _event: &SelectEvent<SearchableVec<ConnectionSelectItem>>, cx| {
                    this.load_source_databases(cx);
                },
            ));
            window_state._subscriptions.push(cx.subscribe(
                &window_state.source_database_select,
                |this, _, _event: &SelectEvent<SearchableVec<String>>, cx| {
                    this.load_source_schemas(cx);
                },
            ));
            // 目标级联:连接 → 数据库 → Schema
            window_state._subscriptions.push(cx.subscribe(
                &window_state.target_connection_select,
                |this, _, _event: &SelectEvent<SearchableVec<ConnectionSelectItem>>, cx| {
                    this.load_target_databases(cx);
                },
            ));
            window_state._subscriptions.push(cx.subscribe(
                &window_state.target_database_select,
                |this, _, _event: &SelectEvent<SearchableVec<String>>, cx| {
                    this.load_target_schemas(cx);
                },
            ));
            window_state
        });
        view.update(cx, |this, cx| {
            if !selected_connection_id(
                &this.source_connection_select,
                &this.source_connection_id,
                cx,
            )
            .trim()
            .is_empty()
            {
                this.load_source_databases(cx);
            }
            if !selected_connection_id(
                &this.target_connection_select,
                &this.target_connection_id,
                cx,
            )
            .trim()
            .is_empty()
            {
                this.load_target_databases(cx);
            }
        });
        view
    }

    pub fn popup_title_for(source_node: &DbNode) -> String {
        t!(
            "Compare.schema_compare_title",
            name = source_node.name.clone()
        )
        .to_string()
    }

    fn start_compare(&mut self, cx: &mut Context<Self>) {
        let params = match self.build_params(cx) {
            Ok(params) => params,
            Err(message) => {
                self.set_status(message, cx);
                return;
            }
        };
        let compare_target = CompareTargetScope::from_schema_params(&params);
        register_connection_for_compare(&params.source_connection_id, cx);
        register_connection_for_compare(&params.target_connection_id, cx);
        let target_connection_id = params.target_connection_id.clone();
        let target_database = params.target_database.clone();
        let target_schema = params.target_schema.clone();
        let db_state = Arc::new(cx.global::<GlobalDbState>().clone());
        self.is_running.update(cx, |running, cx| {
            *running = true;
            cx.notify();
        });
        self.set_progress(
            Some(CompareProgress::phase(
                t!("Compare.preparing_compare").to_string(),
            )),
            cx,
        );
        self.set_status(t!("Compare.comparing_schema").to_string(), cx);

        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<CompareProgress>();

        // 进度接收循环:随执行器任务结束(发送端关闭)而退出
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            while let Some(progress) = progress_rx.recv().await {
                if this
                    .update(cx, |view, cx| view.set_progress(Some(progress), cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result =
                match execute_schema_compare(params, db_state.clone(), progress_tx, cx).await {
                    Ok(result) => generate_schema_sync_plan_for_target(
                        &result,
                        &db_state,
                        &target_connection_id,
                        &target_database,
                        target_schema.as_deref(),
                    )
                    .map(|plan| (result, plan)),
                    Err(error) => Err(error),
                };
            let _ = this.update(cx, |view, cx| {
                view.is_running.update(cx, |running, cx| {
                    *running = false;
                    cx.notify();
                });
                view.set_progress(None, cx);
                match result {
                    Ok((result, plan)) => {
                        let selected_ids = default_selected_statement_ids(&plan);
                        refresh_sync_statement_list(&view.sync_statement_list, &plan, cx);
                        view.result.update(cx, |slot, cx| {
                            *slot = Some(result);
                            cx.notify();
                        });
                        view.sync_plan.update(cx, |slot, cx| {
                            *slot = Some(plan);
                            cx.notify();
                        });
                        view.selected_statement_ids.update(cx, |slot, cx| {
                            *slot = selected_ids;
                            cx.notify();
                        });
                        view.compare_target.update(cx, |slot, cx| {
                            *slot = Some(compare_target);
                            cx.notify();
                        });
                        view.set_status(t!("Compare.schema_compare_complete").to_string(), cx);
                    }
                    Err(error) => view.set_status(
                        t!("Compare.compare_failed", error = error.to_string()).to_string(),
                        cx,
                    ),
                }
                cx.notify();
            });
        });
        self.compare_task = Some(task);
    }

    fn swap_source_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self.source_selection(cx);
        let target = self.target_selection(cx);

        set_connection_select(
            &self.source_connection_select,
            &target.connection_id,
            window,
            cx,
        );
        set_connection_select(
            &self.target_connection_select,
            &source.connection_id,
            window,
            cx,
        );
        set_string_select(
            &self.source_database_select,
            &self.source_database,
            target.database,
            window,
            cx,
        );
        set_string_select(
            &self.target_database_select,
            &self.target_database,
            source.database,
            window,
            cx,
        );
        set_string_select(
            &self.source_schema_select,
            &self.source_schema,
            target.schema,
            window,
            cx,
        );
        set_string_select(
            &self.target_schema_select,
            &self.target_schema,
            source.schema,
            window,
            cx,
        );

        self.result.update(cx, |slot, cx| {
            *slot = None;
            cx.notify();
        });
        self.sync_plan.update(cx, |slot, cx| {
            *slot = None;
            cx.notify();
        });
        self.compare_target.update(cx, |slot, cx| {
            *slot = None;
            cx.notify();
        });
        self.set_status(t!("Compare.swapped_source_target").to_string(), cx);
    }

    fn cancel_compare(&mut self, cx: &mut Context<Self>) {
        // 丢弃任务句柄即取消执行器 future,并关闭进度通道
        self.compare_task = None;
        self.is_running.update(cx, |running, cx| {
            *running = false;
            cx.notify();
        });
        self.set_progress(None, cx);
        self.set_status(t!("Compare.cancelled").to_string(), cx);
        cx.notify();
    }

    /// 根据当前选中的语句刷新 SQL 编辑器内容(可被用户后续手动编辑)
    fn refresh_sync_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        let sql = self.selected_sync_sql(cx);
        self.sync_sql_editor.update(cx, |state, cx| {
            state.set_value(sql, window, cx);
        });
    }

    fn build_params(&self, cx: &mut Context<Self>) -> Result<SchemaCompareParams, &'static str> {
        schema_compare_params(self.source_selection(cx), self.target_selection(cx))
    }

    fn start_execute_sync_sql(&mut self, cx: &mut Context<Self>) {
        start_sync_sql_execution(
            self.compare_target.read(cx).clone(),
            self.editor_sql(cx),
            self.status.clone(),
            self.is_executing.clone(),
            cx,
        );
    }

    /// 编辑器中实际待执行的 SQL(用户可能已手动修改)
    fn editor_sql(&self, cx: &Context<Self>) -> String {
        self.sync_sql_editor.read(cx).text().to_string()
    }

    /// 由选中语句生成的 SQL,用于填充编辑器
    fn selected_sync_sql(&self, cx: &Context<Self>) -> String {
        let selected_ids = self.selected_statement_ids.read(cx);
        self.sync_plan
            .read(cx)
            .as_ref()
            .map_or_else(String::new, |plan| {
                selected_sync_sql_text_for_ids(plan, selected_ids)
            })
    }

    fn has_editor_sql(&self, cx: &Context<Self>) -> bool {
        !self.editor_sql(cx).trim().is_empty()
    }

    fn set_progress(&self, progress: Option<CompareProgress>, cx: &mut Context<Self>) {
        self.progress.update(cx, |slot, cx| {
            *slot = progress;
            cx.notify();
        });
    }

    fn set_status(&self, status: impl Into<String>, cx: &mut Context<Self>) {
        self.status.update(cx, |value, cx| {
            *value = status.into();
            cx.notify();
        });
    }

    fn render_source(&self, cx: &mut Context<Self>) -> impl IntoElement {
        db_object_selector_panel(
            t!("Compare.source").to_string(),
            DbSelectorKind::Schema,
            self.source_controls(cx),
            cx,
        )
    }

    fn source_selection(&self, cx: &Context<Self>) -> SchemaCompareSelection {
        let database = selected_string(&self.source_database_select, &self.source_database, cx);
        let schema = selected_string(&self.source_schema_select, &self.source_schema, cx);
        let (database, schema) = effective_database_schema(
            database,
            schema,
            policy_for_connection(&self.source_connection_controls(), cx),
        );
        SchemaCompareSelection {
            connection_id: selected_connection_id(
                &self.source_connection_select,
                &self.source_connection_id,
                cx,
            ),
            database,
            schema,
        }
    }

    fn target_selection(&self, cx: &Context<Self>) -> SchemaCompareSelection {
        let database = selected_string(&self.target_database_select, &self.target_database, cx);
        let schema = selected_string(&self.target_schema_select, &self.target_schema, cx);
        let (database, schema) = effective_database_schema(
            database,
            schema,
            policy_for_connection(&self.connection_controls(), cx),
        );
        SchemaCompareSelection {
            connection_id: selected_connection_id(
                &self.target_connection_select,
                &self.target_connection_id,
                cx,
            ),
            database,
            schema,
        }
    }
}

fn db_object_policy_for_source(source_node: &DbNode, cx: &mut App) -> DbObjectSelectorPolicy {
    cx.try_global::<GlobalDbState>()
        .map(|db_state| {
            DbObjectSelectorPolicy::from_capabilities(
                &db_state.capabilities(&source_node.database_type),
            )
        })
        .unwrap_or_default()
}

impl Focusable for SchemaCompareWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SchemaCompareWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_running = *self.is_running.read(cx);
        let is_executing = *self.is_executing.read(cx);
        let has_sync_sql = self.has_editor_sql(cx);
        let status = self.status.read(cx).clone();
        let editor_sql = self.sync_sql_editor.read(cx).text().to_string();

        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .child(
                div()
                    .font_semibold()
                    .child(t!("Compare.schema_compare").to_string()),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .child(
                        // 第一排:源和目标并排
                        h_flex()
                            .gap_4()
                            .child(div().flex_1().child(self.render_source(cx)))
                            .child(
                                div().pt_10().child(
                                    Button::new("swap-schema-compare-source-target")
                                        .icon(IconName::Replace)
                                        .tooltip(t!("Compare.swap_source_target").to_string())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.swap_source_target(window, cx);
                                        })),
                                ),
                            )
                            .child(div().flex_1().child(self.render_target(cx))),
                    )
                    .child(
                        // 第二排:结构结果和 SQL 并排,各自内部滚动
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .gap_4()
                            .child(
                                div()
                                    .flex_1()
                                    .h_full()
                                    .min_h_0()
                                    .child(self.render_result_meta(cx)),
                            )
                            .child(div().flex_1().h_full().min_h_0().child(sql_editor_panel(
                                "schema-compare-copy-sql",
                                &self.sync_sql_editor,
                                editor_sql,
                                cx,
                            ))),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(status),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(close_button())
                            .when(is_running, |this| {
                                this.child(
                                    Button::new("cancel-compare")
                                        .danger()
                                        .child(t!("Common.cancel").to_string())
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.cancel_compare(cx);
                                        })),
                                )
                            })
                            .child(
                                Button::new("execute-sync-sql")
                                    .child(t!("Compare.execute_sql").to_string())
                                    .loading(is_executing)
                                    .disabled(is_running || is_executing || !has_sync_sql)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.start_execute_sync_sql(cx);
                                    })),
                            )
                            .child(
                                Button::new("compare")
                                    .primary()
                                    .loading(is_running)
                                    .disabled(is_running || is_executing)
                                    .child(t!("Compare.start_compare").to_string())
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.start_compare(cx);
                                    })),
                            ),
                    ),
            )
    }
}
