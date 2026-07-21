use super::WorkspaceExplorer;
use super::frame::{ExplorerFramePlacement, WorkspaceExplorerEvent};
use gpui::{
    Anchor, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Icon, IconName, Sizable as _, Size, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    popover::Popover,
};
use rust_i18n::t;

const FRAME_PLACEMENTS: [ExplorerFramePlacement; 3] = [
    ExplorerFramePlacement::Left,
    ExplorerFramePlacement::Right,
    ExplorerFramePlacement::Bottom,
];

#[derive(Clone, Copy)]
pub(super) enum ExplorerSection {
    Changes,
    Files,
}

impl WorkspaceExplorer {
    /// 合并后的单层面板头部：目录名 + 分支徽章 + 操作按钮 + 宿主框架控制。
    pub(super) fn render_root_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self
            .root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string());
        let branch = self
            .repository
            .as_ref()
            .and_then(|repository| repository.branch.clone());
        let branch_manager = self.branch_manager.clone();
        h_flex()
            .items_center()
            .gap_1()
            .h(px(34.0))
            .pl_2()
            .pr_1()
            .border_b_1()
            .border_color(self.theme.border)
            .bg(self.theme.muted)
            .child(
                Icon::new(IconName::FolderOpen)
                    .with_size(Size::Small)
                    .text_color(self.theme.foreground),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_semibold()
                    .child(label),
            )
            .when_some(branch.zip(branch_manager), |this, (branch, manager)| {
                let search_focus = manager.read(cx).search_input().focus_handle(cx);
                this.child(
                    Popover::new("workspace-branch-manager")
                        .anchor(Anchor::TopRight)
                        .track_focus(&search_focus)
                        .trigger(
                            Button::new("workspace-current-branch")
                                .label(branch)
                                .icon(IconName::ChevronsUpDown)
                                .ghost()
                                .compact()
                                .tooltip(t!("WorkspaceExplorer.branch.manage")),
                        )
                        .content(move |_, _, _| manager.clone()),
                )
            })
            .child(self.render_header_actions(cx))
    }

    fn render_header_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_shrink_0()
            .items_center()
            .child(
                Button::new("workspace-refresh")
                    .icon(IconName::Refresh)
                    .ghost()
                    .compact()
                    .tooltip(t!("WorkspaceExplorer.tooltip.refresh"))
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            )
            .child(
                Button::new("workspace-collapse-all")
                    .icon(IconName::ChevronsUpDown)
                    .ghost()
                    .compact()
                    .tooltip(t!("WorkspaceExplorer.tooltip.collapse_folders"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.expanded.clear();
                        cx.notify();
                    })),
            )
            .when(self.show_frame_controls, |this| {
                this.child(self.render_frame_options_button(cx))
                    .child(self.render_frame_close_button(cx))
            })
    }

    fn render_frame_options_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let placement = self.frame_placement;
        let show_hidden = self.show_hidden;
        let show_ignored = self.show_ignored;
        Button::new("workspace-frame-options")
            .icon(IconName::Ellipsis)
            .ghost()
            .compact()
            .tooltip(t!("WorkspaceExplorer.frame.options").to_string())
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
                build_frame_options_menu(
                    menu,
                    view.clone(),
                    placement,
                    show_hidden,
                    show_ignored,
                    window,
                    cx,
                )
            })
    }

    fn render_frame_close_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Button::new("workspace-frame-close")
            .icon(IconName::Close)
            .ghost()
            .compact()
            .tooltip(t!("WorkspaceExplorer.frame.close").to_string())
            .on_click(cx.listener(|_this, _, _, cx| {
                cx.emit(WorkspaceExplorerEvent::Close);
            }))
    }

    pub(super) fn render_section_header(
        &self,
        section: ExplorerSection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (id, label, expanded) = self.section_header_details(section);
        h_flex()
            .id(id)
            .items_center()
            .gap_1()
            .h(px(28.0))
            .px_2()
            .cursor_pointer()
            .bg(self.theme.muted.opacity(0.55))
            .hover(|style| style.bg(self.theme.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_section(section, cx);
            }))
            .child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .with_size(Size::XSmall)
                .text_color(self.theme.muted_foreground),
            )
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(self.theme.foreground)
                    .child(label),
            )
    }

    fn section_header_details(&self, section: ExplorerSection) -> (&'static str, String, bool) {
        match section {
            ExplorerSection::Changes => (
                "workspace-changes-header",
                t!(
                    "WorkspaceExplorer.section.changes",
                    count = self.changes.len()
                )
                .to_string(),
                self.changes_expanded,
            ),
            ExplorerSection::Files => (
                "workspace-files-header",
                t!("WorkspaceExplorer.section.files").to_string(),
                self.files_expanded,
            ),
        }
    }

    fn toggle_section(&mut self, section: ExplorerSection, cx: &mut Context<Self>) {
        match section {
            ExplorerSection::Changes => self.changes_expanded = !self.changes_expanded,
            ExplorerSection::Files => self.files_expanded = !self.files_expanded,
        }
        cx.notify();
    }
}

