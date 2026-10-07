//! AI 工作台外壳的宿主组装。
//!
//! 外壳本身在 `ai_chat_view::workbench`，这里把四个面板接上：
//! 会话（内置 agent）、审阅（工作区编辑器）、文件（工作区浏览器）、终端。
//! 会话列表由外壳直接读会话面板，所以内建侧栏会被压制。

use ai_chat_view::{
    DefaultAgentChatPanel, DefaultAgentChatPanelEvent, MentionItem, SubagentDetailPanel,
    WorkbenchPanelEntry, WorkbenchPanelKind, WorkbenchShell, WorkbenchShellConfig, WorkbenchState,
};
use gpui::{App, AppContext as _, Entity, Subscription, Window};
use gpui_component::ActiveTheme as _;
use terminal::LocalConfig;
use terminal_view::TerminalView;
use workspace_explorer::{
    WorkspaceEditor, WorkspaceExplorer, WorkspaceExplorerConfig, WorkspaceExplorerEvent,
    WorkspaceTheme,
};

use super::ai_workbench_composer::{composer_context_source, refresh_composer_git};

/// 工作区主题：面板背景、边框与强调色取应用主题，语义色同样取应用主题。
pub(crate) fn workspace_theme(cx: &App) -> WorkspaceTheme {
    let theme = cx.theme();
    WorkspaceTheme {
        background: theme.background,
        foreground: theme.foreground,
        muted: theme.muted,
        muted_foreground: theme.muted_foreground,
        border: theme.border,
        accent: theme.accent,
        accent_foreground: theme.accent_foreground,
        selection: theme.selection,
        caret: theme.caret,
        danger: theme.danger,
        warning: theme.warning,
        success: theme.success,
    }
}

