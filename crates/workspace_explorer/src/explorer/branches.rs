use super::WorkspaceExplorer;
use crate::git::{
    GitBranch, GitBranchKind, GitRepository, create_branch, delete_branch, fetch_branches,
    load_branches, merge_branch, rename_branch, switch_branch,
};
use crate::theme::WorkspaceTheme;
use gpui::{
    Anchor, AnyElement, AppContext as _, AsyncApp, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Disableable as _, Icon, IconName, Sizable as _, Size, StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, LocalInputStyle},
    notification::Notification,
    popover::{Popover, PopoverState},
    v_flex,
};
use rust_i18n::t;

#[derive(Clone)]
enum BranchOperation {
    Switch(GitBranch),
    Create(String),
    Rename { old_name: String, new_name: String },
    Merge(String),
    Delete(GitBranch),
    Fetch,
}

#[derive(Clone)]
enum BranchEditorMode {
    Create,
    Rename(String),
}

#[derive(Clone)]
struct BranchEditor {
    mode: BranchEditorMode,
    input: Entity<InputState>,
}

#[derive(Clone)]
struct BranchConfirmation {
    operation: BranchOperation,
    message: String,
    success_message: String,
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
    theme: WorkspaceTheme,
    branches: Vec<GitBranch>,
    search: Entity<InputState>,
    query: String,
    loading: bool,
    operating: bool,
    error: Option<String>,
    editor: Option<BranchEditor>,
    confirmation: Option<BranchConfirmation>,
    _subscriptions: Vec<Subscription>,
}

