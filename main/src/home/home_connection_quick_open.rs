use crate::connection_visuals::{ConnectionVisualSize, stored_connection_icon_with_catalog};
use crate::home_tab::{HomePage, connection_matches_query};
use db::ipc::IpcDriverRegistry;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, Context, Entity, FontWeight, ParentElement, SharedString, Styled, Task, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IndexPath, Sizable, WindowExt, h_flex,
    list::{ListDelegate, ListItem, ListState},
};
use one_assets::IconName;
use one_core::storage::{SshAuthMethod, SshParams, StoredConnection};
use one_ui::{IconButton, IconButtonRole, IconSize};
use rust_i18n::t;

pub(crate) struct ConnectionQuickOpenDelegate {
    parent: Entity<HomePage>,
    external_driver_registry: IpcDriverRegistry,
    items: Vec<StoredConnection>,
    filtered_items: Vec<StoredConnection>,
    selected_index: Option<IndexPath>,
    search_query: String,
}

impl ConnectionQuickOpenDelegate {
    pub(crate) fn new(
        parent: Entity<HomePage>,
        external_driver_registry: IpcDriverRegistry,
    ) -> Self {
        Self {
            parent,
            external_driver_registry,
            items: Vec::new(),
            filtered_items: Vec::new(),
            selected_index: None,
            search_query: String::new(),
        }
    }

    pub(crate) fn update_items(&mut self, connections: &[StoredConnection]) {
        self.items = connections.to_vec();
        self.apply_filter();
    }

    fn apply_filter(&mut self) {
        if let Some(connection) = temporary_ssh_connection(&self.search_query) {
            self.filtered_items = vec![connection];
            self.selected_index = Some(IndexPath::default());
            return;
        }
        if self.search_query.is_empty() {
            self.filtered_items = self.items.clone();
            return;
        }
        let query = self.search_query.to_lowercase();
        self.filtered_items = self
            .items
            .iter()
            .filter(|conn| quick_open_matches_connection(conn, &query))
            .cloned()
            .collect();
    }
}

fn quick_open_matches_connection(connection: &StoredConnection, query: &str) -> bool {
    connection_matches_query(connection, query)
        || connection
            .connection_type
            .label()
            .to_lowercase()
            .contains(query)
}

fn parse_temporary_ssh_command(input: &str) -> Option<(String, String, u16)> {
    let mut args = input.split_whitespace();
    if args.next()? != "ssh" {
        return None;
    }
    let mut destination = None;
    let mut explicit_port = None;
    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("-p") {
            let value = if value.is_empty() {
                args.next()?
            } else {
                value
            };
            if !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let port = value.parse::<u16>().ok().filter(|port| *port > 0)?;
            explicit_port.get_or_insert(port);
        } else if arg.starts_with('-') || destination.replace(arg).is_some() {
            return None;
        }
    }
    let destination = destination?;
    let (username, address) = destination.split_once('@').unwrap_or(("", destination));
    let (host, port) = match address.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => (host, port.parse().ok()?),
        _ => (address, 22),
    };
    let port = explicit_port.unwrap_or(port);
    (!host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_'))
        && !host.starts_with('-')
        && !username.contains('@')
        && port > 0)
        .then(|| (username.to_string(), host.to_string(), port))
}

pub(crate) fn temporary_ssh_connection(input: &str) -> Option<StoredConnection> {
    let (username, host, port) = parse_temporary_ssh_command(input)?;
    let name = format!(
        "SSH {}{host} (temporary)",
        if username.is_empty() {
            String::new()
        } else {
            format!("{username}@")
        }
    );
    let params = SshParams {
        remote_file: None,
        host,
        port,
        prompt_username: username.is_empty().then_some(true),
        username,
        auth_method: SshAuthMethod::Password {
            password: String::new(),
        },
        prompt_password: Some(true),
        sftp_account: None,
        sftp_default_directory: None,
        credential_reference: None,
        keyboard_interactive: None,
        terminal_encoding: Default::default(),
        terminal_type: Default::default(),
        account_expect: Default::default(),
        connect_timeout: None,
        keepalive_interval: None,
        keepalive_max: None,
        default_directory: None,
        init_script: None,
        disable_shell_integration: None,
        x11_forwarding: None,
        allow_legacy_algorithms: None,
        forward_agent: None,
        agent_forward_key: None,
        jump_server: None,
        disabled_jump_server: None,
        proxy: None,
        os_id: None,
        icon: None,
        icon_file_path: None,
    };
    Some(StoredConnection::new_ssh(name, params, None))
}

#[cfg(test)]
mod temporary_ssh_tests {
    use super::parse_temporary_ssh_command;