fn build_frame_options_menu(
    menu: PopupMenu,
    view: Entity<WorkspaceExplorer>,
    placement: ExplorerFramePlacement,
    show_hidden: bool,
    show_ignored: bool,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let remove_view = view.clone();
    let hidden_view = view.clone();
    let ignored_view = view.clone();
    menu.min_w(px(220.0))
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.tooltip.show_hidden_files").to_string())
                .icon(IconName::Eye)
                .checked(show_hidden)
                .on_click(move |_, _, cx| {
                    hidden_view.update(cx, |this, cx| this.toggle_show_hidden(cx));
                }),
        )
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.tooltip.show_ignored_files").to_string())
                .icon(IconName::Filter)
                .checked(show_ignored)
                .on_click(move |_, _, cx| {
                    ignored_view.update(cx, |this, cx| this.toggle_show_ignored(cx));
                }),
        )
        .separator()
        .submenu_with_icon(
            Some(IconName::PanelRight.into()),
            t!("WorkspaceExplorer.frame.move_to").to_string(),
            window,
            cx,
            move |submenu, _window, _cx| {
                FRAME_PLACEMENTS
                    .into_iter()
                    .fold(submenu, |submenu, option| {
                        let view = view.clone();
                        let current = option == placement;
                        submenu.item(
                            PopupMenuItem::new(frame_placement_label(option))
                                .icon(frame_placement_icon(option))
                                .checked(current)
                                .disabled(current)
                                .on_click(move |_, _, cx| {
                                    view.update(cx, |_this, cx| {
                                        cx.emit(WorkspaceExplorerEvent::MoveTo(option));
                                    });
                                }),
                        )
                    })
            },
        )
        .separator()
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.frame.remove").to_string())
                .icon(IconName::Close)
                .on_click(move |_, _, cx| {
                    remove_view.update(cx, |_this, cx| {
                        cx.emit(WorkspaceExplorerEvent::Close);
                    });
                }),
        )
}

fn frame_placement_label(placement: ExplorerFramePlacement) -> String {
    match placement {
        ExplorerFramePlacement::Left => t!("WorkspaceExplorer.frame.left").to_string(),
        ExplorerFramePlacement::Right => t!("WorkspaceExplorer.frame.right").to_string(),
        ExplorerFramePlacement::Bottom => t!("WorkspaceExplorer.frame.bottom").to_string(),
    }
}

fn frame_placement_icon(placement: ExplorerFramePlacement) -> IconName {
    match placement {
        ExplorerFramePlacement::Left => IconName::PanelLeft,
        ExplorerFramePlacement::Right => IconName::PanelRight,
        ExplorerFramePlacement::Bottom => IconName::PanelBottom,
    }
}
