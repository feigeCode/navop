//! 独立产品补齐的连接打开器：FTP 双栏文件视图、串口与 Telnet 终端。
//!
//! 这三类连接原先只有主产品 `main` 里存在打开逻辑，领域注册表里没有对应
//! opener，独立产品里点「打开」会失败。

use domain_api::{ConnectionOpener, HomeContext};
use gpui::{App, AppContext as _, Window};
use one_core::storage::{ConnectionType, StoredConnection};
use one_core::tab_container::{TabItem, TabOpenMode};
use sftp_view::{SftpView, SftpViewEvent};
use terminal_view::TerminalWorkspace;

/// 串口 / Telnet 终端的类型差异（tab id 前缀与 TabItem kind）。
#[derive(Clone, Copy)]
enum SerialLikeKind {
    Serial,
    Telnet,
}

impl SerialLikeKind {
    fn tab_kind(self) -> &'static str {
        match self {
            Self::Serial => "serial",
            Self::Telnet => "telnet",
        }
    }
}

/// 打开 Tab 需要的三个可变句柄；收拢成结构体避免辅助函数参数过多。
pub(crate) struct TabEnvironment<'a> {
    pub(crate) mode: TabOpenMode,
    pub(crate) home: &'a mut dyn HomeContext,
    pub(crate) window: &'a mut Window,
    pub(crate) cx: &'a mut App,
}

impl TabEnvironment<'_> {
    /// 打开双栏 SFTP 视图；FTP 与 SSH/SFTP 共用同一实现。
    pub(crate) fn open_sftp_view(self, connection: &StoredConnection) -> bool {
        let Self {
            home, window, cx, ..
        } = self;
        let connection_id = connection.id.unwrap_or(0);
        let tab_id = format!("sftp-{connection_id}-{}", current_millis());
        let tab_index = home.next_tab_index(&connection.name, cx);
        let sftp = cx.new(|cx| SftpView::new_with_index(connection.clone(), tab_index, window, cx));
        let window_handle = window.window_handle();
        let subscription = cx.subscribe(&sftp, move |_, event: &SftpViewEvent, cx| {
            let Some(host) = app_kit::shell_host() else {
                return;
            };
            match event {
                SftpViewEvent::OpenLocalTerminal { working_dir } => {
                    let working_dir = working_dir.clone();
                    let _ = window_handle.update(cx, move |_, window, cx| {
                        host.open_local_terminal(working_dir, window, cx);
                    });
                }
                SftpViewEvent::OpenSshTerminal {
                    connection,
                    working_dir,
                } => {
                    let connection = connection.clone();
                    let working_dir = working_dir.clone();
                    let _ = window_handle.update(cx, move |_, window, cx| {
                        host.open_ssh_terminal(connection, working_dir, window, cx);
                    });
                }
            }
        });
        home.keep_subscription(subscription);
        let item = TabItem::new(tab_id, "sftp", sftp);
        home.open_tab_with_focus(item, window, cx)
    }

    /// 打开串口 / Telnet 终端标签。
    fn open_serial_like(self, kind: SerialLikeKind, connection: StoredConnection) -> bool {
        let Self {
            mode,
            home,
            window,
            cx,
        } = self;
        let connection_id = connection.id.unwrap_or(0);
        let tab_id = format!(
            "{}-terminal-{connection_id}-{}",
            kind.tab_kind(),
            current_millis()
        );
        let tab_index = home.next_tab_index(&connection.name, cx);
        let terminal = match kind {
            SerialLikeKind::Serial => cx.new(|cx| {
                TerminalWorkspace::new_serial_with_index(connection, tab_index, window, cx)
            }),
            SerialLikeKind::Telnet => cx.new(|cx| {
                TerminalWorkspace::new_telnet_with_index(connection, tab_index, window, cx)
            }),
        };
        let item = TabItem::new(tab_id, kind.tab_kind(), terminal);
        home.open_tab(item, mode, window, cx)
    }
}

/// 当前毫秒时间戳；Tab id 只需要进程内单调唯一，取不到时退化为 0。
fn current_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

/// FTP 连接：打开时直接进入双栏文件视图。
pub(crate) struct FtpConnectionOpener;

impl ConnectionOpener for FtpConnectionOpener {
    fn open(
        &self,
        connection: &StoredConnection,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if connection.connection_type != ConnectionType::Ftp {
            return false;
        }
        TabEnvironment {
            mode,
            home,
            window,
            cx,
        }
        .open_sftp_view(connection)
    }

    fn open_sftp(
        &self,
        connection: &StoredConnection,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if connection.connection_type != ConnectionType::Ftp {
            return false;
        }
        TabEnvironment {
            mode: TabOpenMode::Activate,
            home,
            window,
            cx,
        }
        .open_sftp_view(connection)
    }
}

/// 串口连接：打开串口终端标签。
pub(crate) struct SerialConnectionOpener;

impl ConnectionOpener for SerialConnectionOpener {
    fn open(
        &self,
        connection: &StoredConnection,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if connection.connection_type != ConnectionType::Serial {
            return false;
        }
        TabEnvironment {
            mode,
            home,
            window,
            cx,
        }
        .open_serial_like(SerialLikeKind::Serial, connection.clone())
    }
}

/// Telnet 连接：打开 Telnet 终端标签。
pub(crate) struct TelnetConnectionOpener;

impl ConnectionOpener for TelnetConnectionOpener {
    fn open(
        &self,
        connection: &StoredConnection,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if connection.connection_type != ConnectionType::Telnet {
            return false;
        }
        TabEnvironment {
            mode,
            home,
            window,
            cx,
        }
        .open_serial_like(SerialLikeKind::Telnet, connection.clone())
    }
}
