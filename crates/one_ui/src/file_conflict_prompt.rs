use std::{
    rc::Rc,
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
};

use gpui::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::{
    ActiveTheme, StyledExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex, v_flex,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileConflictChoice {
    Skip,
    KeepBoth,
    Merge,
    Overwrite,
}

/// 文件夹冲突里的「覆盖」是整目录替换，目标目录中独有的文件会一起消失；
/// 它和「合并」的区别最容易被误选，因此执行前必须在同一提示框里再确认一次。
pub fn requires_overwrite_confirmation(choice: FileConflictChoice, is_directory: bool) -> bool {
    is_directory && choice == FileConflictChoice::Overwrite
}

#[derive(Clone)]
pub struct FileConflictPromptLabels {
    pub exists: SharedString,
    pub progress: SharedString,
    pub choose_action: SharedString,
    pub apply_all: SharedString,
    pub skip: SharedString,
    pub keep_both: SharedString,
    pub merge: SharedString,
    pub overwrite: SharedString,
    /// 覆盖确认区域的标题。
    pub overwrite_warning_title: SharedString,
    /// 覆盖确认区域的正文。
    pub overwrite_warning: SharedString,
    /// 已勾选「应用到所有同类冲突」时的追加提醒。
    pub overwrite_warning_apply_all: SharedString,
    /// 「仍然覆盖」确认按钮。
    pub overwrite_confirm: SharedString,
    /// 取消确认按钮。
    pub cancel: SharedString,
}

type ChoiceHandler = Rc<dyn Fn(FileConflictChoice, bool, &mut Window, &mut App)>;

pub struct FileConflictPromptSpec {
    pub name: SharedString,
    pub is_directory: bool,
    /// 是否展示「应用于全部」复选框。只有存在多个冲突项时才有意义。
    pub show_apply_all: bool,
    pub apply_all: Arc<AtomicBool>,
    pub labels: FileConflictPromptLabels,
}

/// 冲突处理提示框。
///
/// 目录冲突选择「覆盖」时不再叠加第二个弹窗，而是在同一框内展开红色风险说明，
/// 并把按钮换成「取消 / 仍然覆盖」；再次确认后才执行。
pub struct FileConflictPrompt {
    name: SharedString,
    is_directory: bool,
    show_apply_all: bool,
    apply_all: Arc<AtomicBool>,
    labels: FileConflictPromptLabels,
    on_choice: ChoiceHandler,
    confirming_overwrite: bool,
}

impl FileConflictPrompt {
    pub fn new(
        spec: FileConflictPromptSpec,
        on_choice: impl Fn(FileConflictChoice, bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            name: spec.name,
            is_directory: spec.is_directory,
            show_apply_all: spec.show_apply_all,
            apply_all: spec.apply_all,
            labels: spec.labels,
            on_choice: Rc::new(on_choice),
            confirming_overwrite: false,
        }
    }

    /// 创建可复用的提示框实体。
    ///
    /// 确认状态要跨渲染保留，调用方需在 `window.open_dialog` 之前创建实体，
    /// 不能在对话框构建闭包里创建。
    pub fn entity(
        cx: &mut App,
        spec: FileConflictPromptSpec,
        on_choice: impl Fn(FileConflictChoice, bool, &mut Window, &mut App) + 'static,
    ) -> Entity<Self> {
        cx.new(|_| Self::new(spec, on_choice))
    }

    fn choose(
        &self,
        choice: FileConflictChoice,
        label: SharedString,
        cx: &mut Context<Self>,
    ) -> Button {
        let apply_all = self.apply_all.clone();
        let on_choice = self.on_choice.clone();
        let button = Button::new(format!("file-conflict-{choice:?}")).label(label);
        if requires_overwrite_confirmation(choice, self.is_directory) {
            let prompt = cx.entity();
            return button.on_click(move |_, _window, cx| {
                prompt.update(cx, |this, cx| {
                    this.confirming_overwrite = true;
                    cx.notify();
                });
            });
        }
        button.on_click(move |_, window, cx| {
            on_choice(choice, apply_all.load(Ordering::Relaxed), window, cx);
        })
    }
}

impl Render for FileConflictPrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = v_flex()
            .gap_3()
            .child(self.labels.exists.clone())
            .child(self.info_card(cx));

        let content = if self.confirming_overwrite {
            content.child(self.overwrite_warning(cx))
        } else {
            content.child(self.labels.choose_action.clone())
        };
        let content = content.when(self.show_apply_all, |this| {
            this.child(self.apply_all_checkbox(cx))
        });

        content.child(
            h_flex()
                .justify_end()
                .gap_2()
                .children(self.action_buttons(cx)),
        )
    }
}

