use super::WorkspaceExplorer;
use super::frame::{ExplorerFramePlacement, WorkspaceExplorerEvent};
use std::path::{Path, PathBuf};

use gpui::{
    Anchor, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Icon, Sizable as _, Size, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    popover::Popover,
};
use one_assets::IconName;
use one_ui::IconSize;
use one_ui::{IconButton, PanelHeader, PanelHeaderVariant};
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
        let trailing = h_flex()
            .flex_shrink_0()
            .items_center()
            .when_some(branch.zip(branch_manager), |this, (branch, manager)| {
                let search_focus = manager.read(cx).search_input().focus_handle(cx);
                this.child(
                    Popover::new("workspace-branch-manager")
                        .anchor(Anchor::TopRight)
                        .appearance(false)
                        .track_focus(&search_focus)
                        .trigger(
                            Button::new("workspace-current-branch")
                                .label(branch)
                                .icon(IconName::ChevronsUpDown)
                                .ghost()
                                .compact()
                                .custom(self.theme.icon_button_style(cx))
                                .tooltip(t!("WorkspaceExplorer.branch.manage")),
                        )
                        .content(move |_, _, _| manager.clone()),
                )
            })
            .child(self.render_header_actions(cx));

        PanelHeader::new("workspace-root-header")
            .variant(PanelHeaderVariant::Panel)
            .background(self.theme.muted)
            .border_color(self.theme.border)
            .leading(
                Icon::new(IconName::FolderOpen)
                    .with_size(IconSize::Small)
                    .text_color(self.theme.foreground),
            )
            .title(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_semibold()
                    .child(label),
            )
            .trailing(trailing)
    }

    fn render_header_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_shrink_0()
            .items_center()
            .child(
                IconButton::new("workspace-refresh", IconName::Refresh)
                    .tooltip(t!("WorkspaceExplorer.tooltip.refresh"))
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            )
            .child(
                IconButton::new("workspace-collapse-all", IconName::ChevronsUpDown)
                    .tooltip(t!("WorkspaceExplorer.tooltip.collapse_folders"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.expanded.clear();
                        cx.notify();
                    })),
            )
            .child(
                IconButton::new("workspace-selector", IconName::FolderOpen)
                    .tooltip(t!("WorkspaceExplorer.workspace.tooltip").to_string())
                    .dropdown_menu_with_anchor(Anchor::TopRight, {
                        let view = cx.entity();
                        let worktrees = self.worktrees.clone();
                        let recent_roots = self.recent_roots.clone();
                        let current_root = self.root.clone();
                        let has_repository = self.repository.is_some();
                        move |menu, _window, _cx| {
                            build_workspace_menu(
                                menu,
                                view.clone(),
                                &worktrees,
                                &recent_roots,
                                &current_root,
                                has_repository,
                            )
                        }
                    }),
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
        IconButton::new("workspace-frame-options", IconName::Ellipsis)
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
        IconButton::new("workspace-frame-close", IconName::Close)
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
            .h(one_ui::theme_geometry().layout.list_header)
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

/// worktree 菜单：创建、切换、删除。
///
/// 列表来自 `git worktree list`，所以应用重启后依然有效；删除只对带 `navop/`
/// 前缀的受管 worktree 开放，主工作区不可删。
/// 工作区菜单：选择目录、最近工作区、以及隔离 worktree 的切换/删除。
///
/// 工作区的唯一入口就放在这里，且不受 `show_frame_controls` 影响——工作台把
/// frame controls 关掉了，不能因此让用户失去选择工作区的能力。
fn build_workspace_menu(
    mut menu: PopupMenu,
    view: Entity<WorkspaceExplorer>,
    worktrees: &[crate::git::WorktreeEntry],
    recent_roots: &[PathBuf],
    current_root: &Path,
    has_repository: bool,
) -> PopupMenu {
    let choose_view = view.clone();
    menu = menu
        .min_w(px(280.0))
        .label(current_root.display().to_string())
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.workspace.choose").to_string())
                .icon(IconName::FolderOpen)
                .on_click(move |_, window, cx| {
                    choose_view.update(cx, |this, cx| this.choose_root(window, cx));
                }),
        );

    let recent = recent_workspace_choices(recent_roots, current_root);
    if !recent.is_empty() {
        menu = menu
            .separator()
            .label(t!("WorkspaceExplorer.workspace.recent").to_string());
        for root in recent {
            let label = workspace_label(&root);
            let switch_view = view.clone();
            menu = menu.item(
                PopupMenuItem::new(label)
                    .icon(IconName::FolderOpen)
                    .on_click(move |_, _, cx| {
                        let root = root.clone();
                        switch_view.update(cx, |this, cx| this.switch_workspace(root, cx));
                    }),
            );
        }
    }

    let commit_view = view.clone();
    let push_view = view.clone();
    let create_view = view.clone();
    menu = menu
        .separator()
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.commit.menu").to_string())
                .icon(IconName::Upload)
                .disabled(!has_repository)
                .on_click(move |_, window, cx| {
                    commit_view.update(cx, |this, cx| this.prompt_commit(window, cx));
                }),
        )
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.push.menu").to_string())
                .icon(IconName::ArrowUp)
                .disabled(!has_repository)
                .on_click(move |_, window, cx| {
                    push_view.update(cx, |this, cx| this.push_current(window, cx));
                }),
        )
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.worktree.create").to_string())
                .icon(IconName::GitBranch)
                .disabled(!has_repository)
                .on_click(move |_, _, cx| {
                    create_view.update(cx, |this, cx| this.create_worktree(cx));
                }),
        );

    let linked: Vec<&crate::git::WorktreeEntry> = worktrees
        .iter()
        .filter(|entry| !entry.is_main)
        .collect();
    if linked.is_empty() {
        return menu;
    }
    menu = menu
        .separator()
        .label(t!("WorkspaceExplorer.worktree.section", count = linked.len()).to_string());
    for entry in linked {
        let label = worktree_label(entry);
        let is_current = entry.path == current_root;
        let switch_view = view.clone();
        let switch_root = entry.path.clone();
        menu = menu.item(
            PopupMenuItem::new(label.clone())
                .icon(IconName::FolderOpen)
                .checked(is_current)
                .on_click(move |_, _, cx| {
                    let root = switch_root.clone();
                    switch_view.update(cx, |this, cx| this.switch_worktree(root, cx));
                }),
        );
        if entry.managed && !entry.is_main {
            let remove_view = view.clone();
            let remove_root = entry.path.clone();
            menu = menu.item(
                PopupMenuItem::new(
                    t!("WorkspaceExplorer.worktree.remove", name = label).to_string(),
                )
                .icon(IconName::Delete)
                .on_click(move |_, _, cx| {
                    let root = remove_root.clone();
                    remove_view.update(cx, |this, cx| this.confirm_remove_worktree(root, cx));
                }),
            );
        }
    }
    menu
}