impl BranchManager {
    pub(super) fn new(
        repository: GitRepository,
        explorer: WeakEntity<WorkspaceExplorer>,
        theme: WorkspaceTheme,
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
            theme,
            branches: Vec::new(),
            search,
            query: String::new(),
            loading: false,
            operating: false,
            error: None,
            editor: None,
            confirmation: None,
            _subscriptions: vec![subscription],
        };
        this.reload(cx);
        this
    }

    pub(super) fn search_input(&self) -> &Entity<InputState> {
        &self.search
    }

    pub(super) fn set_theme(&mut self, theme: WorkspaceTheme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
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

    fn filtered_branches(&self, kind: GitBranchKind) -> Vec<GitBranch> {
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
                    let _ = explorer.update(cx, |explorer, cx| {
                        explorer.refresh_after_branch_operation(cx);
                    });
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
        self.editor = Some(BranchEditor {
            mode: BranchEditorMode::Create,
            input: input.clone(),
        });
        self.confirmation = None;
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn prompt_rename(&mut self, branch: GitBranch, window: &mut Window, cx: &mut Context<Self>) {
        let current_name = branch.name.clone();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(current_name.clone())
                .placeholder(t!("WorkspaceExplorer.branch.name_placeholder").to_string())
        });
        self.editor = Some(BranchEditor {
            mode: BranchEditorMode::Rename(branch.name),
            input: input.clone(),
        });
        self.confirmation = None;
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn submit_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.clone() else {
            return;
        };
        let name = editor.input.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        let (operation, message) = match editor.mode {
            BranchEditorMode::Create => (
                BranchOperation::Create(name),
                t!("WorkspaceExplorer.branch.created").to_string(),
            ),
            BranchEditorMode::Rename(old_name) if old_name != name => (
                BranchOperation::Rename {
                    old_name,
                    new_name: name,
                },
                t!("WorkspaceExplorer.branch.renamed").to_string(),
            ),
            BranchEditorMode::Rename(_) => return,
        };
        self.editor = None;
        self.execute(operation, message, window, cx);
    }

    fn cancel_inline_action(&mut self, cx: &mut Context<Self>) {
        self.editor = None;
        self.confirmation = None;
        cx.notify();
    }

    fn confirm_merge(&mut self, branch: GitBranch, cx: &mut Context<Self>) {
        self.editor = None;
        self.confirmation = Some(BranchConfirmation {
            operation: BranchOperation::Merge(branch.name.clone()),
            message: t!("WorkspaceExplorer.branch.merge_confirm", name = branch.name).to_string(),
            success_message: t!("WorkspaceExplorer.branch.merged").to_string(),
        });
        cx.notify();
    }

    fn confirm_delete(&mut self, branch: GitBranch, cx: &mut Context<Self>) {
        self.editor = None;
        self.confirmation = Some(BranchConfirmation {
            operation: BranchOperation::Delete(branch.clone()),
            message: t!(
                "WorkspaceExplorer.branch.delete_confirm",
                name = branch.name
            )
            .to_string(),
            success_message: t!("WorkspaceExplorer.branch.deleted").to_string(),
        });
        cx.notify();
    }

    fn submit_confirmation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(confirmation) = self.confirmation.take() else {
            return;
        };
        self.execute(
            confirmation.operation,
            confirmation.success_message,
            window,
            cx,
        );
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
            .when(branch.current, |this| this.bg(self.theme.muted))
            .when(!branch.current && !self.operating, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(self.theme.muted))
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
                .with_size(Size::XSmall)
                .text_color(if branch.current {
                    self.theme.accent
                } else {
                    self.theme.muted_foreground
                }),
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
                                .text_color(self.theme.muted_foreground)
                                .child(upstream),
                        )
                    }),
            )
            .child(self.render_branch_actions(branch, cx))
            .into_any_element()
    }

    fn render_branch_actions(&self, branch: GitBranch, cx: &mut Context<Self>) -> impl IntoElement {
        let manager = cx.entity();
        let theme = self.theme;
        let popover_id = SharedString::from(format!(
            "workspace-branch-actions-popover-{:?}-{}",
            branch.kind, branch.name
        ));
        Popover::new(popover_id)
            .anchor(Anchor::TopRight)
            .appearance(false)
            .trigger(
                Button::new(SharedString::from(format!(
                    "workspace-branch-actions-{:?}-{}",
                    branch.kind, branch.name
                )))
                .icon(IconName::Ellipsis)
                .ghost()
                .compact()
                .custom(self.theme.icon_button_style(cx))
                .disabled(self.operating),
            )
            .content(move |_, _, cx| {
                render_branch_actions_popover(manager.clone(), branch.clone(), theme, cx)
            })
    }

    fn render_branch_section(
        &self,
        title: String,
        branches: Vec<GitBranch>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .when(!branches.is_empty(), |this| {
                this.child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .h(px(28.0))
                        .px_2()
                        .text_xs()
                        .font_semibold()
                        .text_color(self.theme.muted_foreground)
                        .child(
                            Icon::new(IconName::ChevronDown)
                                .with_size(Size::XSmall)
                                .text_color(self.theme.muted_foreground),
                        )
                        .child(title),
                )
                .children(
                    branches
                        .into_iter()
                        .map(|branch| self.render_branch(branch, cx)),
                )
            })
            .into_any_element()
    }
}

