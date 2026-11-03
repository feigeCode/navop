//! `navop.terminal` 原生组件：把原生终端画进扩展页面。
//!
//! 页面是 TSX，终端只能是原生组件：
//!
//! ```tsx
//! const Terminal = nativeComponent("navop.terminal");
//! <Terminal command="docker" args={["exec", "-it", id, "/bin/sh"]} />
//! ```
//!
//! 谁起进程：**宿主的本地终端**，不是 provider。扩展协议只有请求-响应与
//! job 两种形态，没有流式 pty 通道，交互式 exec 复用不了；走本地终端可以
//! 立刻可用。代价是终端进程不受扩展沙箱约束，所以命令与参数由页面显式给、
//! 且页面所在扩展必须声明 `shell:exec`（网关在 `universal_plugins` 的
//! native 注册表里，这里再挡一道，防止有人绕过清单直接注册）。

use std::{rc::Rc, sync::OnceLock};

use gpui::prelude::*;
use gpui::{App, Focusable as _, Window};
use gpui_component::ActiveTheme as _;
use nsh_platform_gpui::{ComponentContext, NativeComponent};
use serde_json::Value;
use universal_plugins::{NativeViewContext, register_native_component};

/// 注册到宿主：应用启动时调用一次（`shell_domain` 的 init）。
pub fn install(cx: &mut App) {
    let (sender, receiver) =
        smol::channel::unbounded::<gpui::Entity<terminal_view::TerminalView>>();
    let _ = CLEANUP.set(sender);
    // 组件被 drop 时主线程拿不到 `App`，所以把要关的终端丢进这条队列，
    // 由主线程上的任务补一次 `shutdown_embedded`：注销广播输入、释放连接
    // 占用、关掉 pty。队列表空时这条任务只是在等。
    cx.spawn(async move |cx| {
        while let Ok(view) = receiver.recv().await {
            cx.update(|cx| view.update(cx, |view, cx| view.shutdown_embedded(cx)));
        }
    })
    .detach();
    register_native_component(
        TERMINAL,
        Rc::new(|context: NativeViewContext| {
            Box::new(TerminalComponent::new(context)) as Box<dyn NativeComponent>
        }),
        cx,
    );
}

/// 待回收的终端：组件 drop 时投递，主线程任务消费。
static CLEANUP: OnceLock<smol::channel::Sender<gpui::Entity<terminal_view::TerminalView>>> =
    OnceLock::new();

const TERMINAL: &str = "navop.terminal";

/// 页面给的一组建终端参数（`set_prop` 逐条进来）。
#[derive(Default)]
struct TerminalProps {
    title: Option<String>,
    command: Option<String>,
    args: Vec<String>,
    env: Vec<(String, String)>,
    cwd: Option<String>,
    operation: Option<String>,
}

struct TerminalComponent {
    context: ComponentContext,
    allow_process: bool,
    props: TerminalProps,
    view: Option<gpui::Entity<terminal_view::TerminalView>>,
    /// 建视图时抄下来的焦点句柄：`focus_handle()` 拿不到 `App`，只能提前存。
    focus: Option<gpui::FocusHandle>,
    error: Option<String>,
    /// 启动参数变了（页面点了「重新连接」）：下一帧先回收旧终端再起新的。
    restart: bool,
}

impl TerminalComponent {
    fn new(context: NativeViewContext) -> Self {
        Self {
            allow_process: context.allow_process,
            context: context.component,
            props: TerminalProps::default(),
            view: None,
            focus: None,
            error: None,
            restart: false,
        }
    }

