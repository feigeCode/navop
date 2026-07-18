use crate::notes_actions::CreateKind;
use crate::notes_view::NotesLoadState;
use crate::theme_provider::cditor_theme;
use crate::{DocumentFormat, NodeKind, NotesView, TreeRow};
use gpui::{
    Context, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    menu::{ContextMenuExt, PopupMenu, PopupMenuItem},
    scroll::ScrollableElement,
    v_flex,
};
use std::path::{Path, PathBuf};

const NOTES_SIDEBAR_EXPANDED_WIDTH: gpui::Pixels = px(248.0);
const NOTES_SIDEBAR_COLLAPSED_WIDTH: gpui::Pixels = px(28.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarContextTarget {
    Background,
    Directory,
    RichTextDocument,
    MarkdownDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarMenuAction {
    NewRichText,
    NewMarkdown,
    NewFolder,
    RevealInFileManager,
    Refresh,
    ConvertToMarkdown,
    Rename,
    Delete,
}

fn sidebar_menu_actions(target: SidebarContextTarget) -> Vec<SidebarMenuAction> {
    match target {
        SidebarContextTarget::Background => vec![
            SidebarMenuAction::NewRichText,
            SidebarMenuAction::NewMarkdown,
            SidebarMenuAction::NewFolder,
            SidebarMenuAction::RevealInFileManager,
            SidebarMenuAction::Refresh,
        ],
        SidebarContextTarget::Directory => vec![
            SidebarMenuAction::NewRichText,
            SidebarMenuAction::NewMarkdown,
            SidebarMenuAction::NewFolder,
            SidebarMenuAction::RevealInFileManager,
            SidebarMenuAction::Rename,
            SidebarMenuAction::Delete,
        ],
        SidebarContextTarget::RichTextDocument => vec![
            SidebarMenuAction::NewRichText,
            SidebarMenuAction::NewMarkdown,
            SidebarMenuAction::NewFolder,
            SidebarMenuAction::RevealInFileManager,
            SidebarMenuAction::ConvertToMarkdown,
            SidebarMenuAction::Rename,
            SidebarMenuAction::Delete,
        ],
        SidebarContextTarget::MarkdownDocument => vec![
            SidebarMenuAction::NewRichText,
            SidebarMenuAction::NewMarkdown,
            SidebarMenuAction::NewFolder,
            SidebarMenuAction::RevealInFileManager,
            SidebarMenuAction::Rename,
            SidebarMenuAction::Delete,
        ],
    }
}

impl NotesView {
    fn render_ready(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.standalone_markdown {
            return self.render_editor(cx);
        }
        v_flex()
            .size_full()
            .min_h_0()
            .overflow_hidden()
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
        let collapsed = self.sidebar_collapsed;
        let view = cx.entity();
        let rows = self
            .rows
            .iter()
            .map(|row| self.render_row(row, cx))
            .collect::<Vec<_>>();
        let rows_are_empty = rows.is_empty();
        v_flex()
            .relative()
            .w(if collapsed {
                NOTES_SIDEBAR_COLLAPSED_WIDTH
            } else {
                NOTES_SIDEBAR_EXPANDED_WIDTH
            })
            .flex_shrink_0()
            .h_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .border_r_1()
            .border_color(cx.theme().border)
            .when(!collapsed, |this| this.child(self.render_toolbar(cx)))
            .when(!collapsed, |this| {
                this.child(
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
                                .context_menu(move |menu, window, cx| {
                                    let (target, directory, row) = {
                                        let notes = view.read(cx);
                                        let row =
                                            notes.context_menu_path.as_ref().and_then(|path| {
                                                notes
                                                    .rows
                                                    .iter()
                                                    .find(|row| row.relative_path == *path)
                                                    .cloned()
                                            });
                                        match row {
                                            Some(row) => (
                                                sidebar_context_target(&row),
                                                creation_directory(&row),
                                                Some(row),
                                            ),
                                            None => (
                                                SidebarContextTarget::Background,
                                                notes.current_directory.clone(),
                                                None,
                                            ),
                                        }
                                    };
                                    view.update(cx, |notes, _| {
                                        notes.context_menu_path = None;
                                    });
                                    build_sidebar_context_menu(
                                        menu,
                                        view.clone(),
                                        target,
                                        directory,
                                        row,
                                        window,
                                        cx,
                                    )
                                })
                                .children(rows)
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_h(px(64.0))
                                        .items_center()
                                        .justify_center()
                                        .on_mouse_down(
                                            MouseButton::Right,
                                            cx.listener(|view, _, _, cx| {
                                                view.context_menu_path = None;
                                                cx.notify();
                                            }),
                                        )
                                        .when(rows_are_empty, |this| {
                                            this.gap_1()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child("还没有笔记")
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .child("使用顶部按钮或在这里右键新建"),
                                                )
                                        }),
                                ),
                        ),
                )
            })
            .child(
                div()
                    .absolute()
                    .right_0()
                    .top_0()
                    .bottom_0()
                    .w(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .occlude()
                    .cursor_pointer()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|view, _, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            view.sidebar_collapsed = !view.sidebar_collapsed;
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .id("notes-sidebar-toggle")
                            .w(px(18.0))
                            .h(px(52.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(9.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .shadow_sm()
                            .hover(|this| this.bg(cx.theme().muted))
                            .child(
                                Icon::new(if collapsed {
                                    IconName::ChevronRight
                                } else {
                                    IconName::ChevronLeft
                                })
                                .with_size(Size::Small)
                                .text_color(cx.theme().muted_foreground),
                            ),
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

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .p_2()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_sm()
                    .font_semibold()
                    .child(self.notebook_name.clone()),
            )
            .child(
                Button::new("new_note_document")
                    .icon(IconName::RichTextColor.color())
                    .ghost()
                    .xsmall()
                    .tooltip("新建富文本文档")
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
                    .icon(IconName::MarkdownColor.color())
                    .ghost()
                    .xsmall()
                    .tooltip("新建 Markdown 文档")
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
                    .ghost()
                    .xsmall()
                    .tooltip("新建文件夹")
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.start_create(CreateKind::Directory, window, cx)
                    })),
            )
            .child(
                Button::new("refresh_notes_sidebar")
                    .icon(IconName::Refresh)
                    .ghost()
                    .xsmall()
                    .tooltip("刷新笔记列表")
                    .on_click(cx.listener(|view, _, window, cx| {
                        if let Err(error) = view.refresh_tree(window, cx) {
                            crate::notes_notifications::notify_operation_error(window, cx, error);
                        }
                        cx.notify();
                    })),
            )
    }

    fn render_row(&self, row: &TreeRow, cx: &mut Context<Self>) -> gpui::AnyElement {
        let select_path = row.relative_path.clone();
        let kind = row.kind;
        let selected = self.selected_sidebar_path.as_ref() == Some(&row.relative_path);
        let icon = match (kind, row.expanded, row.format) {
            (NodeKind::Directory, true, _) => Icon::new(IconName::FolderOpen),
            (NodeKind::Directory, false, _) => Icon::new(IconName::Folder),
            (NodeKind::Document, _, Some(DocumentFormat::RichText)) => {
                IconName::RichTextColor.color()
            }
            (NodeKind::Document, _, Some(DocumentFormat::Markdown)) => {
                IconName::MarkdownColor.color()
            }
            (NodeKind::Document, _, None) => Icon::new(IconName::File),
        };
        let disclosure = if kind == NodeKind::Directory {
            Icon::new(if row.expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .with_size(Size::XSmall)
            .text_color(cx.theme().muted_foreground)
            .into_any_element()
        } else {
            div().w(px(16.0)).into_any_element()
        };
        let indent = px(8.0 + row.depth as f32 * 14.0);
        h_flex()
            .id(SharedString::from(format!(
                "notes-row-{}",
                row.relative_path.display()
            )))
            .h(px(28.0))
            .w_full()
            .relative()
            .pl(indent)
            .pr_2()
            .gap_1()
            .cursor_pointer()
            .text_sm()
            .text_color(cx.theme().sidebar_foreground)
            .when(selected, |this| {
                this.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(3.0))
                        .bg(cx.theme().blue),
                )
                .bg(cx.theme().sidebar_accent)
                .text_color(cx.theme().sidebar_accent_foreground)
            })
            .when(!selected, |this| {
                this.hover(|this| this.bg(cx.theme().secondary))
            })
            .on_click(cx.listener(move |view, _, window, cx| {
                view.select_row(select_path.clone(), kind, window, cx)
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener({
                    let menu_path = row.relative_path.clone();
                    move |view, _, _, cx| {
                        view.selected_sidebar_path = Some(menu_path.clone());
                        view.context_menu_path = Some(menu_path.clone());
                        cx.notify();
                    }
                }),
            )
            .child(disclosure)
            .child(icon.small())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_sm()
                    .child(row.display_name.clone()),
            )
            .into_any_element()
    }
}