impl Render for BranchManager {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = self.editor.clone();
        let confirmation = self.confirmation.clone();
        let local = self.filtered_branches(GitBranchKind::Local);
        let remote = self.filtered_branches(GitBranchKind::Remote);
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
        let show_create = self.query.is_empty()
            || t!("WorkspaceExplorer.branch.create")
                .to_lowercase()
                .contains(&self.query);
        let show_fetch = self.query.is_empty()
            || t!("WorkspaceExplorer.branch.fetch")
                .to_lowercase()
                .contains(&self.query);
        let input_style = LocalInputStyle {
            background: self.theme.muted,
            foreground: self.theme.foreground,
            muted_foreground: self.theme.muted_foreground,
            border: self.theme.border,
        };
        v_flex()
            .w(px(420.0))
            .max_h(px(560.0))
            .gap_2()
            .p_3()
            .rounded(px(10.0))
            .border_1()
            .border_color(self.theme.border)
            .bg(self.theme.background)
            .text_color(self.theme.foreground)
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.search)
                                .w_full()
                                .prefix(Icon::new(IconName::Search))
                                .local_style(input_style),
                        ),
                    )
                    .child(
                        Button::new("workspace-branch-create")
                            .icon(IconName::Plus)
                            .ghost()
                            .compact()
                            .custom(self.theme.icon_button_style(cx))
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
                            .custom(self.theme.icon_button_style(cx))
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
            .when_some(editor, |this, editor| {
                let title = match editor.mode {
                    BranchEditorMode::Create => t!("WorkspaceExplorer.branch.create"),
                    BranchEditorMode::Rename(_) => t!("WorkspaceExplorer.branch.rename"),
                };
                this.child(
                    v_flex()
                        .gap_2()
                        .p_2()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(self.theme.border)
                        .bg(self.theme.muted)
                        .child(div().text_sm().font_semibold().child(title))
                        .child(
                            Input::new(&editor.input)
                                .w_full()
                                .local_style(LocalInputStyle {
                                    background: self.theme.background,
                                    foreground: self.theme.foreground,
                                    muted_foreground: self.theme.muted_foreground,
                                    border: self.theme.border,
                                }),
                        )
                        .child(
                            h_flex()
                                .justify_end()
                                .gap_1()
                                .child(
                                    Button::new("workspace-branch-inline-cancel")
                                        .label(t!("WorkspaceExplorer.action.cancel"))
                                        .small()
                                        .custom(self.theme.icon_button_style(cx))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.cancel_inline_action(cx);
                                        })),
                                )
                                .child(
                                    Button::new("workspace-branch-inline-submit")
                                        .label(t!("WorkspaceExplorer.action.save"))
                                        .small()
                                        .custom(self.theme.button_style(cx))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.submit_editor(window, cx);
                                        })),
                                ),
                        ),
                )
            })
            .when_some(confirmation, |this, confirmation| {
                this.child(
                    v_flex()
                        .gap_2()
                        .p_2()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(self.theme.warning)
                        .bg(self.theme.muted)
                        .child(div().text_sm().child(confirmation.message))
                        .child(
                            h_flex()
                                .justify_end()
                                .gap_1()
                                .child(
                                    Button::new("workspace-branch-confirm-cancel")
                                        .label(t!("WorkspaceExplorer.action.cancel"))
                                        .small()
                                        .custom(self.theme.icon_button_style(cx))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.cancel_inline_action(cx);
                                        })),
                                )
                                .child(
                                    Button::new("workspace-branch-confirm-submit")
                                        .label(t!("WorkspaceExplorer.action.save"))
                                        .small()
                                        .custom(self.theme.button_style(cx))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.submit_confirmation(window, cx);
                                        })),
                                ),
                        ),
                )
            })
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(self.theme.danger)
                        .child(error),
                )
            })
            .child(
                v_flex()
                    .id("workspace-branch-list")
                    .min_h(px(120.0))
                    .max_h(px(430.0))
                    .overflow_y_scroll()
                    .when(show_fetch, |this| {
                        this.child(
                            h_flex()
                                .id("workspace-branch-action-fetch")
                                .items_center()
                                .gap_2()
                                .h(px(34.0))
                                .px_2()
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .hover(|style| style.bg(self.theme.muted))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.execute(
                                        BranchOperation::Fetch,
                                        t!("WorkspaceExplorer.branch.fetched").to_string(),
                                        window,
                                        cx,
                                    );
                                }))
                                .child(
                                    Icon::new(IconName::Refresh)
                                        .with_size(Size::Small)
                                        .text_color(self.theme.muted_foreground),
                                )
                                .child(t!("WorkspaceExplorer.branch.fetch")),
                        )
                    })
                    .when(show_create, |this| {
                        this.child(
                            h_flex()
                                .id("workspace-branch-action-create")
                                .items_center()
                                .gap_2()
                                .h(px(34.0))
                                .px_2()
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .hover(|style| style.bg(self.theme.muted))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.prompt_create(window, cx);
                                }))
                                .child(
                                    Icon::new(IconName::Plus)
                                        .with_size(Size::Small)
                                        .text_color(self.theme.muted_foreground),
                                )
                                .child(t!("WorkspaceExplorer.branch.create")),
                        )
                    })
                    .when(show_create || show_fetch, |this| {
                        this.child(div().h(px(1.0)).my_2().mx_1().bg(self.theme.border))
                    })
                    .when(self.loading, |this| {
                        this.child(
                            div()
                                .p_3()
                                .text_sm()
                                .text_color(self.theme.muted_foreground)
                                .child(t!("WorkspaceExplorer.branch.loading")),
                        )
                    })
                    .when(
                        !self.loading && local.is_empty() && remote.is_empty(),
                        |this| {
                            this.child(
                                div()
                                    .p_3()
                                    .text_sm()
                                    .text_color(self.theme.muted_foreground)
                                    .child(t!("WorkspaceExplorer.branch.empty")),
                            )
                        },
                    )
                    .when(!self.loading, |this| {
                        this.child(self.render_branch_section(
                            t!("WorkspaceExplorer.branch.local", count = local_count).to_string(),
                            local,
                            cx,
                        ))
                        .child(self.render_branch_section(
                            t!("WorkspaceExplorer.branch.remote", count = remote_count).to_string(),
                            remote,
                            cx,
                        ))
                    }),
            )
    }
}

