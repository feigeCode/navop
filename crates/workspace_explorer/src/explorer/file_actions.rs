use super::WorkspaceExplorer;
use crate::file_system::{create_directory, create_file, delete_entry, rename_entry};
use crate::git::{GitChange, GitRepository, discard_change, stage_change, unstage_change};
use anyhow::Result;
use gpui::{
    AnyElement, AppContext as _, AsyncApp, Context, Entity, IntoElement, ParentElement as _,
    Styled as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Disableable as _, Icon, IconName, Sizable as _, Size, StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, LocalInputStyle},
    menu::{PopupMenu, PopupMenuItem},
    notification::Notification,
    v_flex,
};
use rust_i18n::t;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(super) enum FileActionEditorMode {
    CreateFile { parent: PathBuf },
    CreateDirectory { parent: PathBuf },
    Rename { path: PathBuf },
}

#[derive(Clone)]
pub(super) struct FileActionEditor {
    pub(super) mode: FileActionEditorMode,
    pub(super) input: Entity<InputState>,
}

#[derive(Clone)]
pub(super) enum ExplorerConfirmationOperation {
    Delete(PathBuf),
    Discard(GitChange),
}

#[derive(Clone)]
pub(super) struct ExplorerConfirmation {
    pub(super) operation: ExplorerConfirmationOperation,
    pub(super) message: String,
}

enum ExplorerOperation {
    CreateFile {
        parent: PathBuf,
        name: String,
    },
    CreateDirectory {
        parent: PathBuf,
        name: String,
    },
    Rename {
        path: PathBuf,
        new_name: String,
    },
    Delete(PathBuf),
    Stage {
        repository: GitRepository,
        change: GitChange,
    },
    Unstage {
        repository: GitRepository,
        change: GitChange,
    },
    Discard {
        repository: GitRepository,
        change: GitChange,
    },
}

#[derive(Clone)]
struct FileTreeRefresh {
    parent: PathBuf,
    remove_subtree: Option<PathBuf>,
}

impl ExplorerOperation {
    fn run(self) -> Result<()> {
        match self {
            Self::CreateFile { parent, name } => {
                create_file(&parent, &name)?;
            }
            Self::CreateDirectory { parent, name } => {
                create_directory(&parent, &name)?;
            }
            Self::Rename { path, new_name } => {
                rename_entry(&path, &new_name)?;
            }
            Self::Delete(path) => delete_entry(&path)?,
            Self::Stage { repository, change } => stage_change(&repository, &change)?,
            Self::Unstage { repository, change } => unstage_change(&repository, &change)?,
            Self::Discard { repository, change } => discard_change(&repository, &change)?,
        }
        Ok(())
    }

    fn file_tree_refresh(&self) -> Option<FileTreeRefresh> {
        match self {
            Self::CreateFile { parent, .. } | Self::CreateDirectory { parent, .. } => {
                Some(FileTreeRefresh {
                    parent: parent.clone(),
                    remove_subtree: None,
                })
            }
            Self::Rename { path, .. } | Self::Delete(path) => Some(FileTreeRefresh {
                parent: path.parent()?.to_path_buf(),
                remove_subtree: Some(path.clone()),
            }),
            Self::Stage { .. } | Self::Unstage { .. } | Self::Discard { .. } => None,
        }
    }
}

impl WorkspaceExplorer {
    pub(super) fn prompt_create_file(
        &mut self,
        parent: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_file_editor(
            FileActionEditorMode::CreateFile { parent },
            String::new(),
            t!("WorkspaceExplorer.file_action.file_name_placeholder").to_string(),
            window,
            cx,
        );
    }

    pub(super) fn prompt_create_directory(
        &mut self,
        parent: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_file_editor(
            FileActionEditorMode::CreateDirectory { parent },
            String::new(),
            t!("WorkspaceExplorer.file_action.directory_name_placeholder").to_string(),
            window,
            cx,
        );
    }

    pub(super) fn prompt_rename(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.prompt_file_editor(
            FileActionEditorMode::Rename { path },
            current_name,
            t!("WorkspaceExplorer.file_action.name_placeholder").to_string(),
            window,
            cx,
        );
    }

