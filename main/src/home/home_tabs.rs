use crate::credential_vault::CredentialVaultView;
use crate::home_tab::HomePage;
use crate::license::is_feature_enabled;

use crate::setting_tab::SettingsPanel;
#[cfg(feature = "shell")]
use domain_api::{TabIntent, TerminalProfileDescriptor};
use gpui::{App, AppContext, Context, Entity, Window};
#[cfg(feature = "shell")]
use gpui_component::{WindowExt, notification::Notification};
use json_view::JsonFormatterView;
use notes::NotesView;
use one_core::license::Feature;
#[cfg(feature = "shell")]
use one_core::storage::StoredConnection;
use one_core::tab_actions::next_duplicate_tab_index;
use one_core::tab_container::GlobalTabContainer;
#[cfg(feature = "shell")]
use one_core::tab_container::TabOpenMode;
use one_core::tab_container::{TabContainer, TabItem};

#[cfg(test)]
mod tests {

    #[test]
    fn notes_tab_uses_stable_identity() {
        let source = include_str!("home_tabs.rs").replace("\r\n", "\n");
        assert!(source.contains("fn add_notes_tab"));
        assert!(source.contains("activate_or_add_tab_lazy(\n                    \"notes\""));
        assert!(source.contains("TabItem::new(\"notes\", \"home\", notes)"));
    }

    #[test]
    fn both_type_filters_share_the_stateless_menu() {
        let menu = include_str!("../connection_type_menu.rs");
        assert!(menu.contains("ConnectionType::all()"));
        // 菜单项去图标、扩展按名称逐项列出（MQTT 与内置项合并去重）。
        assert!(!menu.contains("connection_type_rail_icon"));
        assert!(menu.contains("ConnectionFilter::Builtin"));
        assert!(menu.contains("extension_filter_targets"));
        // 菜单与筛选条共用同一份筛选项清单，避免两处顺序/命名分叉。
        assert!(menu.contains("filter_targets"));
        assert!(
            include_str!("../persistent_connection_sidebar/filter_bar.rs")
                .contains("build_filter_menu")
        );
        // 标题行的平铺筛选条与「更多」菜单复用同一无状态菜单构建器。
        let bar = include_str!("../home_tab/connection_type_filter_bar.rs");
        assert!(bar.contains("build_filter_menu_for"));
        assert!(bar.contains("filter_targets"));
        // 工具栏不再保留类型筛选下拉入口。
        assert!(!include_str!("../home_tab/toolbar.rs").contains("build_filter_menu"));
    }

    #[test]
    fn ai_workbench_opener_reuses_startup_pin_before_creating_regular_tab() {
        let source = include_str!("home_tabs.rs");
        let opener = source
            .rsplit("pub(crate) fn add_ai_workbench_tab")
            .next()
            .unwrap()
            .split("pub(crate) fn ")
            .next()
            .unwrap();
        assert!(opener.contains("activate_pinned_tab_by_id"));
        assert!(opener.contains("activate_or_add_tab_lazy"));
        let pinned = opener.find("activate_pinned_tab_by_id").unwrap();
        let regular = opener.find("activate_or_add_tab_lazy").unwrap();
        assert!(pinned < regular);
        // 「AI 工作台标签可关闭」这件事是在 `build_ai_workbench_shell` 里设置的
        // （opener 只是 `TabItem::new(..)` 引用它），所以断言要分开落两处：
        // 这里确认 opener 确实走那个构造函数，具体标志位去它自己的文件里查。
        assert!(opener.contains("build_ai_workbench_shell"));
        assert!(include_str!("ai_workbench.rs").contains(".with_tab_closeable(true)"));
    }
}

impl HomePage {
    fn active_tab_container(&self, cx: &App) -> Entity<TabContainer> {
        cx.try_global::<GlobalTabContainer>()
            .map(|global| global.primary_pane())
            .unwrap_or_else(|| self.tab_container.clone())
    }

    /// 计算同基础名称的下一个可用标签序号；没有任何同名标签时返回 None（首标签不加序号）。
    pub(crate) fn next_available_tab_index(&self, base_title: &str, cx: &App) -> Option<usize> {
        let tab_container = self.active_tab_container(cx);
        let titles: Vec<String> = tab_container
            .read(cx)
            .tabs()
            .iter()
            .map(|tab| tab.title(cx).to_string())
            .collect();
        next_duplicate_tab_index(base_title, titles.iter().map(String::as_str))
    }

    #[cfg(feature = "shell")]
    pub(crate) fn open_sftp_view(
        &mut self,
        conn: StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !app_kit::registry().open_sftp(&conn, self, window, cx) {
            notify_unsupported(&conn, window, cx);
        }
    }

    /// SFTP 双栏视图请求在指定目录打开本地终端。
    #[cfg(feature = "shell")]
    pub(crate) fn open_sftp_local_terminal(
        &mut self,
        working_dir: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(extras) = app_kit::registry().shell_extras() else {
            return;
        };
        if !extras.open_terminal_in_dir(self, Some(working_dir), None, window, cx) {
            window.push_notification(Notification::error("当前产品不支持本地终端"), cx);
        }
    }

    /// SFTP 双栏视图请求在指定目录打开该连接的 SSH 终端。
    #[cfg(feature = "shell")]
    pub(crate) fn open_sftp_ssh_terminal(
        &mut self,
        connection: StoredConnection,
        working_dir: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(extras) = app_kit::registry().shell_extras() else {
            return;
        };
        if !extras.open_terminal_in_dir(self, Some(working_dir), Some(connection), window, cx) {
            window.push_notification(Notification::error("当前产品不支持 SSH 终端"), cx);
        }
    }

