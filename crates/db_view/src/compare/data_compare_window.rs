use std::collections::HashSet;
use std::sync::Arc;

use db::{DbNode, GlobalDbState};
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement,
    Render, Styled, Subscription, Task, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Disableable, StyledExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::InputState,
    select::{SearchableVec, SelectEvent, SelectState},
    v_flex,
};
use tokio::sync::mpsc;

use crate::compare::sync_statement_picker::{
    default_selected_statement_ids, selected_sync_sql_text_for_ids,
};
use crate::compare::target_picker::{StringSelect, selected_string, string_select_state};
use crate::compare::window_params::{DataCompareSelection, data_compare_params};
use crate::compare::window_ui::{
    ConnectionSelectItem, close_button, connection_select_state, selected_connection_id,
    sql_editor_panel, start_sync_sql_execution, sync_sql_editor_state,
};
use crate::compare::{
    CompareProgress, CompareTargetScope, DataCompareParams, execute_data_compare,
    generate_data_sync_plan,
};
use db::compare::{DataCompareResult, SyncPlan};

pub struct DataCompareWindow {
    pub(super) source_connection_id: Entity<InputState>,
    pub(super) source_connection_select: Entity<SelectState<Vec<ConnectionSelectItem>>>,
    pub(super) source_database: Entity<InputState>,
    pub(super) source_database_select: StringSelect,
    pub(super) source_schema: Entity<InputState>,
    pub(super) source_schema_select: StringSelect,
    pub(super) source_table: Entity<InputState>,
    pub(super) source_table_select: StringSelect,
    pub(super) target_connection_id: Entity<InputState>,
    pub(super) target_connection_select: Entity<SelectState<Vec<ConnectionSelectItem>>>,
    pub(super) target_database: Entity<InputState>,
    pub(super) target_database_select: StringSelect,
    pub(super) target_schema: Entity<InputState>,
    pub(super) target_schema_select: StringSelect,
    pub(super) target_table: Entity<InputState>,
    pub(super) target_table_select: StringSelect,
    pub(super) key_columns: Entity<InputState>,
    pub(super) result: Entity<Option<DataCompareResult>>,
    pub(super) sync_plan: Entity<Option<SyncPlan>>,
    pub(super) selected_statement_ids: Entity<HashSet<String>>,
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

impl DataCompareWindow {
    pub fn new(source_node: DbNode, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let default_database = source_node.get_database_name().unwrap_or_default();
        let default_schema = source_node.get_schema_name().unwrap_or_default();
        let default_table = source_node
            .get_table_name()
            .unwrap_or_else(|| source_node.name.clone());

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
        let source_table =
            cx.new(|cx| InputState::new(window, cx).default_value(default_table.clone()));
        let source_table_select = string_select_state(default_table.clone(), window, cx);
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
        let target_table =
            cx.new(|cx| InputState::new(window, cx).default_value(default_table.clone()));
        let target_table_select = string_select_state(default_table.clone(), window, cx);
        let key_columns = cx.new(|cx| InputState::new(window, cx).placeholder("id, tenant_id"));
        let sync_sql_editor = sync_sql_editor_state(window, cx);

        let view = cx.new(|cx: &mut Context<Self>| {
            let mut window_state = Self {
                source_connection_id,
                source_connection_select,
                source_database,
                source_database_select,
                source_schema,
                source_schema_select,
                source_table,
                source_table_select,
                target_connection_id,
                target_connection_select,
                target_database,
                target_database_select,
                target_schema,
                target_schema_select,
                target_table,
                target_table_select,
                key_columns,
                sync_sql_editor,
                result: cx.new(|_| None),
                sync_plan: cx.new(|_| None),
                selected_statement_ids: cx.new(|_| HashSet::new()),
                progress: cx.new(|_| None),
                compare_target: cx.new(|_| None),
                status: cx.new(|_| "就绪".to_string()),
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
                },
            );
            window_state._subscriptions.push(sub);
            // 源级联:连接 → 数据库 → Schema → 表
            window_state._subscriptions.push(cx.subscribe(
                &window_state.source_connection_select,
                |this, _, _event: &SelectEvent<Vec<ConnectionSelectItem>>, cx| {
                    this.load_source_databases(cx);
                },
            ));
            window_state._subscriptions.push(cx.subscribe(
                &window_state.source_database_select,
                |this, _, _event: &SelectEvent<SearchableVec<String>>, cx| {
                    this.load_source_schemas(cx);
                },
            ));
            window_state._subscriptions.push(cx.subscribe(
                &window_state.source_schema_select,
                |this, _, _event: &SelectEvent<SearchableVec<String>>, cx| {
                    this.load_source_tables(cx);
                },
            ));
            // 目标级联:连接 → 数据库 → Schema → 表
            window_state._subscriptions.push(cx.subscribe(
                &window_state.target_connection_select,
                |this, _, _event: &SelectEvent<Vec<ConnectionSelectItem>>, cx| {
                    this.load_target_databases(cx);
                },
            ));
            window_state._subscriptions.push(cx.subscribe(
                &window_state.target_database_select,
                |this, _, _event: &SelectEvent<SearchableVec<String>>, cx| {
                    this.load_target_schemas(cx);
                },
            ));
            window_state._subscriptions.push(cx.subscribe(
                &window_state.target_schema_select,
                |this, _, _event: &SelectEvent<SearchableVec<String>>, cx| {
                    this.load_target_tables(cx);
                },
            ));
            window_state
        });
        // 打开时按默认连接预加载源/目标数据库,后续逐级联动
        view.update(cx, |this, cx| {
            this.load_source_databases(cx);
            this.load_target_databases(cx);
        });
        view
    }