    fn prompt_file_editor(
        &mut self,
        mode: FileActionEditorMode,
        value: String,
        placeholder: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_operation_running {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value)
                .placeholder(placeholder)
        });
        self.file_action_editor = Some(FileActionEditor {
            mode,
            input: input.clone(),
        });
        self.file_action_subscription =
            Some(
                cx.subscribe_in(&input, window, |this, _, event, window, cx| {
                    if matches!(event, InputEvent::PressEnter { secondary: false }) {
                        this.submit_file_editor(window, cx);
                    }
                }),
            );
        self.file_confirmation = None;
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(super) fn confirm_delete(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.file_operation_running {
            return;
        }
        let name = display_name(&path);
        self.file_action_editor = None;
        self.file_action_subscription = None;
        self.file_confirmation = Some(ExplorerConfirmation {
            operation: ExplorerConfirmationOperation::Delete(path),
            message: t!("WorkspaceExplorer.file_action.delete_confirm", name = name).to_string(),
        });
        cx.notify();
    }

    pub(super) fn confirm_discard_change(&mut self, change: GitChange, cx: &mut Context<Self>) {
        if self.file_operation_running {
            return;
        }
        let path = change.path.display().to_string();
        self.file_action_editor = None;
        self.file_action_subscription = None;
        self.file_confirmation = Some(ExplorerConfirmation {
            operation: ExplorerConfirmationOperation::Discard(change),
            message: t!("WorkspaceExplorer.git_action.discard_confirm", path = path).to_string(),
        });
        cx.notify();
    }

    pub(super) fn stage_git_change(
        &mut self,
        change: GitChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        self.execute_explorer_operation(
            ExplorerOperation::Stage { repository, change },
            t!("WorkspaceExplorer.git_action.staged").to_string(),
            window,
            cx,
        );
    }

    pub(super) fn unstage_git_change(
        &mut self,
        change: GitChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        self.execute_explorer_operation(
            ExplorerOperation::Unstage { repository, change },
            t!("WorkspaceExplorer.git_action.unstaged").to_string(),
            window,
            cx,
        );
    }

    fn submit_file_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.file_action_editor.clone() else {
            return;
        };
        let name = editor.input.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        let (operation, success_message) = match editor.mode {
            FileActionEditorMode::CreateFile { parent } => (
                ExplorerOperation::CreateFile { parent, name },
                t!("WorkspaceExplorer.file_action.file_created").to_string(),
            ),
            FileActionEditorMode::CreateDirectory { parent } => (
                ExplorerOperation::CreateDirectory { parent, name },
                t!("WorkspaceExplorer.file_action.directory_created").to_string(),
            ),
            FileActionEditorMode::Rename { path } => {
                if path
                    .file_name()
                    .is_some_and(|current| current == name.as_str())
                {
                    self.cancel_file_action(cx);
                    return;
                }
                (
                    ExplorerOperation::Rename {
                        path,
                        new_name: name,
                    },
                    t!("WorkspaceExplorer.file_action.renamed").to_string(),
                )
            }
        };
        self.file_action_editor = None;
        self.file_action_subscription = None;
        self.execute_explorer_operation(operation, success_message, window, cx);
    }

    fn submit_file_confirmation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(confirmation) = self.file_confirmation.take() else {
            return;
        };
        let (operation, success_message) = match confirmation.operation {
            ExplorerConfirmationOperation::Delete(path) => (
                ExplorerOperation::Delete(path),
                t!("WorkspaceExplorer.file_action.deleted").to_string(),
            ),
            ExplorerConfirmationOperation::Discard(change) => {
                let Some(repository) = self.repository.clone() else {
                    return;
                };
                (
                    ExplorerOperation::Discard { repository, change },
                    t!("WorkspaceExplorer.git_action.discarded").to_string(),
                )
            }
        };
        self.execute_explorer_operation(operation, success_message, window, cx);
    }

    fn cancel_file_action(&mut self, cx: &mut Context<Self>) {
        self.file_action_editor = None;
        self.file_confirmation = None;
        self.file_action_subscription = None;
        cx.notify();
    }

    fn execute_explorer_operation(
        &mut self,
        operation: ExplorerOperation,
        success_message: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_operation_running {
            return;
        }
        self.file_operation_running = true;
        self.file_action_editor = None;
        self.file_confirmation = None;
        self.file_action_subscription = None;
        let file_tree_refresh = operation.file_tree_refresh();
        let task = cx.background_spawn(async move { operation.run() });
        let entity = cx.entity().downgrade();
        let window_handle = window.window_handle();
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let Some(entity) = entity.upgrade() else {
                    return;
                };
                entity.update(cx, |this, cx| {
                    this.file_operation_running = false;
                    match result {
                        Ok(()) => {
                            window.push_notification(
                                Notification::success(success_message).autohide(true),
                                cx,
                            );
                            if let Some(refresh) = file_tree_refresh {
                                if let Some(path) = refresh.remove_subtree {
                                    this.remove_file_tree_state(&path);
                                }
                                this.reload_directory(refresh.parent, cx);
                            }
                            this.refresh_git(cx);
                        }
                        Err(error) => {
                            window.push_notification(
                                Notification::error(error.to_string()).autohide(false),
                                cx,
                            );
                            cx.notify();
                        }
                    }
                });
            });
        })
        .detach();
        cx.notify();
    }

    fn remove_file_tree_state(&mut self, path: &Path) {
        self.expanded
            .retain(|candidate| !candidate.starts_with(path));
        self.listings
            .retain(|candidate, _| !candidate.starts_with(path));
        if self
            .selected_path
            .as_ref()
            .is_some_and(|selected| selected.starts_with(path))
        {
            self.selected_path = None;
        }
    }

    fn reload_directory(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.loading_directories.contains(&path) {
            return;
        }
        self.loading_directories.insert(path.clone());
        let generation = self.refresh_generation;
        let task_path = path.clone();
        let show_hidden = self.show_hidden;
        let show_ignored = self.show_ignored;
        let matcher = self.ignore_matcher.clone();
        let task = cx.background_spawn(async move {
            crate::file_system::read_directory(
                &task_path,
                matcher.as_deref(),
                show_hidden,
                show_ignored,
            )
        });
        let entity = cx.entity().downgrade();
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = entity.update(cx, |this, cx| {
                if this.refresh_generation != generation {
                    return;
                }
                this.loading_directories.remove(&path);
                match result {
                    Ok(entries) => {
                        this.listings.insert(path, entries);
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_file_action_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let editor = self.file_action_editor.clone();
        let confirmation = self.file_confirmation.clone();
        let input_style = LocalInputStyle {
            background: self.theme.background,
            foreground: self.theme.foreground,
            muted_foreground: self.theme.muted_foreground,
            border: self.theme.border,
        };
        v_flex()
            .w_full()
            .gap_2()
            .p_2()
            .border_b_1()
            .border_color(self.theme.border)
            .bg(self.theme.muted)
            .when_some(editor, |this, editor| {
                let title = match editor.mode {
                    FileActionEditorMode::CreateFile { .. } => {
                        t!("WorkspaceExplorer.file_action.new_file")
                    }
                    FileActionEditorMode::CreateDirectory { .. } => {
                        t!("WorkspaceExplorer.file_action.new_directory")
                    }
                    FileActionEditorMode::Rename { .. } => {
                        t!("WorkspaceExplorer.file_action.rename")
                    }
                };
                this.child(
                    v_flex()
                        .gap_2()
                        .child(div().text_sm().font_semibold().child(title))
                        .child(
                            Input::new(&editor.input)
                                .w_full()
                                .small()
                                .local_style(input_style),
                        )
                        .child(self.render_file_action_buttons(false, cx)),
                )
            })
            .when_some(confirmation, |this, confirmation| {
                this.child(
                    v_flex()
                        .gap_2()
                        .child(
                            h_flex()
                                .items_start()
                                .gap_2()
                                .child(
                                    Icon::new(IconName::TriangleAlert)
                                        .with_size(Size::Small)
                                        .text_color(self.theme.warning),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .text_sm()
                                        .text_color(self.theme.foreground)
                                        .child(confirmation.message),
                                ),
                        )
                        .child(self.render_file_action_buttons(true, cx)),
                )
            })
            .into_any_element()
    }

    fn render_file_action_buttons(
        &self,
        confirmation: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .justify_end()
            .gap_1()
            .child(
                Button::new("workspace-file-action-cancel")
                    .label(t!("WorkspaceExplorer.action.cancel"))
                    .small()
                    .custom(self.theme.icon_button_style(cx))
                    .disabled(self.file_operation_running)
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_file_action(cx))),
            )
            .child(
                Button::new("workspace-file-action-submit")
                    .label(if confirmation {
                        t!("WorkspaceExplorer.action.confirm")
                    } else {
                        t!("WorkspaceExplorer.action.save")
                    })
                    .small()
                    .custom(if confirmation {
                        self.theme.danger_button_style(cx)
                    } else {
                        self.theme.button_style(cx)
                    })
                    .disabled(self.file_operation_running)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if confirmation {
                            this.submit_file_confirmation(window, cx);
                        } else {
                            this.submit_file_editor(window, cx);
                        }
                    })),
            )
    }
}

