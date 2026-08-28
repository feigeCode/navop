//! 全局后台任务面板。
//!
//! 该组件负责：
//! - 在 TabContainer 标签栏最右侧渲染任务入口按钮（带运行数 / 失败数徽标）
//! - 点击弹出 Dialog 弹窗，展示任务列表、状态、进度和错误
//! - 订阅全局任务管理器，在任务状态变化时自动刷新
//!
//! 该组件不负责调度任务，仅作为 [`crate::background_tasks`] 注册表的全局视图。

use crate::background_tasks::{
    BackgroundTask, BackgroundTaskCounts, BackgroundTaskFilter, BackgroundTaskManager,
    BackgroundTaskStatus, global,
};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled, Subscription,
    Window, prelude::FluentBuilder, px,
};
use gpui_component::progress::Progress;
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use rust_i18n::t;
use std::sync::Arc;

/// 面板状态实体，由 `TabContainer` 持有一个实例。
pub struct BackgroundTaskPanel {
    manager: Entity<BackgroundTaskManager>,
    _subscription: Subscription,
}

impl BackgroundTaskPanel {
    /// 创建面板并订阅全局任务管理器。若管理器尚未初始化，会立即创建。
    pub fn new(cx: &mut Context<Self>) -> Self {
        let manager = global(cx);
        let subscription = cx.subscribe(&manager, |_this, _entity, _event, cx| {
            // 任务变化时刷新入口按钮的徽标计数。
            cx.notify();
        });
        Self {
            manager,
            _subscription: subscription,
        }
    }

    /// 获取任务管理器实体，供集成方更新任务状态。
    pub fn manager(&self) -> Entity<BackgroundTaskManager> {
        self.manager.clone()
    }

    /// 读取任务数量聚合。
    pub fn counts(&self, cx: &App) -> BackgroundTaskCounts {
        self.manager.read(cx).counts()
    }

    /// 渲染完整入口（Button）。作为标签栏末尾的固定 flex item。
    fn render_entry(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let counts = self.manager.read(cx).counts();
        gpui::div()
            .relative()
            .child(
                Button::new("background-task-button")
                    .icon(IconName::ListChecks)
                    .ghost()
                    .compact()
                    .tooltip(t!("BackgroundTasks.open").to_string())
                    .when(counts.active > 0 || counts.failed > 0, |btn| {
                        btn.label(Self::badge_text(&counts))
                    })
                    .on_click({
                        let manager = self.manager.clone();
                        move |_, window, cx| {
                            open_background_task_dialog(manager.clone(), window, cx);
                        }
                    }),
            )
            .when(Self::shows_running_dot(&counts), |entry| {
                entry.child(
                    gpui::div()
                        .absolute()
                        .top(px(2.0))
                        .right(px(2.0))
                        .size(px(6.0))
                        .rounded_full()
                        .bg(cx.theme().danger),
                )
            })
    }

    fn badge_text(counts: &BackgroundTaskCounts) -> String {
        let active = counts.queued + counts.running + counts.cancelling;
        if counts.failed > 0 {
            format!("{active} / {}", counts.failed)
        } else {
            active.to_string()
        }
    }

    fn shows_running_dot(counts: &BackgroundTaskCounts) -> bool {
        counts.active > 0
    }
}

impl Render for BackgroundTaskPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_entry(cx)
    }
}

/// 打开后台任务 Dialog 弹窗。
///
/// 与标签页切换器（`crate::tab_switcher::open_tab_switcher_dialog`）保持一致，
/// 通过 `window.open_dialog` 打开模态弹窗。弹窗内容由独立实体渲染，
/// 该实体订阅任务管理器，任务状态变化时内容自动刷新。
pub fn open_background_task_dialog(
    manager: Entity<BackgroundTaskManager>,
    window: &mut Window,
    cx: &mut App,
) {
    let panel_content = cx.new(|cx| BackgroundTaskPanelContent::new(manager, cx));
    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .w(px(560.0))
            .margin_top(px(72.0))
            .title(t!("BackgroundTasks.title").to_string())
            .content({
                let panel_content = panel_content.clone();
                move |content, _window, _cx| content.p_0().child(panel_content.clone())
            })
    });
}