    #[test]
    fn parses_temporary_ssh_targets_without_persisting() {
        assert!(
            super::temporary_ssh_connection("ssh example.com")
                .unwrap()
                .id
                .is_none()
        );
        assert!(parse_temporary_ssh_command("ssh host:invalid").is_none());
        assert!(parse_temporary_ssh_command("ssh host:0").is_none());
        assert!(parse_temporary_ssh_command("ssh host;whoami").is_none());
        assert_eq!(
            Some(("alice".into(), "example.com".into(), 22)),
            parse_temporary_ssh_command("ssh alice@example.com")
        );
        assert_eq!(
            Some(("".into(), "127.0.0.1".into(), 2222)),
            parse_temporary_ssh_command("ssh 127.0.0.1:2222")
        );
        for command in [
            "ssh -p 2222 alice@example.com",
            "ssh alice@example.com -p 2222",
            "ssh -p2222 alice@example.com",
            "ssh alice@example.com -p2222",
        ] {
            assert_eq!(
                Some(("alice".into(), "example.com".into(), 2222)),
                parse_temporary_ssh_command(command),
                "{command}"
            );
            let connection = super::temporary_ssh_connection(command).unwrap();
            assert!(connection.id.is_none());
            assert_eq!(2222, connection.to_ssh_params().unwrap().port);
        }
        for command in [
            "ssh -p host",
            "ssh host -p",
            "ssh -p 0 host",
            "ssh -p 65536 host",
            "ssh -p invalid host",
            "ssh -p 22",
            "ssh -i key host",
            "ssh host extra",
        ] {
            assert!(parse_temporary_ssh_command(command).is_none(), "{command}");
        }
    }
}

