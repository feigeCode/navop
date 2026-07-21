use super::WorkspaceExplorer;
use crate::git::{
    GitBranch, GitBranchKind, GitRepository, create_branch, delete_branch, fetch_branches,
    load_branches, merge_branch, rename_branch, switch_branch,
};
use gpui::{
    Anchor, AnyElement, AppContext as _, AsyncApp, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, Size,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    notification::Notification,
    v_flex,
};
use rust_i18n::t;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BranchTab {
    Local,
    Remote,
}

enum BranchOperation {
    Switch(GitBranch),
    Create(String),
    Rename { old_name: String, new_name: String },
    Merge(String),
    Delete(GitBranch),
    Fetch,
}

impl BranchOperation {
    fn run(self, repository: &GitRepository) -> anyhow::Result<()> {
        match self {
            Self::Switch(branch) => switch_branch(repository, &branch),
            Self::Create(name) => create_branch(repository, &name),
            Self::Rename { old_name, new_name } => rename_branch(repository, &old_name, &new_name),
            Self::Merge(name) => merge_branch(repository, &name),
            Self::Delete(branch) => delete_branch(repository, &branch),
            Self::Fetch => fetch_branches(repository),
        }
    }
}

pub(super) struct BranchManager {
    repository: GitRepository,
    explorer: WeakEntity<WorkspaceExplorer>,
    branches: Vec<GitBranch>,
    active_tab: BranchTab,
    search: Entity<InputState>,
    query: String,
    loading: bool,
    operating: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl BranchManager {
    pub(super) fn new(
        repository: GitRepository,
        explorer: WeakEntity<WorkspaceExplorer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("WorkspaceExplorer.branch.search_placeholder").to_string())
        });
        let subscription = cx.subscribe(&search, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = input.read(cx).value().trim().to_lowercase();
                cx.notify();
            }
        });
        let mut this = Self {
            repository,
            explorer,
            branches: Vec::new(),
            active_tab: BranchTab::Local,
            search,
            query: String::new(),
            loading: false,
            operating: false,
            error: None,
            _subscriptions: vec![subscription],
        };
        this.reload(cx);
        this
    }

    pub(super) fn search_input(&self) -> &Entity<InputState> {
        &self.search
    }

    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.error = None;
        let repository = self.repository.clone();
        let task = cx.background_spawn(async move { load_branches(&repository) });
        let entity = cx.entity().downgrade();
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = entity.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(branches) => this.branches = branches,
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn set_tab(&mut self, tab: BranchTab, cx: &mut Context<Self>) {
        self.active_tab = tab;
        cx.notify();
    }

    fn filtered_branches(&self) -> Vec<GitBranch> {
        let kind = match self.active_tab {
            BranchTab::Local => GitBranchKind::Local,
            BranchTab::Remote => GitBranchKind::Remote,
        };
        self.branches
            .iter()
            .filter(|branch| {
                branch.kind == kind
                    && (self.query.is_empty()
                        || branch.name.to_lowercase().contains(self.query.as_str()))
            })
            .cloned()
            .collect()
    }

    fn execute(
        &mut self,
        operation: BranchOperation,
        success_message: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.operating {
            return;
        }
        self.operating = true;
        self.error = None;
        let repository = self.repository.clone();
        let task = cx.background_spawn(async move { operation.run(&repository) });
        let entity = cx.entity().downgrade();
        let explorer = self.explorer.clone();
        let window_handle = window.window_handle();
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let Some(entity) = entity.upgrade() else {
                    return;
                };
                entity.update(cx, |this, cx| {
                    this.operating = false;
                    match result {
                        Ok(()) => {
                            window.push_notification(
                                Notification::success(success_message).autohide(true),
                                cx,
                            );
                        }
                        Err(error) => {
                            let message = error.to_string();
                            this.error = Some(message.clone());
                            window.push_notification(
                                Notification::error(message).autohide(false),
                                cx,
                            );
                        }
                    }
                    this.reload(cx);
                    let _ = explorer.update(cx, |explorer, cx| explorer.refresh(cx));
                    cx.notify();
                });
            });
        })
        .detach();
        cx.notify();
    }

    fn prompt_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("WorkspaceExplorer.branch.name_placeholder").to_string())
        });
        let manager = cx.entity();
        let dialog_input = input.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let manager = manager.clone();
            let input = dialog_input.clone();
            dialog
                .title(t!("WorkspaceExplorer.branch.create").to_string())
                .w(px(380.0))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("WorkspaceExplorer.branch.create").to_string())
                        .cancel_text(t!("WorkspaceExplorer.action.cancel").to_string()),
                )
                .on_ok(move |_, window, cx| {
                    let name = input.read(cx).value().trim().to_string();
                    if name.is_empty() {
                        return false;
                    }
                    manager.update(cx, |this, cx| {
                        this.execute(
                            BranchOperation::Create(name),
                            t!("WorkspaceExplorer.branch.created").to_string(),
                            window,
                            cx,
                        );
                    });
                    true
                })
                .child(Input::new(&dialog_input).w_full())
        });
    }

    fn prompt_rename(&mut self, branch: GitBranch, window: &mut Window, cx: &mut Context<Self>) {
        let current_name = branch.name.clone();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(current_name.clone())
                .placeholder(t!("WorkspaceExplorer.branch.name_placeholder").to_string())
        });
        let manager = cx.entity();
        let dialog_input = input.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let manager = manager.clone();
            let input = dialog_input.clone();
            let old_name = branch.name.clone();
            dialog
                .title(t!("WorkspaceExplorer.branch.rename").to_string())
                .w(px(380.0))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("WorkspaceExplorer.branch.rename").to_string())
                        .cancel_text(t!("WorkspaceExplorer.action.cancel").to_string()),
                )
                .on_ok(move |_, window, cx| {
                    let new_name = input.read(cx).value().trim().to_string();
                    if new_name.is_empty() || new_name == old_name {
                        return false;
                    }
                    manager.update(cx, |this, cx| {
                        this.execute(
                            BranchOperation::Rename {
                                old_name: old_name.clone(),
                                new_name,
                            },
                            t!("WorkspaceExplorer.branch.renamed").to_string(),
                            window,
                            cx,
                        );
                    });
                    true
                })
                .child(Input::new(&dialog_input).w_full())
        });
    }

    fn confirm_merge(&mut self, branch: GitBranch, window: &mut Window, cx: &mut Context<Self>) {
        let manager = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let manager = manager.clone();
            let branch_name = branch.name.clone();
            dialog
                .title(t!("WorkspaceExplorer.branch.merge").to_string())
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("WorkspaceExplorer.branch.merge").to_string())
                        .cancel_text(t!("WorkspaceExplorer.action.cancel").to_string()),
                )
                .on_ok(move |_, window, cx| {
                    manager.update(cx, |this, cx| {
                        this.execute(
                            BranchOperation::Merge(branch_name.clone()),
                            t!("WorkspaceExplorer.branch.merged").to_string(),
                            window,
                            cx,
                        );
                    });
                    true
                })
                .child(
                    div().child(
                        t!(
                            "WorkspaceExplorer.branch.merge_confirm",
                            name = branch.name.clone()
                        )
                        .to_string(),
                    ),
                )
        });
    }

    fn confirm_delete(&mut self, branch: GitBranch, window: &mut Window, cx: &mut Context<Self>) {
        let manager = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let manager = manager.clone();
            let branch_for_action = branch.clone();
            dialog
                .title(t!("WorkspaceExplorer.branch.delete").to_string())
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("WorkspaceExplorer.branch.delete").to_string())
                        .cancel_text(t!("WorkspaceExplorer.action.cancel").to_string()),
                )
                .on_ok(move |_, window, cx| {
                    manager.update(cx, |this, cx| {
                        this.execute(
                            BranchOperation::Delete(branch_for_action.clone()),
                            t!("WorkspaceExplorer.branch.deleted").to_string(),
                            window,
                            cx,
                        );
                    });
                    true
                })
                .child(
                    div().child(
                        t!(
                            "WorkspaceExplorer.branch.delete_confirm",
                            name = branch.name.clone()
                        )
                        .to_string(),
                    ),
                )
        });
    }

    fn render_branch(&self, branch: GitBranch, cx: &mut Context<Self>) -> AnyElement {
        let branch_for_switch = branch.clone();
        h_flex()
            .id(SharedString::from(format!(
                "workspace-branch-{:?}-{}",
                branch.kind, branch.name
            )))
            .items_center()
            .gap_2()
            .h(px(32.0))
            .px_2()
            .rounded(px(4.0))
            .when(!branch.current && !self.operating, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(cx.theme().muted))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.execute(
                            BranchOperation::Switch(branch_for_switch.clone()),
                            t!("WorkspaceExplorer.branch.switched").to_string(),
                            window,
                            cx,
                        );
                    }))
            })
            .child(
                Icon::new(if branch.current {
                    IconName::Check
                } else if branch.kind == GitBranchKind::Remote {
                    IconName::Globe
                } else {
                    IconName::Dash
                })
                .with_size(Size::XSmall),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().text_sm().child(branch.name.clone()))
                    .when_some(branch.upstream.clone(), |this, upstream| {
                        this.child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(upstream),
                        )
                    }),
            )
            .child(self.render_branch_actions(branch, cx))
            .into_any_element()
    }

    fn render_branch_actions(&self, branch: GitBranch, cx: &mut Context<Self>) -> impl IntoElement {
        let manager = cx.entity();
        Button::new(SharedString::from(format!(
            "workspace-branch-actions-{:?}-{}",
            branch.kind, branch.name
        )))
        .icon(IconName::Ellipsis)
        .ghost()
        .compact()
        .disabled(self.operating)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
            build_branch_actions_menu(menu, manager.clone(), branch.clone(), window, cx)
        })
    }
}