/// Dialog 弹窗内容实体：订阅任务管理器，任务变化时刷新内容。
///
/// 持有当前过滤条件，关闭再打开后保留上次选择。
struct BackgroundTaskPanelContent {
    manager: Entity<BackgroundTaskManager>,
    filter: BackgroundTaskFilter,
    _subscription: Subscription,
}

impl BackgroundTaskPanelContent {
    fn new(manager: Entity<BackgroundTaskManager>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&manager, |_this, _entity, _event, cx| {
            cx.notify();
        });
        Self {
            manager,
            filter: BackgroundTaskFilter::All,
            _subscription: subscription,
        }
    }

    fn update_filter(&mut self, filter: BackgroundTaskFilter) {
        self.filter = filter;
    }
}

impl Render for BackgroundTaskPanelContent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.entity();
        let filter = self.filter;
        render_panel_content(
            &self.manager,
            filter,
            std::sync::Arc::new(move |filter, cx: &mut App| {
                panel.update(cx, |this: &mut BackgroundTaskPanelContent, cx| {
                    this.update_filter(filter);
                    cx.notify();
                });
            }),
            cx,
        )
    }
}

/// 渲染弹窗内容（不含入口按钮）。
pub(crate) fn render_panel_content(
    manager: &Entity<BackgroundTaskManager>,
    filter: BackgroundTaskFilter,
    update_filter: Arc<dyn Fn(BackgroundTaskFilter, &mut App)>,
    cx: &App,
) -> impl IntoElement {
    let tasks = manager.read(cx).filtered_tasks(filter);
    let counts = manager.read(cx).counts();
    let manager = manager.clone();

    v_flex()
        .id("background-task-dialog-content")
        .w_full()
        .max_h(px(500.0))
        .gap_2()
        .p_2()
        .child(render_header(
            counts,
            manager.clone(),
            filter,
            update_filter,
            cx,
        ))
        .when(tasks.is_empty(), |this| {
            this.child(
                v_flex()
                    .id("background-task-empty")
                    .py_8()
                    .items_center()
                    .justify_center()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("BackgroundTasks.no_tasks").to_string()),
            )
        })
        .when(!tasks.is_empty(), |this| {
            this.child(
                v_flex()
                    .id("background-task-list")
                    .gap_2()
                    .max_h(px(400.0))
                    .overflow_y_scroll()
                    .children(
                        tasks
                            .iter()
                            .map(|task| render_task_item(task, manager.clone(), cx)),
                    ),
            )
        })
}

fn render_header(
    counts: BackgroundTaskCounts,
    manager: Entity<BackgroundTaskManager>,
    filter: BackgroundTaskFilter,
    update_filter: Arc<dyn Fn(BackgroundTaskFilter, &mut App)>,
    cx: &App,
) -> impl IntoElement {
    let has_cancellable_tasks = manager
        .read(cx)
        .tasks()
        .iter()
        .any(BackgroundTask::can_cancel);
    h_flex()
        .id("background-task-header")
        .items_center()
        .gap_2()
        .child(render_counts_badge(counts))
        .child(render_filter_bar(filter, update_filter))
        .child(
            Button::new("background-task-cancel-all-btn")
                .label(t!("BackgroundTasks.cancel_all").to_string())
                .ghost()
                .small()
                .danger()
                .disabled(!has_cancellable_tasks)
                .on_click({
                    let manager = manager.clone();
                    move |_, _window, cx| {
                        manager.update(cx, |manager, cx| manager.cancel_all_active(cx));
                    }
                }),
        )
        .child(
            Button::new("background-task-clear-btn")
                .label(t!("BackgroundTasks.clear_finished").to_string())
                .ghost()
                .small()
                .disabled(counts.succeeded + counts.failed + counts.cancelled == 0)
                .on_click(move |_, _window, cx| {
                    manager.update(cx, |manager, cx| manager.clear_finished(cx));
                }),
        )
}