impl ListDelegate for ConnectionQuickOpenDelegate {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.search_query = query.to_string();
        self.apply_filter();
        cx.notify();
        Task::ready(())
    }

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.filtered_items.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let connection = self.filtered_items.get(ix.row)?.clone();
        let name = connection.name.clone();
        let icon = {
            let extension_catalog = crate::connection_visuals::extension_catalog_from_cx(cx);
            stored_connection_icon_with_catalog(
                &connection,
                ConnectionVisualSize::Tree,
                &self.external_driver_registry,
                extension_catalog.as_deref(),
            )
        };
        let save_parent = self.parent.clone();
        // 临时连接（无数据库 ID）提供“保存为连接”入口。
        let temporary_connection = connection.id.is_none().then(|| connection.clone());

        // 打开连接只走 `confirm` 这一条路径：列表内建的点击处理会先
        // `set_selected_index` 再调用 `confirm`，键盘 Enter 走的也是同一条路径。
        // 这里不要再给行挂 `on_click` 打开连接，否则一次点击会先后触发行内
        // 回调与列表 confirm，一次点击创建两个会话（issue #461）。
        Some(
            ListItem::new(ix)
                .mx_2()
                .h(px(44.0))
                .px_3()
                .rounded(px(6.0))
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_3()
                        .child(div().flex_shrink_0().flex().items_center().child(icon))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(SharedString::from(name)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(SharedString::from(
                                    crate::connection_visuals::connection_type_display_label(
                                        &connection,
                                        cx,
                                    ),
                                )),
                        )
                        .when_some(temporary_connection, |this, connection| {
                            this.child(
                                IconButton::new(
                                    SharedString::from("quick-open-save-connection"),
                                    Icon::new(IconName::Save).mono().with_size(IconSize::Small),
                                )
                                .role(IconButtonRole::Compact)
                                .tooltip(t!("Home.save_as_connection"))
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    let connection = connection.clone();
                                    save_parent.update(cx, |this, cx| {
                                        this.show_save_temporary_connection_form(
                                            connection, window, cx,
                                        );
                                    });
                                    window.close_dialog(cx);
                                }),
                            )
                        }),
                ),
        )
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = ix;
    }

    fn confirm(
        &mut self,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        if let Some(ix) = self.selected_index {
            if let Some(connection) = self.filtered_items.get(ix.row).cloned() {
                let parent = self.parent.clone();
                parent.update(cx, |this, cx| {
                    this.open_connection_from_quick(&connection, window, cx);
                });
                window.close_dialog(cx);
            }
        }
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        window.close_dialog(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::{
        DatabaseType, DbConnectionConfig, RemoteDesktopParams, RemoteDesktopProtocol,
    };

    struct EscDelegate;

    impl ListDelegate for EscDelegate {
        type Item = ListItem;

        fn items_count(&self, _section: usize, _cx: &App) -> usize {
            0
        }

        fn render_item(
            &mut self,
            _ix: IndexPath,
            _window: &mut Window,
            _cx: &mut Context<ListState<Self>>,
        ) -> Option<Self::Item> {
            None
        }

        fn set_selected_index(
            &mut self,
            _ix: Option<IndexPath>,
            _window: &mut Window,
            _cx: &mut Context<ListState<Self>>,
        ) {
        }

        fn cancel(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
            window.close_dialog(cx);
        }
    }

    struct FocusHost {
        focus: gpui::FocusHandle,
    }

    impl gpui::Render for FocusHost {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl gpui::IntoElement {
            use gpui::InteractiveElement as _;
            div().size_full().track_focus(&self.focus)
        }
    }

    #[gpui::test]
    fn escape_closes_quick_open_dialog(cx: &mut gpui::TestAppContext) {
        use gpui::{AppContext as _, InteractiveElement as _};
        use gpui_component::{Root, Theme, list::List};

        cx.update(|cx| {
            cx.set_global(Theme::default());
            gpui_component::init(cx);
            notes::init(cx);
        });
        let (_host, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| FocusHost {
                focus: cx.focus_handle(),
            });
            let focus = view.read(cx).focus.clone();
            focus.focus(window, cx);
            Root::new(view, window, cx)
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));

        cx.update(|window, cx| {
            let list = cx.new(|cx| ListState::new(EscDelegate, window, cx).searchable(true));
            let list_for_dialog = list.clone();
            window.open_dialog(cx, move |dialog, _window, _cx| {
                let list = list_for_dialog.clone();
                dialog
                    .title("open")
                    .w(px(640.0))
                    .close_button(false)
                    .content(move |content, _window, _cx| {
                        content.p_0().child(div().id("quick-open").child(List::new(&list)))
                    })
            });
            list.update(cx, |state, cx| state.focus(window, cx));
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| {
            assert!(window.has_active_dialog(cx));
        });

        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(!window.has_active_dialog(cx), "Esc must close the quick open dialog");
        });
    }

    #[test]
    fn quick_open_matches_database_connection_by_ip() {
        let connection = StoredConnection::new_database(
            "Production".to_string(),
            DbConnectionConfig {
                id: String::new(),
                database_type: DatabaseType::MySQL,
                name: "Production".to_string(),
                host: "192.168.10.42".to_string(),
                port: 3306,
                username: "root".to_string(),
                password: String::new(),
                credential_reference: None,
                database: Some("app".to_string()),
                service_name: None,
                sid: None,
                workspace_id: None,
                proxy: None,
                extra_params: std::collections::HashMap::new(),
            },
            None,
        );

        assert!(quick_open_matches_connection(&connection, "192.168.10.42"));
        assert!(quick_open_matches_connection(&connection, "168.10"));
        assert!(!quick_open_matches_connection(&connection, "10.0.0.1"));
    }

    #[test]
    fn quick_open_matches_remote_desktop_connections_by_ip() {
        let rdp = remote_desktop_connection(
            RemoteDesktopProtocol::Rdp,
            "rdp-production",
            "10.0.0.8",
            Some("administrator"),
        );
        let vnc = remote_desktop_connection(
            RemoteDesktopProtocol::Vnc,
            "vnc-production",
            "10.0.0.9",
            None,
        );

        assert!(quick_open_matches_connection(&rdp, "10.0.0.8"));
        assert!(quick_open_matches_connection(
            &rdp,
            "administrator@10.0.0.8"
        ));
        assert!(quick_open_matches_connection(&vnc, "10.0.0.9"));
        assert!(quick_open_matches_connection(&vnc, "10.0.0.9:5900"));
    }

    fn remote_desktop_connection(
        protocol: RemoteDesktopProtocol,
        name: &str,
        host: &str,
        username: Option<&str>,
    ) -> StoredConnection {
        StoredConnection::new_remote_desktop(
            name.to_string(),
            RemoteDesktopParams {
                protocol,
                host: host.to_string(),
                port: protocol.default_port(),
                username: username.map(str::to_string),
                password: None,
                credential_reference: None,
                domain: None,
                read_only: false,
                audio_playback: false,
                proxy: None,
                backend_preference: Default::default(),
                rdp: None,
            },
            None,
        )
    }

    /// 回归保护（issue #461）：快速打开对话框里点一行只能新建一条会话。
    /// 列表内建的点击处理会调用 `ListState` 的 confirm，若行内再挂一层打开回调，
    /// 同一次点击就会打开两次连接，因此行渲染路径不得直接打开连接。
    #[test]
    fn quick_open_rows_open_connections_only_through_confirm() {
        let source = include_str!("home_connection_quick_open.rs");
        let render_item = source
            .split("fn render_item(")
            .nth(1)
            .expect("quick open delegate renders items")
            .split("fn set_selected_index(")
            .next()
            .expect("render_item has an end marker");
        let confirm = source
            .split("fn confirm(")
            .nth(1)
            .expect("quick open delegate confirms the selected item")
            .split("fn cancel(")
            .next()
            .expect("confirm has an end marker");

        assert!(
            !render_item.contains("open_connection_from_quick"),
            "行渲染不得自己打开连接，否则一次点击会先后触发行回调和列表 confirm（issue #461）"
        );
        assert!(
            confirm.contains("open_connection_from_quick"),
            "点击与回车都应通过 confirm 打开连接"
        );
    }
}
