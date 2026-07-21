use super::WorkspaceExplorer;
use crate::git::{GitChange, GitChangeKind};
use crate::model::{ExplorerRow, visible_rows};
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size, StyledExt as _, h_flex, v_flex};
use rust_i18n::t;

use super::header::ExplorerSection;

impl WorkspaceExplorer {
    fn render_changes(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.repository.is_none() {
            return div().into_any_element();
        }
        let mut section = v_flex()
            .w_full()
            .child(self.render_section_header(ExplorerSection::Changes, cx));
        if !self.changes_expanded {
            return section.into_any_element();
        }
        if self.git_loading {
            return section
                .child(self.section_message(
                    t!("WorkspaceExplorer.state.refreshing_changes").to_string(),
                    self.theme.muted_foreground,
                ))
                .into_any_element();
        }
        if let Some(error) = self.git_error.as_ref() {
            return section
                .child(self.section_message(error.clone(), self.theme.danger))
                .into_any_element();
        }
        if self.changes.is_empty() {
            return section
                .child(self.section_message(
                    t!("WorkspaceExplorer.state.working_tree_clean").to_string(),
                    self.theme.muted_foreground,
                ))
                .into_any_element();
        }
        section = section.children(
            self.changes
                .iter()
                .cloned()
                .map(|change| self.render_change_row(change, cx)),
        );
        section.into_any_element()
    }

    fn section_message(&self, message: String, color: gpui::Hsla) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .text_xs()
            .text_color(color)
            .child(message)
    }

    fn render_change_row(&self, change: GitChange, cx: &mut Context<Self>) -> AnyElement {
        let badge_color = match change.kind {
            GitChangeKind::Added | GitChangeKind::Untracked => self.theme.success,
            GitChangeKind::Deleted | GitChangeKind::Conflicted => self.theme.danger,
            GitChangeKind::Modified | GitChangeKind::Renamed => self.theme.warning,
        };
        let path = change.path.display().to_string();
        let change_for_click = change.clone();
        h_flex()
            .id(SharedString::from(format!("workspace-git-change-{path}")))
            .items_center()
            .gap_2()
            .h(px(27.0))
            .px_2()
            .cursor_pointer()
            .hover(|style| style.bg(self.theme.muted))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_change(change_for_click.clone(), window, cx);
            }))
            .child(
                div()
                    .w(px(18.0))
                    .text_center()
                    .text_xs()
                    .font_semibold()
                    .text_color(badge_color)
                    .child(change.kind.badge()),
            )
            .child(div().flex_1().min_w_0().truncate().text_sm().child(path))
            .when(change.staged, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(self.theme.muted_foreground)
                        .child(t!("WorkspaceExplorer.badge.staged")),
                )
            })
            .into_any_element()
    }

    fn render_files(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = visible_rows(&self.root, &self.listings, &self.expanded);
        let mut section = v_flex()
            .w_full()
            .child(self.render_section_header(ExplorerSection::Files, cx));
        if self.files_expanded {
            section = section.children(rows.into_iter().map(|row| self.render_file_row(row, cx)));
        }
        section
    }

    fn render_file_row(&self, row: ExplorerRow, cx: &mut Context<Self>) -> AnyElement {
        let path = row.entry.path.clone();
        let is_dir = row.entry.is_dir;
        let selected = self.selected_path.as_ref() == Some(&path);
        let loading = self.loading_directories.contains(&path);
        h_flex()
            .id(SharedString::from(format!(
                "workspace-file-row-{}",
                path.display()
            )))
            .items_center()
            .gap_1()
            .h(px(27.0))
            .pl(px(8.0 + row.depth as f32 * 14.0))
            .pr_2()
            .cursor_pointer()
            .when(selected, |this| this.bg(self.theme.accent))
            .hover(|style| style.bg(self.theme.muted))
            .on_click(cx.listener(move |this, _, window, cx| {
                if is_dir {
                    this.toggle_directory(path.clone(), cx);
                } else {
                    this.open_file(path.clone(), window, cx);
                }
            }))
            .child(self.render_file_icon(&row, loading))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .child(row.entry.name),
            )
            .into_any_element()
    }

    fn render_file_icon(&self, row: &ExplorerRow, loading: bool) -> Icon {
        let icon = match (row.entry.is_dir, row.expanded, loading) {
            (_, _, true) => IconName::LoaderCircle,
            (true, true, false) => IconName::FolderOpen,
            (true, false, false) => IconName::FolderClosed,
            (false, _, false) => IconName::File,
        };
        Icon::new(icon)
            .with_size(Size::Small)
            .text_color(if row.entry.is_dir {
                self.theme.warning
            } else {
                self.theme.muted_foreground
            })
    }
}

impl Render for WorkspaceExplorer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(self.theme.background)
            .text_color(self.theme.foreground)
            .child(self.render_root_bar(cx))
            .child(
                v_flex()
                    .id("workspace-explorer-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll_handle)
                    .when(self.repository.is_some(), |this| {
                        this.child(self.render_changes(cx))
                    })
                    .child(self.render_files(cx))
                    .when(self.loading, |this| {
                        this.child(self.section_message(
                            t!("WorkspaceExplorer.state.loading_workspace").to_string(),
                            self.theme.muted_foreground,
                        ))
                    })
                    .when_some(self.error.clone(), |this, error| {
                        this.child(self.section_message(error, self.theme.danger))
                    }),
            )
    }
}