fn render_filter_bar(
    filter: BackgroundTaskFilter,
    update_filter: Arc<dyn Fn(BackgroundTaskFilter, &mut App)>,
) -> impl IntoElement {
    h_flex()
        .id("background-task-filters")
        .gap_1()
        .child(render_filter_button(
            "all",
            t!("BackgroundTasks.filter_all").to_string(),
            BackgroundTaskFilter::All,
            filter,
            update_filter.clone(),
        ))
        .child(render_filter_button(
            "active",
            t!("BackgroundTasks.filter_active").to_string(),
            BackgroundTaskFilter::Active,
            filter,
            update_filter.clone(),
        ))
        .child(render_filter_button(
            "finished",
            t!("BackgroundTasks.filter_finished").to_string(),
            BackgroundTaskFilter::Finished,
            filter,
            update_filter.clone(),
        ))
        .child(render_filter_button(
            "failed",
            t!("BackgroundTasks.filter_failed").to_string(),
            BackgroundTaskFilter::Failed,
            filter,
            update_filter,
        ))
}

fn render_filter_button(
    id: &str,
    label: String,
    target: BackgroundTaskFilter,
    current: BackgroundTaskFilter,
    update_filter: Arc<dyn Fn(BackgroundTaskFilter, &mut App)>,
) -> AnyElement {
    let mut button = Button::new(SharedString::from(format!("background-task-filter-{id}")))
        .label(label)
        .ghost()
        .small();
    if current == target {
        button = button.primary();
    } else {
        button = button.on_click(move |_, _window, cx| update_filter(target, cx));
    }
    button.into_any_element()
}

fn render_counts_badge(counts: BackgroundTaskCounts) -> impl IntoElement {
    h_flex()
        .id("background-task-counts")
        .gap_1()
        .text_xs()
        .when(counts.queued > 0, |this| {
            this.child(render_count_chip(
                "queued",
                &t!("BackgroundTasks.queued").to_string(),
                counts.queued,
            ))
        })
        .when(counts.running > 0, |this| {
            this.child(render_count_chip(
                "running",
                &t!("BackgroundTasks.running").to_string(),
                counts.running,
            ))
        })
        .when(counts.cancelling > 0, |this| {
            this.child(render_count_chip(
                "cancelling",
                &t!("BackgroundTasks.cancelling").to_string(),
                counts.cancelling,
            ))
        })
        .when(counts.failed > 0, |this| {
            this.child(render_count_chip(
                "failed",
                &t!("BackgroundTasks.failed").to_string(),
                counts.failed,
            ))
        })
}

fn render_count_chip(id: &str, label: &str, count: usize) -> AnyElement {
    let id = SharedString::from(format!("background-task-count-{id}"));
    gpui::div()
        .id(id)
        .px_1()
        .rounded_sm()
        .text_xs()
        .child(format!("{label} {count}"))
        .into_any_element()
}