fn sidebar_context_target(row: &TreeRow) -> SidebarContextTarget {
    match (row.kind, row.format) {
        (NodeKind::Directory, _) => SidebarContextTarget::Directory,
        (NodeKind::Document, Some(DocumentFormat::RichText)) => {
            SidebarContextTarget::RichTextDocument
        }
        (NodeKind::Document, _) => SidebarContextTarget::MarkdownDocument,
    }
}

fn creation_directory(row: &TreeRow) -> PathBuf {
    if row.kind == NodeKind::Directory {
        row.relative_path.clone()
    } else {
        row.relative_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf()
    }
}

fn build_sidebar_context_menu(
    mut menu: PopupMenu,
    view: Entity<NotesView>,
    target: SidebarContextTarget,
    directory: PathBuf,
    row: Option<TreeRow>,
    window: &mut Window,
    _cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let actions = sidebar_menu_actions(target);
    for (index, action) in actions.iter().copied().enumerate() {
        if index > 0
            && matches!(
                action,
                SidebarMenuAction::RevealInFileManager
                    | SidebarMenuAction::Refresh
                    | SidebarMenuAction::ConvertToMarkdown
                    | SidebarMenuAction::Rename
            )
        {
            menu = menu.separator();
        }
        menu = match action {
            SidebarMenuAction::NewRichText => {
                let directory = directory.clone();
                menu.item(
                    PopupMenuItem::new("新建富文本文档")
                        .icon(IconName::RichTextColor.color())
                        .on_click(window.listener_for(&view, move |view, _, window, cx| {
                            view.start_create_in(
                                directory.clone(),
                                CreateKind::Document(DocumentFormat::RichText),
                                window,
                                cx,
                            );
                        })),
                )
            }
            SidebarMenuAction::NewMarkdown => {
                let directory = directory.clone();
                menu.item(
                    PopupMenuItem::new("新建 Markdown 文档")
                        .icon(IconName::MarkdownColor.color())
                        .on_click(window.listener_for(&view, move |view, _, window, cx| {
                            view.start_create_in(
                                directory.clone(),
                                CreateKind::Document(DocumentFormat::Markdown),
                                window,
                                cx,
                            );
                        })),
                )
            }
            SidebarMenuAction::NewFolder => {
                let directory = directory.clone();
                menu.item(
                    PopupMenuItem::new("新建文件夹")
                        .icon(IconName::NewFolder)
                        .on_click(window.listener_for(&view, move |view, _, window, cx| {
                            view.start_create_in(
                                directory.clone(),
                                CreateKind::Directory,
                                window,
                                cx,
                            );
                        })),
                )
            }
            SidebarMenuAction::RevealInFileManager => {
                let relative_path = row
                    .as_ref()
                    .map(|row| row.relative_path.clone())
                    .unwrap_or_else(|| directory.clone());
                menu.item(
                    PopupMenuItem::new(crate::file_manager::menu_label())
                        .icon(IconName::FolderOpen)
                        .on_click(window.listener_for(&view, move |view, _, window, cx| {
                            view.reveal_in_file_manager(relative_path.clone(), window, cx);
                        })),
                )
            }
            SidebarMenuAction::Refresh => {
                menu.item(PopupMenuItem::new("刷新").icon(IconName::Refresh).on_click(
                    window.listener_for(&view, |view, _, window, cx| {
                        if let Err(error) = view.refresh_tree(window, cx) {
                            crate::notes_notifications::notify_operation_error(window, cx, error);
                        }
                        cx.notify();
                    }),
                ))
            }
            SidebarMenuAction::ConvertToMarkdown => {
                let row = row.clone().expect("document context menu requires a row");
                menu.item(
                    PopupMenuItem::new("转换为 Markdown（保留原文档）")
                        .icon(IconName::Copy)
                        .on_click(window.listener_for(&view, move |view, _, window, cx| {
                            view.convert_to_markdown(&row, window, cx);
                        })),
                )
            }
            SidebarMenuAction::Rename => {
                let row = row.clone().expect("node context menu requires a row");
                menu.item(PopupMenuItem::new("重命名").icon(IconName::Edit).on_click(
                    window.listener_for(&view, move |view, _, window, cx| {
                        view.start_rename(row.clone(), window, cx);
                    }),
                ))
            }
            SidebarMenuAction::Delete => {
                let row = row.clone().expect("node context menu requires a row");
                menu.item(PopupMenuItem::new("删除").icon(IconName::Remove).on_click(
                    window.listener_for(&view, move |view, _, window, cx| {
                        view.confirm_delete(row.clone(), window, cx);
                    }),
                ))
            }
        };
    }
    menu
}

