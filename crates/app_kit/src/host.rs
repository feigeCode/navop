//! 领域事件需要回到应用壳时使用的全局宿主桥。
//!
//! `App::subscribe` 的回调只能拿到 `&mut App`，拿不到 `Context<HomePage>`；
//! 领域把事件交给这里注册的应用壳实现，由具体产品决定怎么打开 SFTP、
//! 保存临时连接等。没有注册宿主时（例如领域单元测试）调用会被忽略。

use gpui::{AnyWindowHandle, App, Window};
use one_core::storage::StoredConnection;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

/// 应用壳为 shell 领域提供的事件出口。
pub trait ShellHost: Send + Sync {
    fn open_sftp(&self, connection: StoredConnection, window: &mut Window, cx: &mut App);

    fn save_temporary_connection(
        &self,
        connection: StoredConnection,
        window: &mut Window,
        cx: &mut App,
    );

    /// SFTP 双栏视图请求在指定目录打开本地终端。
    fn open_local_terminal(&self, working_dir: String, window: &mut Window, cx: &mut App);

    /// SFTP 双栏视图请求在指定目录打开该连接的 SSH 终端。
    fn open_ssh_terminal(
        &self,
        connection: StoredConnection,
        working_dir: String,
        window: &mut Window,
        cx: &mut App,
    );

    /// 会话日志列表请求打开一条 `.cast` 记录。
    fn open_session_log_file(&self, path: PathBuf, window: &mut Window, cx: &mut App);
}

static SHELL_HOST: OnceLock<Arc<dyn ShellHost>> = OnceLock::new();

/// 注册当前产品的 shell 宿主；重复注册会被忽略。
pub fn set_shell_host(host: Arc<dyn ShellHost>) {
    let _ = SHELL_HOST.set(host);
}

pub fn shell_host() -> Option<Arc<dyn ShellHost>> {
    SHELL_HOST.get().cloned()
}

/// 应用壳打开「扩展管理」页的出口。
///
/// 新建连接向导里的「更多连接 / 安装扩展」入口属于壳层能力：`app_kit`
/// 不依赖扩展运行时，只把父窗口交给产品注册的实现去打开对应 tab。
pub trait ExtensionsHost: Send + Sync {
    /// `parent` 是向导弹窗的父窗口（应用主窗口）；返回 `false` 表示无法打开。
    fn open_extensions(&self, parent: AnyWindowHandle, window: &mut Window, cx: &mut App) -> bool;
}

static EXTENSIONS_HOST: OnceLock<Arc<dyn ExtensionsHost>> = OnceLock::new();

/// 注册扩展管理页入口；重复注册会被忽略。
pub fn set_extensions_host(host: Arc<dyn ExtensionsHost>) {
    let _ = EXTENSIONS_HOST.set(host);
}

pub fn extensions_host() -> Option<Arc<dyn ExtensionsHost>> {
    EXTENSIONS_HOST.get().cloned()
}