    pub fn popup_title_for(source_node: &DbNode) -> String {
        format!("数据比较 - {}", source_node.name)
    }

    fn start_compare(&mut self, cx: &mut Context<Self>) {
        let params = match self.build_params(cx) {
            Ok(params) => params,
            Err(message) => {
                self.set_status(message, cx);
                return;
            }
        };
        let compare_target = CompareTargetScope::from_data_params(&params);
        let db_state = Arc::new(cx.global::<GlobalDbState>().clone());
        self.is_running.update(cx, |running, cx| {
            *running = true;
            cx.notify();
        });
        self.set_progress(Some(CompareProgress::phase("正在准备比较…")), cx);
        self.set_status("正在比较数据…", cx);

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
            let result = execute_data_compare(params, db_state, progress_tx, cx).await;
            let _ = this.update(cx, |view, cx| {
                view.is_running.update(cx, |running, cx| {
                    *running = false;
                    cx.notify();
                });
                view.set_progress(None, cx);
                match result {
                    Ok(result) => {
                        let plan = generate_data_sync_plan(&result);
                        let selected_ids = default_selected_statement_ids(&plan);
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
                        view.set_status("数据比较完成", cx);
                    }
                    Err(error) => view.set_status(format!("比较失败:{error}"), cx),
                }
                cx.notify();
            });
        });
        self.compare_task = Some(task);
    }

    fn cancel_compare(&mut self, cx: &mut Context<Self>) {
        // 丢弃任务句柄即取消执行器 future,并关闭进度通道
        self.compare_task = None;
        self.is_running.update(cx, |running, cx| {
            *running = false;
            cx.notify();
        });
        self.set_progress(None, cx);
        self.set_status("已取消", cx);
        cx.notify();
    }

    /// 根据当前选中的语句刷新 SQL 编辑器内容(可被用户后续手动编辑)
    fn refresh_sync_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        let sql = self.selected_sync_sql(cx);
        self.sync_sql_editor.update(cx, |state, cx| {
            state.set_value(sql, window, cx);
        });
    }

    fn build_params(&self, cx: &mut Context<Self>) -> Result<DataCompareParams, &'static str> {
        data_compare_params(
            self.source_selection(cx),
            self.target_selection(cx),
            self.key_columns.read(cx).text().to_string(),
        )
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

    fn source_selection(&self, cx: &Context<Self>) -> DataCompareSelection {
        DataCompareSelection {
            connection_id: selected_connection_id(
                &self.source_connection_select,
                &self.source_connection_id,
                cx,
            ),
            database: selected_string(&self.source_database_select, &self.source_database, cx),
            schema: selected_string(&self.source_schema_select, &self.source_schema, cx),
            table: selected_string(&self.source_table_select, &self.source_table, cx),
        }
    }

    fn target_selection(&self, cx: &Context<Self>) -> DataCompareSelection {
        DataCompareSelection {
            connection_id: selected_connection_id(
                &self.target_connection_select,
                &self.target_connection_id,
                cx,
            ),
            database: selected_string(&self.target_database_select, &self.target_database, cx),
            schema: selected_string(&self.target_schema_select, &self.target_schema, cx),
            table: selected_string(&self.target_table_select, &self.target_table, cx),
        }
    }
}

impl Focusable for DataCompareWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DataCompareWindow {
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
            .child(div().font_semibold().child("数据比较"))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .child(
                        // 左栏:配置固定,同步语句列表单独滚动
                        div().w(px(360.0)).h_full().min_h_0().child(
                            v_flex()
                                .size_full()
                                .gap_3()
                                .child(self.render_source(cx))
                                .child(self.render_target(cx))
                                .child(self.render_result_meta(cx)),
                        ),
                    )
                    .child(
                        // 右栏:可编辑 SQL 编辑器(填满高度,内部滚动)
                        div().flex_1().h_full().min_h_0().child(sql_editor_panel(
                            "data-compare-copy-sql",
                            &self.sync_sql_editor,
                            editor_sql,
                            cx,
                        )),
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
                                        .child("取消")
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.cancel_compare(cx);
                                        })),
                                )
                            })
                            .child(
                                Button::new("execute-sync-sql")
                                    .child("执行 SQL")
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
                                    .child("开始比较")
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.start_compare(cx);
                                    })),
                            ),
                    ),
            )
    }
}
