//! 宿主终端组件:把原生终端适配为资源工作台可嵌入的终端页面。
//!
//! 契约由 `resource_view::TerminalHost` 定义,这里给出宿主的真实实现。
//! 扩展在 manifest 的 `terminal` 页面里声明要跑的命令(例如
//! `docker exec -it {{id}} sh`),宿主据插值后的命令行启动一个本地终端
//! 并嵌进工作台页面。
//!
//! 选择「宿主起本地进程」而不是「provider 侧执行」的原因:扩展协议目前
//! 只有请求-响应与 job 两种形态,没有流式通道,交互式 exec 无法复用。
//! 走宿主本地终端可以立刻可用(见 `docs/extension-resource-plugins/`),
//! 代价是终端进程不受扩展沙箱约束——因此命令完全由 manifest 声明,
//! 宿主不执行任何未声明的自由文本命令。

use std::rc::Rc;

use gpui::{App, AppContext as _, Window};
use resource_view::{
    GlobalTerminalHost, TerminalHost, TerminalMount, TerminalMountError, TerminalMountRequest,
};

/// 以原生 `TerminalView` 实现的终端宿主。
struct NativeTerminalHost;

impl TerminalHost for NativeTerminalHost {
    fn mount(
        &self,
        request: TerminalMountRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<TerminalMount, TerminalMountError> {
        let command = request.command.trim();
        if command.is_empty() {
            return Err(TerminalMountError::LaunchFailed(
                "the terminal page declares an empty command".into(),
            ));
        }
        let config = terminal::LocalConfig {
            shell: Some(command.to_string()),
            args: request.args,
            working_dir: request.working_dir,
            env: request.env,
        };
        let view = cx.new(|cx| terminal_view::TerminalView::new(config, window, cx));
        let handle = view.clone();
        Ok(TerminalMount::new(view.into(), move |cx| {
            // 与标签页关闭复用同一条终态清理路径:注销注册表、释放连接占用、
            // 关闭底层 pty 进程。嵌入场景没有关闭确认流程。
            let _ = handle.update(cx, |view, cx| view.shutdown_embedded(cx));
        }))
    }

    fn dispose(&self, mount: TerminalMount, cx: &mut App) {
        mount.dispose(cx);
    }
}

/// 注入全局终端 host。应用启动时调用一次,须在 `terminal_view::init` 之后
/// (由 `onetcli_app::init` 负责)。
pub(crate) fn install_terminal_host(cx: &mut App) {
    cx.set_global(GlobalTerminalHost {
        host: Rc::new(NativeTerminalHost),
    });
}