impl Render for BranchManager {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let branches = self.filtered_branches();
        let local_count = self
            .branches
            .iter()
            .filter(|branch| branch.kind == GitBranchKind::Local)
            .count();
        let remote_count = self
            .branches
            .iter()
            .filter(|branch| branch.kind == GitBranchKind::Remote)
            .count();
        v_flex()
            .w(px(380.0))
            .max_h(px(500.0))
            .gap_2()
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.search).w_full()),
                    )
                    .child(
                        Button::new("workspace-branch-create")
                            .icon(IconName::Plus)
                            .ghost()
                            .compact()
                            .tooltip(t!("WorkspaceExplorer.branch.create"))
                            .disabled(self.operating)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.prompt_create(window, cx);
                            })),
                    )
                    .child(
                        Button::new("workspace-branch-fetch")
                            .icon(IconName::Refresh)
                            .ghost()
                            .compact()
                            .tooltip(t!("WorkspaceExplorer.branch.fetch"))
                            .disabled(self.operating)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.execute(
                                    BranchOperation::Fetch,
                                    t!("WorkspaceExplorer.branch.fetched").to_string(),
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("workspace-branches-local")
                            .label(
                                t!("WorkspaceExplorer.branch.local", count = local_count)
                                    .to_string(),
                            )
                            .selected(self.active_tab == BranchTab::Local)
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_tab(BranchTab::Local, cx);
                            })),
                    )
                    .child(
                        Button::new("workspace-branches-remote")
                            .label(
                                t!("WorkspaceExplorer.branch.remote", count = remote_count)
                                    .to_string(),
                            )
                            .selected(self.active_tab == BranchTab::Remote)
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_tab(BranchTab::Remote, cx);
                            })),
                    ),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .child(
                v_flex()
                    .id("workspace-branch-list")
                    .min_h(px(120.0))
                    .max_h(px(360.0))
                    .overflow_y_scroll()
                    .when(self.loading, |this| {
                        this.child(
                            div()
                                .p_3()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("WorkspaceExplorer.branch.loading")),
                        )
                    })
                    .when(!self.loading && branches.is_empty(), |this| {
                        this.child(
                            div()
                                .p_3()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("WorkspaceExplorer.branch.empty")),
                        )
                    })
                    .children(
                        branches
                            .into_iter()
                            .map(|branch| self.render_branch(branch, cx)),
                    ),
            )
    }
}

