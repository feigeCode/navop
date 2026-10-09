use super::{DocumentPolicy, WORKSPACE_EDITOR_KEY_CONTEXT, WorkspaceEditor, format_size};
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div,
};
use gpui_component::{
    Disableable as _, Selectable as _, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    diff::Diff,
    h_flex,
    input::Editor,
    tab::{Tab, TabBar},
    v_flex,
};
use one_assets::IconName;
use one_ui::{ContentState, IconButton, IconSize, StatusBar, StatusPresentation};
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
                    IconButton::new(
                        SharedString::from(format!("workspace-close-tab-{index}")),
                        IconName::Close,
                    )
                    .hit_size(Size::Small)
                    .glyph_size(IconSize::Small)
                    .tooltip(t!("WorkspaceExplorer.tooltip.close_tab"))
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

    fn render_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let tab = self.active_tab();
        if tab.is_some_and(|tab| matches!(tab.policy, DocumentPolicy::Markdown)) {
            return div().into_any_element();
        }
        let read_only = tab.is_none_or(|tab| tab.read_only);
        // 编辑器相关动作(保存/查找/替换)要求有 `EditorState`——diff 视图没有，
        // 由 Diff 组件负责渲染。
        let unavailable = tab.is_none_or(|tab| tab.loading || tab.saving || tab.editor.is_none());
        // 重新加载只重发一次 load_request,diff 视图同样适用。
        let reload_unavailable = tab.is_none_or(|tab| tab.loading || tab.saving);
        let soft_wrap = tab.is_some_and(|tab| tab.soft_wrap);
        // Diff 组件原生支持多文件 Split,单文件与整轮快照 diff 都能并排。
        let side_by_side_available = tab.is_some_and(|tab| tab.diff_state.is_some());
        let side_by_side = side_by_side_available && tab.is_some_and(|tab| tab.diff_side_by_side);
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
            .child(self.toolbar_button(EditorAction::Reload, reload_unavailable, cx))
            .child(
                Button::new("workspace-diff-view")
                    .label(t!("WorkspaceExplorer.action.side_by_side"))
                    .selected(side_by_side)
                    .with_size(Size::Small)
                    .custom(self.theme.button_style(cx))
                    .disabled(!side_by_side_available)
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.toggle_diff_view(cx);
                    })),
            )
            .child(
                IconButton::new("workspace-diff-previous", IconName::ArrowUp)
                    .tooltip(t!("WorkspaceExplorer.action.previous_change"))
                    .disabled(!side_by_side)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.previous_diff_change(cx);
                    })),
            )
            .child(
                IconButton::new("workspace-diff-next", IconName::ArrowDown)
                    .tooltip(t!("WorkspaceExplorer.action.next_change"))
                    .disabled(!side_by_side)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.next_diff_change(cx);
                    })),
            )
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
            .into_any_element()
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
            return ContentState::empty(t!("WorkspaceExplorer.body.empty").to_string())
                .into_any_element();
        };
        if tab.loading {
            return ContentState::loading(t!("WorkspaceExplorer.body.loading").to_string())
                .into_any_element();
        }
        if let Some(error) = tab.load_error.as_ref() {
            return self.render_load_error(error);
        }
        if let Some(markdown) = tab.markdown.as_ref() {
            return div()
                .size_full()
                .min_h_0()
                .min_w_0()
                .child(markdown.clone())
                .into_any_element();
        }
        if matches!(tab.policy, DocumentPolicy::Diff) && tab.saved_text.trim().is_empty() {
            return ContentState::empty(t!("WorkspaceExplorer.diff.empty").to_string())
                .into_any_element();
        }
        if let Some(diff_state) = tab.diff_state.as_ref() {
            // Diff 组件自己滚动、自己虚拟化,给它一个有界的 flex 区域即可。
            // 文件头(路径 + 增删统计)由组件绘制。
            return v_flex()
                .size_full()
                .min_h_0()
                .min_w_0()
                .child(Diff::new(diff_state).size_full())
                .into_any_element();
        }
        match tab.editor.as_ref() {
            Some(editor) => v_flex()
                .size_full()
                .min_h_0()
                .child(
                    Editor::new(editor)
                        .size_full()
                        .readonly(tab.read_only)
                        .editor_style(self.theme.editor_style())
                        .bg(self.theme.background)
                        .text_color(self.theme.foreground)
                        .border_color(self.theme.border),
                )
                .into_any_element(),
            None => v_flex().size_full().into_any_element(),
        }
    }

    fn render_load_error(&self, error: &str) -> gpui::AnyElement {
        ContentState::error(t!("WorkspaceExplorer.body.unable_to_open").to_string())
            .detail(error.to_string())
            .into_any_element()
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.active_tab();
        let dirty = tab.is_some_and(|tab| tab.is_dirty(cx));
        let presentation = tab.map_or(StatusPresentation::Neutral, |tab| {
            if matches!(
                tab.status_presentation,
                StatusPresentation::Progress | StatusPresentation::Error
            ) {
                tab.status_presentation
            } else if dirty {
                StatusPresentation::Warning
            } else {
                tab.status_presentation
            }
        });
        let status_message = tab
            .map(|tab| {
                if dirty
                    && !matches!(
                        tab.status_presentation,
                        StatusPresentation::Progress | StatusPresentation::Error
                    )
                {
                    t!("WorkspaceExplorer.status.unsaved").to_string()
                } else {
                    tab.status_message.clone()
                }
            })
            .unwrap_or_default();

        StatusBar::new("workspace-editor-status")
            .presentation(presentation)
            .colors(self.theme.status_bar_colors())
            .leading(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(self.theme.muted_foreground)
                    .child(tab.map(|tab| tab.key.display_path()).unwrap_or_default()),
            )
            .trailing(
                h_flex().gap_2().child(
                    div()
                        .text_sm()
                        .text_color(self.theme.muted_foreground)
                        .child(format_size(tab.map_or(0, |tab| tab.file_size))),
                ),
            )
            .status(div().text_sm().child(status_message))
    }
}

impl Render for WorkspaceEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .key_context(WORKSPACE_EDITOR_KEY_CONTEXT)
            .on_action(cx.listener(Self::save_from_keyboard))
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
        DocumentPolicy::Markdown => t!("WorkspaceExplorer.policy.markdown").to_string(),
        DocumentPolicy::Diff => t!("WorkspaceExplorer.policy.diff").to_string(),
    }
}