    /// 起终端。只做一次；参数变了就报错提示作者重建组件（终端进程没法
    /// 热换 shell，硬要支持只会让 PTY 泄漏）。
    fn ensure_started(&mut self, window: &mut Window, cx: &mut App) {
        if self.restart {
            // 页面换了 command/args/env：旧 pty 交给主线程回收，这里重开一个。
            if let Some(view) = self.view.take()
                && let Some(sender) = CLEANUP.get()
            {
                let _ = sender.try_send(view);
            }
            self.focus = None;
            self.error = None;
            self.restart = false;
        }
        if self.view.is_some() || self.error.is_some() {
            return;
        }
        if !self.allow_process {
            self.error = Some("这个页面没有 shell:exec 权限，不能启动终端".into());
            return;
        }
        if let Some(operation) = self.props.operation.as_deref() {
            self.error = Some(format!(
                "runtime 终端（{operation}）还没有通道：扩展协议暂无流式 pty"
            ));
            return;
        }
        let command = self.props.command.as_deref().unwrap_or_default().trim();
        if command.is_empty() {
            // 还没传 command：下一帧再看，不算错误。
            return;
        }
        let config = terminal::LocalConfig {
            shell: Some(command.to_owned()),
            args: std::mem::take(&mut self.props.args),
            working_dir: self.props.cwd.take(),
            env: std::mem::take(&mut self.props.env),
        };
        let view = cx.new(|cx| terminal_view::TerminalView::new(config, window, cx));
        self.focus = Some(view.read(cx).focus_handle(cx));
        self.view = Some(view);
        // 尺寸或可见性变化要重画：nsh 每帧会调 render，这里只需通知一次。
        self.context.redraw();
    }
}

impl Drop for TerminalComponent {
    fn drop(&mut self) {
        // 页面关掉时组件先被丢弃；`self.view` 这时还在，交给主线程回收，
        // 否则 pty 与连接占用会留到进程退出。
        if let Some(view) = self.view.take()
            && let Some(sender) = CLEANUP.get()
        {
            let _ = sender.try_send(view);
        }
    }
}

impl NativeComponent for TerminalComponent {
    fn render(&mut self, window: &mut Window, cx: &mut App) -> Option<gpui::AnyElement> {
        self.ensure_started(window, cx);
        if let Some(view) = &self.view {
            return Some(view.clone().into_any_element());
        }
        let message = self
            .error
            .clone()
            .unwrap_or_else(|| "终端启动中…".to_owned());
        let text = cx.new(|_| ErrorLine { message });
        Some(text.into_any_element())
    }

    fn set_prop(&mut self, name: &str, value: Option<&Value>, _window: &mut Window, _cx: &mut App) {
        // 只有影响「起哪个进程」的 prop 变化才重启终端：title 变一行字不值得
        // 把用户正在跑的 shell 关掉。
        let relaunch =
            matches!(name, "command" | "args" | "env" | "cwd" | "operation") && self.view.is_some();
        match (name, value) {
            ("title", Some(Value::String(title))) => self.props.title = Some(title.clone()),
            ("command", Some(Value::String(command))) => self.props.command = Some(command.clone()),
            ("args", Some(Value::Array(args))) => {
                self.props.args = args
                    .iter()
                    .filter_map(|arg| arg.as_str().map(str::to_owned))
                    .collect();
            }
            ("env", Some(Value::Object(env))) => {
                self.props.env = env
                    .iter()
                    .filter_map(|(key, value)| {
                        value.as_str().map(|value| (key.clone(), value.to_owned()))
                    })
                    .collect();
            }
            ("cwd", Some(Value::String(cwd))) => self.props.cwd = Some(cwd.clone()),
            ("operation", Some(Value::String(operation))) => {
                self.props.operation = Some(operation.clone());
            }
            // 属性被移除：只有 command 会让终端不可用，其余保留上一次的值。
            ("command", None) => self.props.command = None,
            ("title" | "args" | "env" | "cwd" | "operation", None) => {}
            _ => {}
        }
        if relaunch {
            self.restart = true;
            self.context.redraw();
        }
    }

    fn focus_handle(&self) -> Option<gpui::FocusHandle> {
        // 终端要拿键盘：把它自己的焦点句柄交给 nsh，Tab 就能走进来。
        self.focus.clone()
    }
}

/// 起不来时的占位：一行可读的错误，不是空白。
struct ErrorLine {
    message: String,
}

impl gpui::Render for ErrorLine {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        gpui::div()
            .size_full()
            .p_3()
            .text_sm()
            .text_color(cx.theme().colors.danger)
            .child(self.message.clone())
    }
}