    pub(crate) fn add_settings_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tc, cx| {
                tc.activate_or_add_tab_lazy(
                    "settings",
                    |win, cx| {
                        let settings = cx.new(|cx| SettingsPanel::new(win, cx));
                        TabItem::new("settings", "home", settings)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_sync_settings_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tc, cx| {
                tc.activate_or_add_tab_lazy(
                    "settings-sync",
                    |win, cx| {
                        let settings = cx.new(|cx| SettingsPanel::new_sync(win, cx));
                        TabItem::new("settings-sync", "home", settings)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    /// 打开原生「团队」tab（团队资料 / 成员 / 团队密钥）。
    ///
    /// 过去这里跳浏览器 dashboard，URL 里还带着 access_token / refresh_token；
    /// 现在全部走桌面端云端 API，网站 dashboard 可以下线。
    pub(crate) fn add_team_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !is_feature_enabled(Feature::TeamManagement, cx) {
            return;
        }

        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tc, cx| {
                tc.activate_or_add_tab_lazy(
                    "team",
                    |window, cx| {
                        let team_tab = cx.new(|cx| crate::team_tab::TeamTab::new(window, cx));
                        team_tab.update(cx, |team_tab, cx| team_tab.ensure_loaded(window, cx));
                        TabItem::new("team", "home", team_tab)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_extensions_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tc, cx| {
                tc.activate_or_add_tab_lazy(
                    "extensions",
                    |win, cx| {
                        let host = std::sync::Arc::new(extension_runtime::MainExtensionViewHost);
                        let extensions =
                            cx.new(|cx| extension_view::ExtensionManagerView::new(host, win, cx));
                        TabItem::new("extensions", "home", extensions)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_notes_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tabs, cx| {
                tabs.activate_or_add_tab_lazy(
                    "notes",
                    |window, cx| {
                        let notes = cx.new(|cx| NotesView::new(window, cx));
                        TabItem::new("notes", "home", notes)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_toolbox_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tabs = self.active_tab_container(cx);
        let home = cx.entity();
        window.defer(cx, move |window, cx| {
            tabs.update(cx, |tabs, cx| {
                tabs.activate_or_add_tab_lazy(
                    "toolbox",
                    |window, cx| {
                        let view =
                            cx.new(|cx| crate::toolbox_tab::ToolboxTab::new(home, window, cx));
                        TabItem::new("toolbox", "home", view)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_json_formatter_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tabs, cx| {
                tabs.activate_or_add_tab_lazy(
                    "json-formatter",
                    |window, cx| {
                        let view = cx.new(|cx| JsonFormatterView::new(window, cx));
                        TabItem::new("json-formatter", "home", view)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_credential_vault_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tabs, cx| {
                tabs.activate_or_add_tab_lazy(
                    "credential-vault",
                    |window, cx| {
                        let vault = cx.new(|cx| CredentialVaultView::new(window, cx));
                        TabItem::new("credential-vault", "home", vault)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_ai_workbench_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        let ai_access_levels = ai_chat_view::load_ai_access_levels(cx);
        let (scope, catalog, mentions) =
            ai_chat_view::build_workbench_resource_state(&self.connections, &ai_access_levels);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tabs, cx| {
                if tabs.activate_pinned_tab_by_id("ai-workbench", window, cx) {
                    return;
                }
                tabs.activate_or_add_tab_lazy(
                    "ai-workbench",
                    |window, cx| {
                        let workbench = crate::home::ai_workbench::build_ai_workbench_shell(
                            scope, catalog, mentions, window, cx,
                        );
                        TabItem::new("ai-workbench", "home", workbench)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    #[cfg(feature = "shell")]
    pub(crate) fn open_local_terminal_tab(
        &mut self,
        profile: Option<TerminalProfileDescriptor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let intent = TabIntent::Terminal { profile };
        if !app_kit::registry().open_tab(&intent, TabOpenMode::Activate, self, window, cx) {
            window.push_notification(Notification::error("当前产品不支持本地终端"), cx);
        }
    }

    /// 关闭当前活动标签（pinned tab 不支持通过快捷键关闭）
    pub(crate) fn close_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        let tc = tab_container.read(cx);

        if tc.is_pinned_tab_active() {
            return;
        }

        if tc.active_tab().is_none() {
            return;
        }
        let index = tc.active_index();

        tab_container.update(cx, |tc, cx| {
            tc.close_tab(index, window, cx).detach();
        });
    }

    pub(crate) fn duplicate_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        let (content_key, active_tab) = {
            let tc = tab_container.read(cx);
            // pinned tab 不支持复制
            if tc.is_pinned_tab_active() {
                return;
            }
            let Some(active_tab) = tc.active_tab() else {
                return;
            };
            (active_tab.content().content_key(cx), active_tab.clone())
        };
        let Some(extras) = app_kit::registry().shell_extras() else {
            return;
        };
        // 领域标签（例如终端）由领域自己决定怎么复制；其它类型暂不支持复制。
        if content_key == "Terminal" {
            let _ = extras.duplicate_terminal_tab(&active_tab, self, window, cx);
        }
    }
}

#[cfg(feature = "shell")]

/// 当前产品没有注册该连接类型时给出明确提示，而不是静默失败。
fn notify_unsupported(conn: &StoredConnection, window: &mut Window, cx: &mut gpui::App) {
    window.push_notification(
        Notification::error(format!(
            "当前产品不支持打开该连接：{}（{}）",
            conn.name, conn.connection_type
        )),
        cx,
    );
}