impl Render for NotesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme_changed = self.theme_provider.refresh(cditor_theme(
            cx.theme().background,
            cx.theme().foreground,
            cx.theme().muted_foreground,
            cx.theme().border,
            cx.theme().primary,
            cx.theme().danger,
        ));
        if theme_changed {
            let mut editors = self
                .editors
                .values()
                .map(|editor| editor.handle.clone())
                .collect::<Vec<_>>();
            editors.extend(
                self.markdown_sessions
                    .values()
                    .map(|session| session.preview.clone()),
            );
            for editor in editors {
                editor.entity().update(cx, |_view, cx| cx.notify());
            }
        }
        self.syntax_highlight_provider.refresh_theme(
            cx.theme().highlight_theme.clone(),
            cx.theme().background,
            cx.theme().foreground,
        );
        let content = match &self.load_state {
            NotesLoadState::NeedsLocation => self.render_location_setup(cx),
            NotesLoadState::Ready => self.render_ready(cx),
        };
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::{SidebarContextTarget, SidebarMenuAction, sidebar_menu_actions};

    #[test]
    fn background_menu_exposes_create_file_manager_and_refresh_actions() {
        assert_eq!(
            sidebar_menu_actions(SidebarContextTarget::Background),
            vec![
                SidebarMenuAction::NewRichText,
                SidebarMenuAction::NewMarkdown,
                SidebarMenuAction::NewFolder,
                SidebarMenuAction::RevealInFileManager,
                SidebarMenuAction::Refresh,
            ]
        );
    }

    #[test]
    fn directory_menu_adds_node_management_after_create_actions() {
        assert_eq!(
            sidebar_menu_actions(SidebarContextTarget::Directory),
            vec![
                SidebarMenuAction::NewRichText,
                SidebarMenuAction::NewMarkdown,
                SidebarMenuAction::NewFolder,
                SidebarMenuAction::RevealInFileManager,
                SidebarMenuAction::Rename,
                SidebarMenuAction::Delete,
            ]
        );
    }

    #[test]
    fn rich_text_document_menu_is_the_only_menu_with_conversion() {
        assert!(
            sidebar_menu_actions(SidebarContextTarget::RichTextDocument)
                .contains(&SidebarMenuAction::ConvertToMarkdown)
        );
        assert!(
            !sidebar_menu_actions(SidebarContextTarget::MarkdownDocument)
                .contains(&SidebarMenuAction::ConvertToMarkdown)
        );
    }
}