pub(super) fn build_files_context_menu(
    menu: PopupMenu,
    explorer: Entity<WorkspaceExplorer>,
    parent: PathBuf,
    _window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let directory_explorer = explorer.clone();
    let file_parent = parent.clone();
    let directory_parent = parent;
    let theme = explorer.read(cx).theme.menu_style();
    menu.local_style(theme)
        .min_w(px(190.0))
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.file_action.new_file").to_string())
                .icon(IconName::File)
                .on_click(move |_, window, cx| {
                    explorer.update(cx, |this, cx| {
                        this.prompt_create_file(file_parent.clone(), window, cx);
                    });
                }),
        )
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.file_action.new_directory").to_string())
                .icon(IconName::FolderClosed)
                .on_click(move |_, window, cx| {
                    directory_explorer.update(cx, |this, cx| {
                        this.prompt_create_directory(directory_parent.clone(), window, cx);
                    });
                }),
        )
}

pub(super) fn build_file_context_menu(
    menu: PopupMenu,
    explorer: Entity<WorkspaceExplorer>,
    path: PathBuf,
    is_dir: bool,
    _window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let parent = if is_dir {
        path.clone()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.clone())
    };
    let open_explorer = explorer.clone();
    let create_file_explorer = explorer.clone();
    let create_directory_explorer = explorer.clone();
    let set_root_explorer = explorer.clone();
    let rename_explorer = explorer.clone();
    let delete_explorer = explorer.clone();
    let open_path = path.clone();
    let create_file_parent = parent.clone();
    let create_directory_parent = parent;
    let set_root_path = path.clone();
    let rename_path = path.clone();
    let delete_path = path;
    let theme = explorer.read(cx).theme.menu_style();

    menu.local_style(theme)
        .min_w(px(200.0))
        .when(!is_dir, |menu| {
            menu.item(
                PopupMenuItem::new(t!("WorkspaceExplorer.file_action.open").to_string())
                    .icon(IconName::File)
                    .on_click(move |_, window, cx| {
                        open_explorer.update(cx, |this, cx| {
                            this.open_file(open_path.clone(), window, cx);
                        });
                    }),
            )
            .separator()
        })
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.file_action.new_file").to_string())
                .icon(IconName::File)
                .on_click(move |_, window, cx| {
                    create_file_explorer.update(cx, |this, cx| {
                        this.prompt_create_file(create_file_parent.clone(), window, cx);
                    });
                }),
        )
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.file_action.new_directory").to_string())
                .icon(IconName::FolderClosed)
                .on_click(move |_, window, cx| {
                    create_directory_explorer.update(cx, |this, cx| {
                        this.prompt_create_directory(create_directory_parent.clone(), window, cx);
                    });
                }),
        )
        .when(is_dir, |menu| {
            menu.separator().item(
                PopupMenuItem::new(
                    t!("WorkspaceExplorer.file_action.set_workspace_root").to_string(),
                )
                .icon(IconName::Pin)
                .on_click(move |_, _, cx| {
                    set_root_explorer.update(cx, |this, cx| {
                        this.set_root_manually(set_root_path.clone(), cx);
                    });
                }),
            )
        })
        .separator()
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.file_action.rename").to_string())
                .icon(IconName::Replace)
                .on_click(move |_, window, cx| {
                    rename_explorer.update(cx, |this, cx| {
                        this.prompt_rename(rename_path.clone(), window, cx);
                    });
                }),
        )
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.file_action.delete").to_string())
                .icon(IconName::Delete)
                .on_click(move |_, _, cx| {
                    delete_explorer.update(cx, |this, cx| {
                        this.confirm_delete(delete_path.clone(), cx);
                    });
                }),
        )
}

