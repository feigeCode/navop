use crate::notes_actions::CreateKind;
use crate::notes_view::NotesLoadState;
use crate::{DocumentFormat, NodeKind, NotesView, TreeRow};
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    scroll::ScrollableElement,
    v_flex,
};

impl NotesView {
    fn render_setup(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(px(440.0))
                    .gap_4()
                    .p_6()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .child(div().text_xl().font_semibold().child("创建本地笔记本"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("笔记仅保存在此设备，不会同步到云端。"),
                    )
                    .child(Input::new(&self.setup_name).w_full())
                    .child(Input::new(&self.setup_description).w_full())
                    .child(
                        h_flex().justify_end().child(
                            Button::new("create_notebook")
                                .label("创建")
                                .primary()
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.create_notebook(window, cx)
                                })),
                        ),
                    )
                    .when_some(self.error.clone(), |this, error| {
                        this.child(div().text_sm().text_color(cx.theme().danger).child(error))
                    }),
            )
            .into_any_element()
    }

    fn render_ready(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        v_flex()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .when_some(self.error.clone(), |this, error| {
                this.child(self.render_error_banner(error, cx))
            })
            .child(
                h_flex()
                    .flex_1()
                    .h_full()
                    .min_h_0()
                    .items_start()
                    .child(self.render_sidebar(cx))
                    .child(self.render_editor(cx)),
            )
            .into_any_element()
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let rows = self
            .rows
            .iter()
            .map(|row| self.render_row(row, cx))
            .collect::<Vec<_>>();
        v_flex()
            .w(px(220.0))
            .h_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .size_full()
                            .p_2()
                            .gap_1()
                            .overflow_y_scrollbar()
                            .children(rows),
                    ),
            )
            .into_any_element()
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .when(
                self.active_document_id
                    .as_ref()
                    .is_some_and(|id| self.markdown_sessions.contains_key(id)),
                |this| this.child(self.render_markdown_editor(cx)),
            )
            .when_some(self.active_editor.as_ref(), |this, handle| {
                this.child(handle.entity().clone())
            })
            .when(
                self.active_editor.is_none() && self.active_document_id.is_none(),
                |this| {
                    this.flex().items_center().justify_center().child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child("选择或新建一个文档"),
                    )
                },
            )
            .into_any_element()
    }

    fn render_error_banner(&self, error: SharedString, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .p_2()
            .bg(cx.theme().danger.opacity(0.08))
            .text_color(cx.theme().danger)
            .child(error)
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .p_2()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("new_note_document")
                    .icon(IconName::File)
                    .label("富文本")
                    .small()
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.start_create(
                            CreateKind::Document(DocumentFormat::RichText),
                            window,
                            cx,
                        )
                    })),
            )
            .child(
                Button::new("new_note_markdown")
                    .icon(IconName::File)
                    .label("Markdown")
                    .small()
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.start_create(
                            CreateKind::Document(DocumentFormat::Markdown),
                            window,
                            cx,
                        )
                    })),
            )
            .child(
                Button::new("new_note_directory")
                    .icon(IconName::NewFolder)
                    .label("目录")
                    .small()
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.start_create(CreateKind::Directory, window, cx)
                    })),
            )
    }

    fn render_row(&self, row: &TreeRow, cx: &mut Context<Self>) -> gpui::AnyElement {
        let select_path = row.relative_path.clone();
        let kind = row.kind;
        let selected = self.tree.selected_document.as_ref() == Some(&row.relative_path)
            || (kind == NodeKind::Directory && self.current_directory == row.relative_path);
        let icon = match (kind, row.expanded) {
            (NodeKind::Directory, true) => IconName::FolderOpen,
            (NodeKind::Directory, false) => IconName::Folder,
            (NodeKind::Document, _) => IconName::File,
        };
        h_flex()
            .id(SharedString::from(format!(
                "notes-row-{}",
                row.relative_path.display()
            )))
            .h_8()
            .w_full()
            .pl_2()
            .gap_2()
            .rounded_md()
            .cursor_pointer()
            .when(selected, |this| this.bg(cx.theme().primary.opacity(0.12)))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.select_row(select_path.clone(), kind, window, cx)
            }))
            .child(div().w(px((row.depth * 14) as f32)))
            .child(Icon::new(icon).small())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .child(row.display_name.clone()),
            )
            .when_some(row.format, |this, format| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(match format {
                            DocumentFormat::RichText => "富",
                            DocumentFormat::Markdown => "MD",
                        }),
                )
            })
            .child(self.render_rename_button(row, cx))
            .child(self.render_delete_button(row, cx))
            .into_any_element()
    }

    fn render_rename_button(&self, row: &TreeRow, cx: &mut Context<Self>) -> Button {
        let rename_row = row.clone();
        Button::new(row_action_id("rename", row))
            .icon(IconName::Edit)
            .ghost()
            .xsmall()
            .on_click(cx.listener(move |view, _, window, cx| {
                view.start_rename(rename_row.clone(), window, cx)
            }))
    }

    fn render_delete_button(&self, row: &TreeRow, cx: &mut Context<Self>) -> Button {
        let delete_row = row.clone();
        Button::new(row_action_id("delete", row))
            .icon(IconName::Delete)
            .ghost()
            .xsmall()
            .on_click(cx.listener(move |view, _, window, cx| {
                view.confirm_delete(delete_row.clone(), window, cx)
            }))
    }
}

fn row_action_id(action: &str, row: &TreeRow) -> SharedString {
    format!("{action}-note-{}", row.relative_path.display()).into()
}

impl Render for NotesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.load_state {
            NotesLoadState::NeedsSetup => self.render_setup(cx),
            NotesLoadState::Ready => self.render_ready(cx),
            NotesLoadState::Failed(error) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(Icon::new(IconName::TriangleAlert))
                .child(div().text_color(cx.theme().danger).child(error.clone()))
                .into_any_element(),
        };
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(content)
    }
}