fn render_task_item(
    task: &BackgroundTask,
    manager: Entity<BackgroundTaskManager>,
    cx: &App,
) -> impl IntoElement {
    let status_icon = match task.status {
        BackgroundTaskStatus::Queued => Icon::new(IconName::Inbox).small(),
        BackgroundTaskStatus::Running => Icon::new(IconName::LoaderCircle).small(),
        BackgroundTaskStatus::Cancelling => Icon::new(IconName::LoaderCircle).small(),
        BackgroundTaskStatus::Succeeded => Icon::new(IconName::CircleCheck).small(),
        BackgroundTaskStatus::Failed => Icon::new(IconName::CircleX).small(),
        BackgroundTaskStatus::Cancelled => Icon::new(IconName::CircleX).small(),
    };
    let status_text = match task.status {
        BackgroundTaskStatus::Queued => t!("BackgroundTasks.queued").to_string(),
        BackgroundTaskStatus::Running => t!("BackgroundTasks.running").to_string(),
        BackgroundTaskStatus::Cancelling => t!("BackgroundTasks.cancelling").to_string(),
        BackgroundTaskStatus::Succeeded => t!("BackgroundTasks.succeeded").to_string(),
        BackgroundTaskStatus::Failed => t!("BackgroundTasks.failed").to_string(),
        BackgroundTaskStatus::Cancelled => t!("BackgroundTasks.cancelled").to_string(),
    };
    let status_color = match task.status {
        BackgroundTaskStatus::Queued => cx.theme().muted_foreground,
        BackgroundTaskStatus::Running => cx.theme().info,
        BackgroundTaskStatus::Cancelling => cx.theme().warning,
        BackgroundTaskStatus::Succeeded => cx.theme().success,
        BackgroundTaskStatus::Failed | BackgroundTaskStatus::Cancelled => cx.theme().danger,
    };
    let item_id = SharedString::from(format!("background-task-item-{}", task.id));

    v_flex()
        .id(item_id)
        .gap_1()
        .p_2()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .child(
            h_flex()
                .id("background-task-item-header")
                .items_center()
                .gap_2()
                .child(status_icon.text_color(status_color))
                .child(
                    gpui::div()
                        .id("background-task-item-title")
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(task.title.clone()),
                )
                .child(
                    gpui::div()
                        .id("background-task-item-status")
                        .text_xs()
                        .text_color(status_color)
                        .child(status_text),
                )
                .when(task.status == BackgroundTaskStatus::Cancelling, |this| {
                    this.child(
                        gpui::div()
                            .id("background-task-item-cancelling")
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("…"),
                    )
                })
                .when(task.can_cancel(), |this| {
                    this.child(
                        Button::new(SharedString::from(format!(
                            "background-task-cancel-{}",
                            task.id
                        )))
                        .label(t!("BackgroundTasks.cancel").to_string())
                        .ghost()
                        .small()
                        .danger()
                        .on_click({
                            let manager = manager.clone();
                            let id = task.id;
                            move |_, _window, cx| {
                                manager.update(cx, |manager, cx| {
                                    manager.request_cancel(id, cx);
                                });
                            }
                        }),
                    )
                }),
        )
        .when_some(task.detail.clone(), |this, detail| {
            this.child(
                gpui::div()
                    .id("background-task-item-detail")
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(detail),
            )
        })
        .when_some(task.progress.clone(), |this, progress| {
            let percent = progress.percent();
            let speed = progress.message.clone();
            this.child(
                h_flex()
                    .id("background-task-item-progress")
                    .items_center()
                    .gap_2()
                    .child(
                        Progress::new(SharedString::from(format!(
                            "background-task-progress-{}",
                            task.id
                        )))
                        .value(percent as f32)
                        .flex_1(),
                    )
                    .child(
                        gpui::div()
                            .id("background-task-item-percent")
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if progress.total.is_some() {
                                format!("{percent}%")
                            } else {
                                t!("BackgroundTasks.progress_unknown_total").to_string()
                            }),
                    )
                    .when_some(speed, |row, speed| {
                        row.child(
                            gpui::div()
                                .id("background-task-item-speed")
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(speed),
                        )
                    }),
            )
        })
        .when_some(task.result.clone(), |this, result| {
            this.child(
                gpui::div()
                    .id("background-task-item-result")
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .overflow_hidden()
                    .child(result),
            )
        })
        .when_some(task.error.clone(), |this, error| {
            this.child(
                gpui::div()
                    .id("background-task-item-error")
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .overflow_hidden()
                    .child(error),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_text_prefers_failed_count() {
        let counts = BackgroundTaskCounts {
            queued: 1,
            running: 2,
            failed: 3,
            ..Default::default()
        };
        assert_eq!(BackgroundTaskPanel::badge_text(&counts), "3 / 3");

        let counts = BackgroundTaskCounts {
            queued: 1,
            running: 2,
            ..Default::default()
        };
        assert_eq!(BackgroundTaskPanel::badge_text(&counts), "3");
    }

    #[test]
    fn running_dot_is_visible_for_active_tasks_only() {
        assert!(BackgroundTaskPanel::shows_running_dot(
            &BackgroundTaskCounts {
                running: 1,
                active: 1,
                ..Default::default()
            }
        ));
        assert!(!BackgroundTaskPanel::shows_running_dot(
            &BackgroundTaskCounts {
                failed: 1,
                ..Default::default()
            }
        ));
    }
}