fn render_branch_actions_popover(
    manager: Entity<BranchManager>,
    branch: GitBranch,
    theme: WorkspaceTheme,
    cx: &mut Context<PopoverState>,
) -> AnyElement {
    let popover_entity = cx.entity();
    let rename_manager = manager.clone();
    let merge_manager = manager.clone();
    let delete_manager = manager;
    let rename_popover = popover_entity.clone();
    let merge_popover = popover_entity.clone();
    let delete_popover = popover_entity;
    v_flex()
        .w(px(220.0))
        .p_1()
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .text_color(theme.foreground)
        .when(branch.kind == GitBranchKind::Local, |this| {
            let branch = branch.clone();
            this.child(
                h_flex()
                    .id("workspace-branch-action-rename")
                    .items_center()
                    .gap_2()
                    .h(px(34.0))
                    .px_2()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.muted))
                    .on_click(move |_, window, cx| {
                        rename_popover.update(cx, |popover, cx| {
                            popover.dismiss(window, cx);
                        });
                        rename_manager.update(cx, |manager, cx| {
                            manager.prompt_rename(branch.clone(), window, cx);
                        });
                    })
                    .child(Icon::new(IconName::Replace).with_size(Size::XSmall))
                    .child(t!("WorkspaceExplorer.branch.rename")),
            )
        })
        .when(!branch.current, |this| {
            let branch = branch.clone();
            this.child(
                h_flex()
                    .id("workspace-branch-action-merge")
                    .items_center()
                    .gap_2()
                    .h(px(34.0))
                    .px_2()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.muted))
                    .on_click(move |_, window, cx| {
                        merge_popover.update(cx, |popover, cx| {
                            popover.dismiss(window, cx);
                        });
                        merge_manager.update(cx, |manager, cx| {
                            manager.confirm_merge(branch.clone(), cx);
                        });
                    })
                    .child(Icon::new(IconName::Redo2).with_size(Size::XSmall))
                    .child(t!("WorkspaceExplorer.branch.merge")),
            )
        })
        .when(!branch.current, |this| {
            let branch = branch.clone();
            this.child(div().h(px(1.0)).mx_1().my_1().bg(theme.border))
                .child(
                    h_flex()
                        .id("workspace-branch-action-delete")
                        .items_center()
                        .gap_2()
                        .h(px(34.0))
                        .px_2()
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .text_color(theme.danger)
                        .hover(|style| style.bg(theme.muted))
                        .on_click(move |_, window, cx| {
                            delete_popover.update(cx, |popover, cx| {
                                popover.dismiss(window, cx);
                            });
                            delete_manager.update(cx, |manager, cx| {
                                manager.confirm_delete(branch.clone(), cx);
                            });
                        })
                        .child(Icon::new(IconName::Delete).with_size(Size::XSmall))
                        .child(t!("WorkspaceExplorer.branch.delete")),
                )
        })
        .into_any_element()
}