/// 最近工作区里可切换的项：排除当前项与已失效目录。
fn recent_workspace_choices(recent_roots: &[PathBuf], current_root: &Path) -> Vec<PathBuf> {
    recent_roots
        .iter()
        .filter(|root| root.as_path() != current_root && root.is_dir())
        .cloned()
        .collect()
}

fn workspace_label(root: &Path) -> String {
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    let parent = root
        .parent()
        .map(|parent| parent.display().to_string())
        .unwrap_or_default();
    if parent.is_empty() {
        name
    } else {
        format!("{name}  —  {parent}")
    }
}

fn worktree_label(entry: &crate::git::WorktreeEntry) -> String {
    entry
        .branch
        .clone()
        .unwrap_or_else(|| {
            entry
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| entry.path.display().to_string())
        })
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
    let choose_root_view = view.clone();
    let follow_view = view.clone();
    let hidden_view = view.clone();
    let ignored_view = view.clone();
    let follows_terminal_cwd = view.read(cx).follows_terminal_cwd();
    menu.min_w(px(220.0))
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.frame.choose_root").to_string())
                .icon(IconName::FolderOpen)
                .on_click(move |_, window, cx| {
                    choose_root_view.update(cx, |this, cx| {
                        this.choose_root(window, cx);
                    });
                }),
        )
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.frame.follow_terminal_cwd").to_string())
                .icon(IconName::Refresh)
                .checked(follows_terminal_cwd)
                .on_click(move |_, _, cx| {
                    follow_view.update(cx, |this, cx| {
                        this.set_follow_terminal_cwd(!follows_terminal_cwd, cx);
                    });
                }),
        )
        .separator()
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

#[cfg(test)]
mod workspace_menu_tests {
    use super::{recent_workspace_choices, workspace_label};
    use std::path::PathBuf;

    #[test]
    fn recent_choices_skip_the_current_root_and_missing_directories() {
        let existing = std::env::temp_dir();
        let roots = vec![
            existing.clone(),
            PathBuf::from("/nonexistent/navop-workspace-test"),
            PathBuf::from("/tmp"),
        ];

        let choices = recent_workspace_choices(&roots, &existing);

        assert!(!choices.contains(&existing), "当前工作区不应出现在最近列表");
        assert!(
            choices.iter().all(|root| root.is_dir()),
            "失效目录不应出现在最近列表: {choices:?}"
        );
        assert!(choices.contains(&PathBuf::from("/tmp")));
    }

    #[test]
    fn workspace_label_shows_name_and_parent() {
        assert_eq!(
            "app  —  /Users/me/projects",
            workspace_label(std::path::Path::new("/Users/me/projects/app"))
        );
        assert_eq!("app", workspace_label(std::path::Path::new("app")));
    }
}