impl FileConflictPrompt {
    /// 冲突项的名字与「第几个 / 共几个」进度。
    fn info_card(&self, cx: &App) -> impl IntoElement {
        div()
            .p_2()
            .rounded_md()
            .bg(cx.theme().secondary)
            .overflow_hidden()
            .child(
                div()
                    .text_sm()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(self.name.clone()),
            )
            .child(
                div()
                    .mt_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.labels.progress.clone()),
            )
    }

    fn apply_all_checkbox(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let checked = self.apply_all.load(Ordering::Relaxed);
        let apply_all = self.apply_all.clone();
        let prompt = cx.entity();
        Checkbox::new("file-conflict-apply-all")
            .checked(checked)
            .label(self.labels.apply_all.clone())
            .on_click(move |checked, _window, cx| {
                apply_all.store(*checked, Ordering::Relaxed);
                prompt.update(cx, |_this, cx| cx.notify());
            })
    }

    /// 「覆盖」是整目录替换的风险说明，用红色文字直接展示在原对话框里。
    fn overwrite_warning(&self, cx: &App) -> impl IntoElement {
        let danger = cx.theme().danger;
        div()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(danger)
            .bg(danger.opacity(0.08))
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .text_color(danger)
                    .child(self.labels.overwrite_warning_title.clone()),
            )
            .child(
                div()
                    .mt_1()
                    .text_sm()
                    .text_color(danger)
                    .child(self.labels.overwrite_warning.clone()),
            )
            .when(self.apply_all.load(Ordering::Relaxed), |el| {
                el.child(
                    div()
                        .mt_2()
                        .text_xs()
                        .text_color(danger)
                        .child(self.labels.overwrite_warning_apply_all.clone()),
                )
            })
    }

    fn action_buttons(&mut self, cx: &mut Context<Self>) -> Vec<Button> {
        if self.confirming_overwrite {
            return self.confirm_buttons(cx);
        }

        let mut actions = vec![
            self.choose(FileConflictChoice::Skip, self.labels.skip.clone(), cx)
                .ghost(),
            self.choose(
                FileConflictChoice::KeepBoth,
                self.labels.keep_both.clone(),
                cx,
            )
            .ghost(),
        ];
        if self.is_directory {
            actions.push(
                self.choose(FileConflictChoice::Merge, self.labels.merge.clone(), cx)
                    .ghost(),
            );
        }
        actions.push(
            self.choose(
                FileConflictChoice::Overwrite,
                self.labels.overwrite.clone(),
                cx,
            )
            .primary(),
        );
        actions
    }

    fn confirm_buttons(&self, cx: &mut Context<Self>) -> Vec<Button> {
        let prompt = cx.entity();
        let cancel = Button::new("file-conflict-overwrite-cancel")
            .label(self.labels.cancel.clone())
            .ghost()
            .on_click(move |_, _window, cx| {
                prompt.update(cx, |this, cx| {
                    this.confirming_overwrite = false;
                    cx.notify();
                });
            });

        let apply_all = self.apply_all.clone();
        let on_choice = self.on_choice.clone();
        let confirm = Button::new("file-conflict-overwrite-confirm")
            .label(self.labels.overwrite_confirm.clone())
            .danger()
            .on_click(move |_, window, cx| {
                on_choice(
                    FileConflictChoice::Overwrite,
                    apply_all.load(Ordering::Relaxed),
                    window,
                    cx,
                );
            });

        vec![cancel, confirm]
    }
}