fn build_branch_actions_menu(
    menu: PopupMenu,
    manager: Entity<BranchManager>,
    branch: GitBranch,
    _window: &mut Window,
    _cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let rename_manager = manager.clone();
    let merge_manager = manager.clone();
    let delete_manager = manager;
    menu.min_w(px(180.0))
        .when(branch.kind == GitBranchKind::Local, |menu| {
            let branch = branch.clone();
            menu.item(
                PopupMenuItem::new(t!("WorkspaceExplorer.branch.rename"))
                    .icon(IconName::Replace)
                    .on_click(move |_, window, cx| {
                        rename_manager.update(cx, |this, cx| {
                            this.prompt_rename(branch.clone(), window, cx);
                        });
                    }),
            )
        })
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.branch.merge"))
                .icon(IconName::Redo2)
                .disabled(branch.current)
                .on_click({
                    let branch = branch.clone();
                    move |_, window, cx| {
                        merge_manager.update(cx, |this, cx| {
                            this.confirm_merge(branch.clone(), window, cx);
                        });
                    }
                }),
        )
        .separator()
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.branch.delete"))
                .icon(IconName::Delete)
                .disabled(branch.current)
                .on_click(move |_, window, cx| {
                    delete_manager.update(cx, |this, cx| {
                        this.confirm_delete(branch.clone(), window, cx);
                    });
                }),
        )
}