pub(super) fn build_git_change_context_menu(
    menu: PopupMenu,
    explorer: Entity<WorkspaceExplorer>,
    change: GitChange,
    _window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let open_explorer = explorer.clone();
    let state_explorer = explorer.clone();
    let discard_explorer = explorer.clone();
    let open_change = change.clone();
    let state_change = change.clone();
    let discard_change = change.clone();
    let staged = change.staged;
    let theme = explorer.read(cx).theme.menu_style();

    menu.local_style(theme)
        .min_w(px(210.0))
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.git_action.open_changes").to_string())
                .icon(IconName::Eye)
                .on_click(move |_, window, cx| {
                    open_explorer.update(cx, |this, cx| {
                        this.open_change(open_change.clone(), window, cx);
                    });
                }),
        )
        .item(
            PopupMenuItem::new(
                if staged {
                    t!("WorkspaceExplorer.git_action.unstage")
                } else {
                    t!("WorkspaceExplorer.git_action.stage")
                }
                .to_string(),
            )
            .icon(if staged {
                IconName::ArrowDown
            } else {
                IconName::ArrowUp
            })
            .on_click(move |_, window, cx| {
                state_explorer.update(cx, |this, cx| {
                    if staged {
                        this.unstage_git_change(state_change.clone(), window, cx);
                    } else {
                        this.stage_git_change(state_change.clone(), window, cx);
                    }
                });
            }),
        )
        .separator()
        .item(
            PopupMenuItem::new(t!("WorkspaceExplorer.git_action.discard").to_string())
                .icon(IconName::Undo2)
                .on_click(move |_, _, cx| {
                    discard_explorer.update(cx, |this, cx| {
                        this.confirm_discard_change(discard_change.clone(), cx);
                    });
                }),
        )
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