/// 工作台初始工作区。
///
/// 顺序：上次保存的工作区 → 进程当前目录 → 用户主目录。绝不回退到 `/`：
/// 那会得到一个不是工作区的"工作区"，用户也无从判断当前上下文。
fn workspace_root(cx: &App) -> std::path::PathBuf {
    let saved = one_core::settings::AppSettings::current(cx)
        .ai_chat
        .last_workspace_root;
    saved
        .filter(|path| path.is_dir())
        .or_else(|| std::env::current_dir().ok().filter(|path| path.is_dir()))
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

pub(super) fn recent_workspace_roots(cx: &App) -> Vec<std::path::PathBuf> {
    one_core::settings::AppSettings::current(cx)
        .ai_chat
        .recent_workspace_roots
        .into_iter()
        .filter(|path| path.is_dir())
        .collect()
}

/// 把当前工作区记入最近列表；由 Explorer 的根目录变化驱动。
fn remember_workspace_root(root: &std::path::Path, cx: &mut App) {
    one_core::settings::AppSettings::update_and_save(cx, |settings| {
        settings.ai_chat.remember_workspace_root(root);
    });
}

fn default_terminal_config(root: &std::path::Path) -> LocalConfig {
    LocalConfig {
        shell: None,
        args: Vec::new(),
        working_dir: Some(root.to_string_lossy().into_owned()),
        env: Vec::new(),
    }
}

/// 构建 AI 工作台标签页内容。
pub(crate) fn build_ai_workbench_shell(
    scope: agent_runtime::AgentResourceScope,
    catalog: agent_runtime::ResourceCatalog,
    mentions: Vec<MentionItem>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<WorkbenchShell> {
    let theme = workspace_theme(cx);
    let workspace_root = workspace_root(cx);
    // 工作区文件进 `@` 菜单：连接提及（调用方传入）之后追加，有界收集不拖慢输入框。
    let mut mentions = mentions;
    mentions.extend(ai_chat_view::workspace_files::collect_workspace_files_default(
        &workspace_root,
    ));
    let editor = cx.new(|_| WorkspaceEditor::new(theme));
    let explorer = cx.new(|cx| {
        WorkspaceExplorer::new(
            WorkspaceExplorerConfig {
                root: workspace_root.clone(),
                editor: editor.clone(),
                theme,
                show_frame_controls: false,
                backend: None,
            },
            cx,
        )
    });
    let chat = cx.new(|cx| {
        DefaultAgentChatPanel::new_workbench_with_scope_and_catalog(scope, catalog, mentions, window, cx)
            .with_tab_closeable(true)
            .with_workspace_root(workspace_root.clone())
    });
    let recents = recent_workspace_roots(cx);
    explorer.update(cx, |explorer, cx| explorer.set_recent_roots(recents, cx));
    // 初始工作区也要进入最近列表，否则第一次打开时菜单里是空的。
    remember_workspace_root(&workspace_root, cx);
    chat.update(cx, |panel, cx| panel.set_sidebar_suppressed(true, cx));
    let terminal_root = workspace_root.clone();
    let terminal = cx.new(|cx| {
        TerminalView::new(default_terminal_config(&terminal_root), window, cx).with_workspace_pane()
    });
    // 子代理详情面板：内容由「点开某张子代理卡片」决定。
    //
    // 面板自己订阅聊天面板的事件、自己认领要看哪条子代理；宿主只负责在请求到来时
    // 把它切到前台（见下面的 `SubagentDetailRequested` 分支）。这样面板的目标状态
    // 只有一个副本，不会出现「宿主认为是 A、面板显示 B」。
    let subagent_detail = cx.new(|cx| SubagentDetailPanel::new(chat.clone(), cx));

    let shell = cx.new(|cx| {
        WorkbenchShell::new(
            WorkbenchShellConfig {
                panels: vec![
                    WorkbenchPanelEntry::new(WorkbenchPanelKind::Chat, chat.clone()),
                    WorkbenchPanelEntry::new(WorkbenchPanelKind::Review, editor.clone()),
                    WorkbenchPanelEntry::new(WorkbenchPanelKind::Files, explorer.clone()),
                    WorkbenchPanelEntry::new(WorkbenchPanelKind::Terminal, terminal.clone()),
                    WorkbenchPanelEntry::new(WorkbenchPanelKind::Subagent, subagent_detail.clone()),
                ],
                session_nav: None,
                session_source: Some(chat.clone()),
                // 布局从持久化设置还原；坏字段/空配置由 from_settings 容错。
                initial_state: WorkbenchState::from_settings(
                    &one_core::settings::AppSettings::current(cx)
                        .ai_chat
                        .workbench_layout,
                ),
                theme: None,
                subscriptions: Vec::new(),
                workspace_root: Some(workspace_root.clone()),
            },
            window,
            cx,
        )
    });

    // 外壳先于订阅存在：这里在构造之后再接线，根目录变化同时刷新外壳标题与聊天面板。
    // 另一条分支：Explorer 里点开文件/变更 → 审阅面板就地切到前台。
    let chat_for_root = chat.clone();
    let shell_for_root = shell.clone();
    let root_subscription: Subscription = cx.subscribe(
        &explorer,
        move |_, event: &WorkspaceExplorerEvent, cx| {
            match event {
                WorkspaceExplorerEvent::RootChanged(root) => {
                    remember_workspace_root(root, cx);
                    chat_for_root.update(cx, |panel, cx| {
                        panel.set_workspace_root(root.clone(), cx);
                    });
                    let root = root.clone();
                    shell_for_root.update(cx, |shell, cx| shell.set_workspace_root(root, cx));
                    // 根变了 → 分支 / worktree 缓存整体失效，底栏取一次新的。
                    refresh_composer_git(cx);
                    shell_for_root.update(cx, |shell, cx| shell.refresh_composer_context(cx));
                }
                // Explorer 的仓库句柄变了（首次认出仓库 / 换仓库）：底栏依赖它的
                // 分支与 worktree 入口要跟着重算，否则会一直停在装机那一帧的空状态。
                WorkspaceExplorerEvent::RepositoryChanged => {
                    refresh_composer_git(cx);
                    shell_for_root.update(cx, |shell, cx| shell.refresh_composer_context(cx));
                }
                WorkspaceExplorerEvent::DocumentRequested => {
                    shell_for_root.update(cx, |shell, cx| {
                        shell.reveal_panel(WorkbenchPanelKind::Review, cx);
                    });
                }
                // 快照锚定成功 / 回滚截断后，Explorer 会送上该会话最新的可回滚轮次；
                // 转手灌进聊天面板，它据此决定轮次页脚要不要显示「回到这一轮」。
                WorkspaceExplorerEvent::RestorableTurnsChanged {
                    session_id,
                    turn_ids,
                } => {
                    let turn_ids = turn_ids.iter().cloned().collect();
                    chat_for_root.update(cx, |panel, cx| {
                        panel.set_restorable_turns(session_id.clone(), turn_ids, cx);
                    });
                }
                _ => {}
            }
        },
    );
    let explorer_for_turns = explorer.clone();
    let shell_for_open = shell.clone();
    let explorer_for_files = explorer.clone();
    let turn_subscription: Subscription = cx.subscribe(
        &chat,
        move |_, event: &DefaultAgentChatPanelEvent, cx| {
            match event {
                DefaultAgentChatPanelEvent::TurnFinished {
                    session_id,
                    turn_id,
                    success,
                } => {
                    explorer_for_turns.update(cx, |explorer, cx| {
                        explorer.capture_turn_finished(
                            session_id.clone(),
                            turn_id.clone(),
                            *success,
                            cx,
                        );
                    });
                }
                // 用户在某一轮的页脚点了「回到这一轮」：把工作区退回那一刻的快照。
                DefaultAgentChatPanelEvent::RestoreTurn {
                    session_id,
                    turn_id,
                } => {
                    explorer_for_turns.update(cx, |explorer, cx| {
                        explorer.restore_turn(session_id.clone(), turn_id.clone(), cx);
                    });
                }
                // 用户点了改动摘要里的某个文件：先把审阅面板切到前台，再让编辑器
                // 打开它（`open_file` 自己也会广播 `DocumentRequested`）。
                //
                // 用户点了子代理卡片的「查看推理过程」：把详情面板切到前台。
                // 面板内容由它自己按事件里的子会话 id 现取，这里不搬数据。
                DefaultAgentChatPanelEvent::SubagentDetailRequested { .. } => {
                    shell_for_open.update(cx, |shell, cx| {
                        shell.reveal_panel(WorkbenchPanelKind::Subagent, cx);
                    });
                }
                // 打开文件需要窗口，而这里只有 `App`：推迟到下一帧再取窗口，
                // 避免在当前窗口的更新过程中重入。
                DefaultAgentChatPanelEvent::OpenFileInReview { path } => {
                    shell_for_open.update(cx, |shell, cx| {
                        shell.reveal_panel(WorkbenchPanelKind::Review, cx);
                    });
                    let path = std::path::PathBuf::from(path);
                    let explorer = explorer_for_files.clone();
                    cx.defer(move |cx| {
                        let Some(window) = crate::app_init::resolve_navop_window(cx) else {
                            tracing::warn!(path = %path.display(), "no window for review open");
                            return;
                        };
                        let _ = window.update(cx, |_, window, cx| {
                            explorer.update(cx, |explorer, cx| explorer.open_file(path, window, cx));
                        });
                    });
                }
                _ => {}
            }
        },
    );
    shell.update(cx, |shell, cx| shell.add_subscription(turn_subscription, cx));
    shell.update(cx, |shell, cx| shell.add_subscription(root_subscription, cx));

    // 顶部工作区标签点击 → 打开目录选择器（复用 Explorer 的最近列表与广播）。
    let explorer_for_picker = explorer.clone();
    shell.update(cx, |shell, cx| {
        shell.set_workspace_picker(
            move |window: &mut gpui::Window, cx: &mut gpui::App| {
                explorer_for_picker.update(cx, |explorer, cx| {
                    explorer.choose_root(window, cx);
                });
            },
            cx,
        );
    });

    // 侧栏跨工作区操作（分组 hover 新建 / 底部下拉新建 / 跨工作区选择会话）
    // → 把 explorer 切到指定根，RootChanged 级联回外壳与聊天面板。
    let explorer_for_switch = explorer.clone();
    shell.update(cx, |shell, cx| {
        shell.set_workspace_switcher(
            move |root: &std::path::Path, cx: &mut gpui::App| {
                explorer_for_switch.update(cx, |explorer, cx| {
                    explorer.set_root_manually(root.to_path_buf(), cx);
                });
            },
            cx,
        );
    });

    // 输入框下方上下文栏（工作区 / 分支 / Worktree）：数据与动作都在宿主侧，
    // 视图只做客后转发。必须在 shell 建好之后接线——动作闭包自持它的弱引用。
    let composer_source = composer_context_source(shell.downgrade(), explorer.clone(), cx);
    shell.update(cx, |shell, cx| {
        shell.set_composer_context_source(composer_source, cx)
    });

    // 多例面板工厂：「新建页签」选终端时创建一个全新终端实例（工作目录取
    // 外壳当前工作区根），而不是定位到已有终端。审查/文件是单例面板，
    // 重复打开只会定位，不走这里。
    shell.update(cx, |shell, cx| {
        shell.set_panel_factory(
            move |root: &std::path::Path, window: &mut gpui::Window, cx: &mut gpui::App| {
                let config = default_terminal_config(root);
                cx.new(|cx| TerminalView::new(config, window, cx).with_workspace_pane())
                    .into()
            },
            cx,
        );
    });

    // AI 提交信息：Explorer 发请求，这里取已配置 provider 生成后回填。
    let explorer_for_message = explorer.clone();
    let commit_message_subscription: Subscription = cx.subscribe(
        &explorer,
        move |_, _: &WorkspaceExplorerEvent, cx| {
            spawn_commit_message_generation(&explorer_for_message, cx);
        },
    );
    shell.update(cx, |shell, cx| {
        shell.add_subscription(commit_message_subscription, cx)
    });
    shell
}

/// 用已配置的默认 provider 生成提交信息并回填输入框。
fn spawn_commit_message_generation(
    explorer: &gpui::Entity<WorkspaceExplorer>,
    cx: &mut gpui::App,
) {
    use one_core::llm::storage::ProviderRepository;
    use one_core::llm::{LlmConnector, LlmProvider};
    use one_core::storage::traits::Repository;
    use one_core::storage::GlobalStorageState;

    let Some(storage_state) = cx.try_global::<GlobalStorageState>() else {
        return;
    };
    let Some(repo) = storage_state.storage.get::<ProviderRepository>() else {
        return;
    };
    let Some(config) = repo
        .list()
        .unwrap_or_default()
        .into_iter()
        .find(|config| config.enabled && config.is_default)
    else {
        return;
    };
    let Ok(provider) = LlmConnector::from_config(&config) else {
        return;
    };
    let model = config.model.clone();
    let window_handle = cx.active_window();
    // LLM 客户端（reqwest）的超时实现依赖 tokio reactor：必须把调用放到
    // 应用持有的 Tokio runtime 上执行，GPUI 前台 Future 里直接 await 会
    // panic（there is no reactor running）。
    let tokio_handle = one_core::gpui_tokio::Tokio::handle(cx);
    let explorer = explorer.clone();
    cx.spawn(async move |cx| {
        let fail = |cx: &mut gpui::AsyncApp| {
            let _ = explorer.update(cx, |explorer, cx| {
                explorer.commit_message_generation_failed(cx);
            });
        };
        let repository = explorer.update(cx, |explorer, _| explorer.repository().cloned());
        let Some(repository) = repository else {
            fail(cx);
            return;
        };
        // diff 摘要是 git 只读操作，放后台线程；LLM 调用走 Tokio runtime。
        let context = match cx
            .background_spawn(async move {
                workspace_explorer::commit_context(&repository, 16 * 1024)
            })
            .await
        {
            Ok(context) => context,
            Err(error) => {
                tracing::warn!(%error, "Failed to collect commit context");
                fail(cx);
                return;
            }
        };
        let request = one_core::llm::ChatRequest {
            model,
            messages: vec![one_core::llm::Message::user(format!(
                "Write a concise conventional commit message (one line, imperative mood, \
                 no scope prefix, under 72 characters) for these changes. \
                 Reply with the message only.\n\n{context}"
            ))],
            temperature: Some(0.2),
            max_tokens: Some(100),
            ..Default::default()
        };
        let generated = match tokio_handle
            .spawn(async move { provider.chat(&request).await })
            .await
        {
            Ok(result) => result,
            Err(join_error) => {
                tracing::warn!(%join_error, "Commit message generation task aborted");
                fail(cx);
                return;
            }
        };
        let Some(window_handle) = window_handle else {
            return;
        };
        let _ = cx.update_window(window_handle, |_, window, cx| {
            explorer.update(cx, |explorer, cx| match generated {
                Ok(message) => {
                    let message = message.trim().trim_matches('`').to_string();
                    explorer.set_commit_message(message, window, cx);
                }
                Err(error) => {
                    tracing::warn!(%error, "Commit message generation failed");
                    explorer.commit_message_generation_failed(cx);
                }
            });
        });
    })
    .detach();
}
