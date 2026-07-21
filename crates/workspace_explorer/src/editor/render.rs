use super::{DiffEditors, DocumentPolicy, WorkspaceEditor, format_size};
use gpui::{
    AnyElement, Context, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Disableable as _, IconName, Selectable as _, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, LocalInputStyle},
    tab::{Tab, TabBar},
    v_flex,
};
use rust_i18n::t;

#[derive(Clone, Copy)]
enum EditorAction {
    Save,
    Search,
    Replace,
    Reload,
}

impl WorkspaceEditor {
    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut tabs = TabBar::new("workspace-editor-tabs")
            .menu(true)
            .with_size(Size::Small)
            .local_style(self.theme.tab_style())
            .selected_index(self.active_tab)
            .on_click({
                let entity = cx.entity().downgrade();
                move |index, window, cx| {
                    let _ = entity.update(cx, |this, cx| this.switch_tab(*index, window, cx));
                }
            });
        for (index, tab) in self.tabs.iter().enumerate() {
            let label = if tab.is_dirty(cx) {
                format!("● {}", tab.display_name)
            } else {
                tab.display_name.clone()
            };
            tabs = tabs.child(
                Tab::new().label(label).suffix(
                    Button::new(SharedString::from(format!("workspace-close-tab-{index}")))
                        .label("×")
                        .with_size(Size::XSmall)
                        .custom(self.theme.button_style(cx))
                        .disabled(tab.saving)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.request_close_tab(index, window, cx);
                        })),
                ),
            );
        }
        h_flex()
            .border_b_1()
            .border_color(self.theme.border)
            .bg(self.theme.muted)
            .child(tabs)
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.active_tab();
        let read_only = tab.is_none_or(|tab| tab.read_only);
        let unavailable = tab.is_none_or(|tab| tab.loading || tab.saving || tab.editor.is_none());
        let soft_wrap = tab.is_some_and(|tab| tab.soft_wrap);
        let diff_available = tab.is_some_and(|tab| tab.diff.is_some());
        let side_by_side = diff_available && tab.is_some_and(|tab| tab.diff_side_by_side);
        let diff_change_count = tab
            .and_then(|tab| tab.diff.as_ref())
            .map_or(0, |diff| crate::diff::change_starts(diff).len());
        let has_diff_changes = diff_change_count > 0;
        h_flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(self.theme.border)
            .bg(self.theme.muted)
            .child(self.toolbar_button(EditorAction::Save, unavailable || read_only, cx))
            .child(self.toolbar_button(EditorAction::Search, unavailable, cx))
            .child(self.toolbar_button(EditorAction::Replace, unavailable || read_only, cx))
            .child(self.toolbar_button(EditorAction::Reload, unavailable, cx))
            .child(
                Button::new("workspace-diff-view")
                    .label(t!("WorkspaceExplorer.action.side_by_side"))
                    .selected(side_by_side)
                    .with_size(Size::Small)
                    .custom(self.theme.button_style(cx))
                    .disabled(!diff_available)
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.toggle_diff_view(cx);
                    })),
            )
            .child(
                Button::new("workspace-diff-previous")
                    .icon(IconName::ArrowUp)
                    .ghost()
                    .compact()
                    .tooltip(t!("WorkspaceExplorer.action.previous_change"))
                    .disabled(unavailable || !side_by_side || !has_diff_changes)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.previous_diff_change(cx);
                    })),
            )
            .child(
                Button::new("workspace-diff-next")
                    .icon(IconName::ArrowDown)
                    .ghost()
                    .compact()
                    .tooltip(t!("WorkspaceExplorer.action.next_change"))
                    .disabled(unavailable || !side_by_side || !has_diff_changes)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.next_diff_change(cx);
                    })),
            )
            .when(side_by_side, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(self.theme.muted_foreground)
                        .child(
                            t!(
                                "WorkspaceExplorer.diff.change_count",
                                count = diff_change_count
                            )
                            .to_string(),
                        ),
                )
            })
            .child(
                Button::new("workspace-wrap")
                    .label(t!("WorkspaceExplorer.action.soft_wrap"))
                    .selected(soft_wrap)
                    .with_size(Size::Small)
                    .custom(self.theme.button_style(cx))
                    .disabled(unavailable || side_by_side)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_soft_wrap(window, cx);
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_sm()
                    .text_color(self.theme.muted_foreground)
                    .child(tab.map(|tab| policy_label(tab.policy)).unwrap_or_default()),
            )
    }

    fn toolbar_button(
        &self,
        action: EditorAction,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let (id, label) = match action {
            EditorAction::Save => ("workspace-save", t!("WorkspaceExplorer.action.save")),
            EditorAction::Search => ("workspace-search", t!("WorkspaceExplorer.action.search")),
            EditorAction::Replace => ("workspace-replace", t!("WorkspaceExplorer.action.replace")),
            EditorAction::Reload => ("workspace-reload", t!("WorkspaceExplorer.action.reload")),
        };
        let button = Button::new(id)
            .label(label)
            .with_size(Size::Small)
            .custom(self.theme.button_style(cx))
            .disabled(disabled);
        match action {
            EditorAction::Save => button.on_click(cx.listener(|this, _, window, cx| {
                this.save(false, window, cx);
            })),
            EditorAction::Search => button.on_click(cx.listener(|this, _, window, cx| {
                this.trigger_search(window, cx);
            })),
            EditorAction::Replace => button.on_click(cx.listener(|this, _, window, cx| {
                this.trigger_replace(window, cx);
            })),
            EditorAction::Reload => button.on_click(cx.listener(|this, _, window, cx| {
                this.reload(window, cx);
            })),
        }
    }

    fn render_body(&self) -> AnyElement {
        let Some(tab) = self.active_tab() else {
            return v_flex().size_full().into_any_element();
        };
        if tab.loading {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(self.theme.muted_foreground)
                .child(t!("WorkspaceExplorer.body.loading"))
                .into_any_element();
        }
        if let Some(error) = tab.load_error.as_ref() {
            return self.render_load_error(error);
        }
        if matches!(tab.policy, DocumentPolicy::Diff) && tab.saved_text.trim().is_empty() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(self.theme.muted_foreground)
                .child(t!("WorkspaceExplorer.diff.empty"))
                .into_any_element();
        }
        if tab.diff_side_by_side {
            if let Some(editors) = tab.diff_editors.as_ref() {
                return self.render_diff_view(editors);
            }
        }
        match tab.editor.as_ref() {
            Some(editor) => v_flex()
                .size_full()
                .min_h_0()
                .child(
                    Input::new(editor)
                        .size_full()
                        .read_only(tab.read_only)
                        .highlight_theme(self.theme.highlight_theme())
                        .local_style(LocalInputStyle {
                            background: self.theme.background,
                            foreground: self.theme.foreground,
                            muted_foreground: self.theme.muted_foreground,
                            border: self.theme.border,
                        }),
                )
                .into_any_element(),
            None => v_flex().size_full().into_any_element(),
        }
    }

    fn render_diff_view(&self, editors: &DiffEditors) -> AnyElement {
        let theme = self.theme;
        let input_style = LocalInputStyle {
            background: theme.background,
            foreground: theme.foreground,
            muted_foreground: theme.muted_foreground,
            border: theme.border,
        };

        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .w_full()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .border_b_1()
                    .border_color(theme.border)
                    .bg(theme.muted)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_2()
                            .py_1()
                            .truncate()
                            .child(t!("WorkspaceExplorer.diff.before")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_2()
                            .py_1()
                            .truncate()
                            .border_l_1()
                            .border_color(theme.border)
                            .child(t!("WorkspaceExplorer.diff.after")),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div().flex_1().min_w_0().h_full().overflow_hidden().child(
                            Input::new(&editors.left)
                                .size_full()
                                .read_only(true)
                                .bordered(false)
                                .focus_bordered(false)
                                .highlight_theme(theme.highlight_theme())
                                .indent_guide_color(theme.border.opacity(0.28))
                                .local_style(input_style),
                        ),
                    )
                    .child(div().w(px(1.0)).h_full().flex_none().bg(theme.border))
                    .child(
                        div().flex_1().min_w_0().h_full().overflow_hidden().child(
                            Input::new(&editors.right)
                                .size_full()
                                .read_only(true)
                                .bordered(false)
                                .focus_bordered(false)
                                .highlight_theme(theme.highlight_theme())
                                .indent_guide_color(theme.border.opacity(0.28))
                                .local_style(input_style),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn render_load_error(&self, error: &str) -> gpui::AnyElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                div()
                    .text_base()
                    .child(t!("WorkspaceExplorer.body.unable_to_open")),
            )
            .child(
                div()
                    .max_w(px(640.0))
                    .text_sm()
                    .text_color(self.theme.danger)
                    .child(error.to_string()),
            )
            .into_any_element()
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.active_tab();
        let dirty = tab.is_some_and(|tab| tab.is_dirty(cx));
        h_flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .border_t_1()
            .border_color(self.theme.border)
            .bg(self.theme.background)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(self.theme.muted_foreground)
                    .child(tab.map(|tab| tab.key.display_path()).unwrap_or_default()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(self.theme.muted_foreground)
                    .child(format_size(tab.map_or(0, |tab| tab.file_size))),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(if dirty {
                        self.theme.warning
                    } else {
                        self.theme.muted_foreground
                    })
                    .child(
                        tab.map(|tab| tab.status_message.clone())
                            .unwrap_or_default(),
                    ),
            )
    }
}

impl Render for WorkspaceEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(self.theme.background)
            .text_color(self.theme.foreground)
            .child(self.render_tabs(cx))
            .child(self.render_toolbar(cx))
            .child(v_flex().flex_1().min_h_0().child(self.render_body()))
            .child(self.render_status_bar(cx))
    }
}

fn policy_label(policy: DocumentPolicy) -> String {
    match policy {
        DocumentPolicy::Code => t!("WorkspaceExplorer.policy.code").to_string(),
        DocumentPolicy::PlainText => t!("WorkspaceExplorer.policy.plain_text").to_string(),
        DocumentPolicy::Diff => t!("WorkspaceExplorer.policy.diff").to_string(),
    }
}
