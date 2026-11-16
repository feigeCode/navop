//! 页面可见的宿主 ABI（v2）：`host.call` / `host.post` 的分发表。
//!
//! v2 把启动事实、事实变化和动作分开：
//!
//! - **启动事实**走 `HostChannel::set_params`（`tab.rs` 在首帧前写入，页面同步
//!   读 `host.params`）；
//! - **事实变化**走 `channel.send("navop.event", …)`（provider 事件流，泵在
//!   [`super::tab::EventPump`]）；
//! - **动作**走 `host.call`，只有下面这张表；日志走 `host.post("navop.log")`。
//!
//! ```text
//! navop.resource.{open,invoke,ping,close}
//! navop.job.{start,status,cancel,result,close}
//! navop.blob.{read,close}
//! navop.event.{subscribe,unsubscribe}
//! navop.runtime.info
//! navop.app.open
//! navop.dev.*（9 个，仅内置扩展 com.navop.devtools）
//! navop.log（post）
//! ```
//!
//! 三条约定：
//!
//! 1. **能力由 manifest 声明**：`nsh.json` 里 `capabilities.host` 的调用名在
//!    引擎侧拦一次；宿主这边再按 `contributes.shellViews[].modules` 收一次。
//!    v2 的收法是**全部名字都注册**：模块没声明时 handler 回
//!    `PERMISSION_DENIED`（`details.module` 给出缺的模块），这样"模块忘声明"
//!    与"名字写错（引擎回 ERR_NSH_HOST_ERROR）"页面能区分。
//! 2. **错误是结构化 JS 错误**：`reply.reject_error(JsError::Fields {…})` 把
//!    稳定 code / retryable / details 直接挂在页面 catch 到的 `error` 上
//!    （[`crate::host_error`]）。
//! 3. **handler 跑在引擎线程上**：它必须 `Send + Sync`、不能阻塞、不能碰
//!    `App`。所以要等 provider 的调用都先 `into_reply()`（`HostReply` 是
//!    `Send`），再搬到 tokio 线程上跑，结论回主线程/引擎线程应答。

use std::rc::Rc;
use std::sync::{Arc, Mutex};

use nsh_engine::{HostCall, HostChannel, HostReply};
use serde_json::{Value, json};

use super::session::NshSession;
use super::tab::EventPump;

/// 唯一允许调用 `navop.dev.*` 的内置扩展（spec §6）。
pub(crate) const DEVTOOLS_EXTENSION_ID: &str = "com.navop.devtools";

/// 一次页面挂载的宿主 ABI。
///
/// 一个页面一个：身份（扩展 / view）、后端别名表、模块声明、取消位都在这里。
/// 生命周期与 `NshTab` 一致 —— tab 关掉，`host` 通道随 `AppView` 一起丢，
/// handler 不会再被调到。
pub(crate) struct HostAbi {
    pub(crate) extension_id: String,
    /// 打开这个 tab 时的初始路由（日志用）。
    route: Option<String>,
    backends: std::collections::BTreeMap<String, String>,
    modules: Vec<String>,
    /// 清单声明的权限（`db:*` 等）：navop.db 能力面按它收口。
    /// 清单里的 `permissions`：数据库网关按它判读写（只有 nsh-db 构建读它）。
    #[cfg_attr(not(feature = "nsh-db"), allow(dead_code))]
    pub(crate) permissions: Vec<String>,
    /// 取消位：随页面关闭置位，在途的 provider 调用据此 reject。
    cancelled: Mutex<bool>,
}

impl HostAbi {
    pub(crate) fn new(
        contribution: &extension_runtime::RegisteredShellAppContribution,
        route: Option<&str>,
    ) -> Arc<Self> {
        Arc::new(Self {
            extension_id: contribution.extension_id.clone(),
            route: route.map(str::to_owned),
            backends: contribution.backends.clone(),
            modules: contribution
                .modules
                .iter()
                .map(|module| module.module_specifier().to_owned())
                .collect(),
            permissions: contribution.permissions.clone(),
            cancelled: Mutex::new(false),
        })
    }

    /// manifest 里声明的 host 模块（完整 specifier，如 `"navop.resource"`）。
    pub(crate) fn has_module(&self, module: &str) -> bool {
        self.modules.iter().any(|declared| declared == module)
    }

    /// 唯一的 backend 别名（`runtime.info` 省略 `backend` 时用它）。
    fn single_backend(&self) -> Option<String> {
        let mut aliases = self.backends.keys();
        let first = aliases.next()?;
        aliases.next().is_none().then(|| first.clone())
    }

    /// 页面关闭：置取消位，让在途调用按 `REQUEST_CANCELLED` 退出。
    pub(crate) fn cancel(&self) {
        *self.cancelled.lock().unwrap_or_else(|e| e.into_inner()) = true;
    }

    pub(crate) fn cancelled(&self) -> bool {
        *self.cancelled.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// 把 ABI 注册到页面的通道上。
///
/// 全部 v2 名字都注册；模块声明的检查在 handler 里做（模块缺失 →
/// `PERMISSION_DENIED` + `details.module`），见文件头。
pub(crate) fn register(
    channel: &HostChannel,
    abi: Arc<HostAbi>,
    session: Arc<NshSession>,
    pump: &Arc<EventPump>,
    dispatcher: &super::dispatch::MainThread,
) {
    register_post(channel, &abi);
    register_capability_plane(channel, &abi, &session, pump);
    register_dev(channel, &abi, &session, dispatcher);
    register_ai_plane(channel, &abi, &session, dispatcher);
}
/// 主线程上打开 shell app：解析窗口 → `open_shell_app_with_context`。
///
/// 窗口解析复用 dev openApp 的兜底思路（活动窗口 → 主窗口），但这里在
/// 主线程任务里直接拿得到 `cx.defer` + `AnyWindowHandle::update`，不需要
/// navop_app 的私有 helper。窗口不存在时返回错误文本（settle_dev 会转成
/// 参数错误回给页面）。
pub(crate) fn open_app_on_main(
    extension_id: &str,
    route: Option<String>,
    context: Value,
    cx: &mut gpui::App,
) -> Result<Value, String> {
    let extension_id = extension_id.to_string();
    let context = (!context.is_null()).then_some(context);
    let opened = Rc::new(std::cell::Cell::new(false));
    let opened_flag = Rc::clone(&opened);
    // 打开 app 要窗口，等这一帧的借用结束再做（同 dev openApp）。
    cx.defer(move |cx| {
        let Some(window) = cx
            .active_window()
            .or_else(|| cx.window_stack().and_then(|stack| stack.first().copied()))
        else {
            tracing::warn!(
                extension_id,
                ?route,
                "no window available for navop.app.open"
            );
            return;
        };
        let _ = window.update(cx, |_, window, cx| {
            match &context {
                Some(context) => extension_view::open_shell_app_with_context(
                    &extension_id,
                    route.as_deref(),
                    context.clone(),
                    window,
                    cx,
                ),
                None => extension_view::open_shell_app(&extension_id, route.as_deref(), window, cx),
            }
            opened_flag.set(true);
        });
    });
    // 打开动作被 `defer` 排到下一帧：这里只能确认“已入队”。窗口不在时
    // `defer` 里会记一条 warn——调用方不该因为当帧还没窗口就报错。
    let _ = opened.get();
    Ok(json!({ "ok": true }))
}

/// 在指定页签容器里打开扩展页面（数据库标签页内的 `App.open` 用）。
///
/// 和 [`open_app_on_main`] 的区别只在落点：这里把页面开进 `container` 自己的
/// 页签栏（与触发它的数据库标签页同栏），窗口从主线程活动窗口取。
pub(crate) fn open_app_in_container(
    container: gpui::Entity<one_core::tab_container::TabContainer>,
    extension_id: &str,
    route: Option<String>,
    context: Value,
    cx: &mut gpui::App,
) -> Result<Value, String> {
    let extension_id = extension_id.to_string();
    let context = (!context.is_null()).then_some(context);
    cx.defer(move |cx| {
        let Some(window) = cx
            .active_window()
            .or_else(|| cx.window_stack().and_then(|stack| stack.first().copied()))
        else {
            tracing::warn!(
                extension_id,
                ?route,
                "no window available for navop.app.open"
            );
            return;
        };
        let _ = window.update(cx, |_, window, cx| {
            extension_view::open_shell_app_in_container(
                &extension_id,
                route.as_deref(),
                context.clone().unwrap_or(Value::Null),
                container,
                window,
                cx,
            );
        });
    });
    // 和 `open_app_on_main` 一样：打开被排到下一帧，这里只能确认“已入队”。
    Ok(json!({ "ok": true }))
}

/// `host.post("navop.log", { level, message, fields })`。
///
/// post 没有应答：模块没声明、message 缺失或超长都只能记宿主日志，不能
/// 回给页面（spec §5）。`host.call("navop.log")` 不是 v2 表面，按参数错误回。
fn register_post(channel: &HostChannel, abi: &Arc<HostAbi>) {
    let abi_for = Arc::clone(abi);
    channel.handle("navop.log", move |call| {
        if !call.is_post() {
            let Some(reply) = call.into_reply() else {
                return;
            };
            reject(reply, invalid_argument("navop.log 只能通过 host.post 发送"));
            return;
        }
        if !abi_for.has_module("navop.log") {
            tracing::debug!(
                extension_id = %abi_for.extension_id,
                route = ?abi_for.route,
                "页面调用了未声明的 log 模块，日志丢弃"
            );
            return;
        }
        let data = call.data();
        let level = payload_str(data, "level").unwrap_or_else(|| "info".to_owned());
        let message = payload_str(data, "message");
        let fields = data.get("fields").cloned();
        log(&abi_for, &level, message, fields);
    });
}

/// 开发者工具的能力：`navop.dev.*`。
///
/// 这些操作都要读写 GPUI 全局（注册表、窗口、文件选择器），所以在引擎
/// 线程上只做参数搬运，真正的执行投到主线程（[`super::dispatch`]）。
/// 只有内置扩展能调；别的扩展也注册，但拿到 `PERMISSION_DENIED`。
fn register_dev(
    channel: &HostChannel,
    abi: &Arc<HostAbi>,
    session: &Arc<NshSession>,
    dispatcher: &super::dispatch::MainThread,
) {
    for (name, op) in DEV_OPS {
        let abi_for = Arc::clone(abi);
        let session = Arc::clone(session);
        let dispatcher = dispatcher.clone();
        channel.handle(name, move |call| {
            let data = call.data().clone();
            let Some(reply) = call.into_reply() else {
                return;
            };
            if abi_for.cancelled() {
                reject(reply, cancelled());
                return;
            }
            if abi_for.extension_id != DEVTOOLS_EXTENSION_ID || !abi_for.has_module("navop.dev") {
                reject(reply, permission_denied("dev"));
                return;
            }
            // 参数都在这里取成拥有所有权的字符串：闭包要跨线程。
            // `openApp` 认的是扩展 id（宿主内部再补 dev 前缀）加可选路由，
            // 其余操作认的是项目根路径 —— 所以按操作决定先读哪个字段，
            // 不能一律 `root` 优先。
            let first = match op {
                DevOp::OpenApp => payload_str(&data, "extensionId")
                    .or_else(|| payload_str(&data, "root"))
                    .or_else(|| data.as_str().map(str::to_owned)),
                _ => payload_str(&data, "root")
                    .or_else(|| payload_str(&data, "extensionId"))
                    .or_else(|| data.as_str().map(str::to_owned)),
            };
            let second = payload_str(&data, "route");
            let tail = payload_u32(&data, "tail").unwrap_or(200) as usize;
            let dispatcher = dispatcher.clone();
            session.tokio().spawn(async move {
                let (first, second) = match op.arguments() {
                    Arguments::None => (String::new(), String::new()),
                    Arguments::One => match first {
                        Some(first) => (first, String::new()),
                        None => {
                            reject(reply, invalid_argument(format!("{name} 需要参数")));
                            return;
                        }
                    },
                    Arguments::OneOrTwo => match first {
                        Some(first) => (first, second.unwrap_or_default()),
                        None => {
                            reject(reply, invalid_argument(format!("{name} 需要参数")));
                            return;
                        }
                    },
                };
                settle_dev(
                    reply,
                    dispatcher
                        .run(move |cx| op.run(cx, &first, &second, tail))
                        .await,
                );
            });
        });
    }
}

/// `navop.dev` 的全部操作：名字、参数个数、执行方式钉在一处，
/// 注册与执行就不会各写一遍。
const DEV_OPS: [(&str, DevOp); 9] = [
    ("navop.dev.list", DevOp::List),
    ("navop.dev.open", DevOp::Open),
    ("navop.dev.reload", DevOp::Reload),
    ("navop.dev.watch", DevOp::Watch),
    ("navop.dev.remove", DevOp::Remove),
    ("navop.dev.logs", DevOp::Logs),
    ("navop.dev.openApp", DevOp::OpenApp),
    ("navop.dev.pickDirectory", DevOp::PickStart),
    ("navop.dev.pickResult", DevOp::PickResult),
];

#[derive(Clone, Copy)]
enum Arguments {
    None,
    One,
    OneOrTwo,
}

#[derive(Clone, Copy)]
enum DevOp {
    List,
    Open,
    Reload,
    Watch,
    Remove,
    Logs,
    OpenApp,
    PickStart,
    PickResult,
}

impl DevOp {
    fn arguments(self) -> Arguments {
        match self {
            Self::List | Self::PickStart | Self::PickResult => Arguments::None,
            Self::Open | Self::Reload | Self::Watch | Self::Remove | Self::Logs => Arguments::One,
            Self::OpenApp => Arguments::OneOrTwo,
        }
    }

    /// 在主线程上执行；用不到的参数是占位。
    fn run(
        self,
        cx: &mut gpui::App,
        first: &str,
        second: &str,
        tail: usize,
    ) -> Result<Value, String> {
        match self {
            Self::List => super::dev::op(cx, |ops| ops.list.clone())?(cx),
            Self::Open => super::dev::op(cx, |ops| ops.open.clone())?(cx, first),
            Self::Reload => super::dev::op(cx, |ops| ops.reload.clone())?(cx, first),
            Self::Watch => super::dev::op(cx, |ops| ops.watch.clone())?(cx, first),
            Self::Remove => super::dev::op(cx, |ops| ops.remove.clone())?(cx, first),
            Self::Logs => super::dev::op(cx, |ops| ops.logs.clone())?(cx, first, tail),
            Self::OpenApp => super::dev::op(cx, |ops| ops.open_app.clone())?(
                cx,
                first,
                (!second.is_empty()).then_some(second),
            ),
            Self::PickStart => super::dev::op(cx, |ops| ops.pick_start.clone())?(cx),
            Self::PickResult => super::dev::op(cx, |ops| ops.pick_result.clone())?(cx),
        }
    }
}

/// AI 平面（`navop.ai.*`，plugin-ai-tools.md §9.4）。
///
/// 宿主不再内置「AI 侧栏 + 图标栏」：页面自己挂 `nativeComponent("navop.ai")`
/// 决定放在哪儿。因此这三个名字的语义变成：
///
/// - `navop.ai.context`：`host.post`，把页面上下文转给**本页面里已挂载的**
///   AI 组件（同扩展的实例都收一份；没挂就丢弃，post 没有应答）。
/// - `navop.ai.open` / `navop.ai.ask`：保留名字，但宿主没有可展开的侧栏，
/// 宿主不再内置 AI 侧栏：页面要自己挂 `nativeComponent("navop.ai")`。
fn ai_unavailable() -> HostFailure {
    invalid_argument(
        "宿主不再内置 AI 侧栏；请在页面里用 nativeComponent(\"navop.ai\") 自己挂一个",
    )
}

fn register_ai_plane(
    channel: &HostChannel,
    abi: &Arc<HostAbi>,
    session: &Arc<NshSession>,
    dispatcher: &super::dispatch::MainThread,
) {
    let abi_for_context = Arc::clone(abi);
    let session_for_context = Arc::clone(session);
    let dispatcher_for_context = dispatcher.clone();
    channel.handle("navop.ai.context", move |call| {
        let context = call.data().clone();
        if !call.is_post() {
            if let Some(reply) = call.into_reply() {
                reject(reply, invalid_argument("navop.ai.context 只接受 host.post"));
            }
            return;
        }
        if abi_for_context.cancelled() {
            return;
        }
        let extension_id = abi_for_context.extension_id.clone();
        let dispatcher = dispatcher_for_context.clone();
        session_for_context.tokio().spawn(async move {
            let _ = dispatcher
                .run(move |cx| {
                    super::native_ai::publish_context(&extension_id, context, cx);
                    Ok(serde_json::json!(null))
                })
                .await;
        });
    });

    for name in ["navop.ai.open", "navop.ai.ask"] {
        let abi_for = Arc::clone(abi);
        channel.handle(name, move |call| {
            let Some(reply) = call.into_reply() else {
                return;
            };
            if abi_for.cancelled() {
                reject(reply, cancelled());
                return;
            }
            reject(reply, ai_unavailable());
        });
    }
}

/// 收敛一次 dev 操作的结果。
///
/// dev 操作的失败只有一句话（没有稳定 code），按参数错误回给页面。
fn settle_dev(reply: HostReply, result: Result<Value, String>) {
    match result {
        Ok(value) => reply.resolve(value),
        Err(message) => reply.reject_error(invalid_argument(message).to_js_error()),
    }
}

/// 能力平面（provider 侧资源）的 handler。
///
/// 这些调用都要等 provider 进程，绝不能占着引擎线程：所以先把应答句柄
/// `into_reply()` 取出来（`HostReply` 是 `Send`），再搬到 tokio 线程上
/// 跑，结论照结构化错误回填。
fn register_capability_plane(
    channel: &HostChannel,
    abi: &Arc<HostAbi>,
    session: &Arc<NshSession>,
    pump: &Arc<EventPump>,
) {
    // ---- runtime ----------------------------------------------------
    let abi_for_runtime = Arc::clone(abi);
    let session_for_runtime = Arc::clone(session);
    channel.handle("navop.runtime.info", move |call| {
        let backend = payload_str(call.data(), "backend");
        respond(call, &abi_for_runtime, "navop.runtime", |abi| {
            let backend = backend
                .or_else(|| abi.single_backend())
                .ok_or_else(|| invalid_argument("navop.runtime.info 需要 backend 参数"))?;
            session_for_runtime
                .runtime_info(&backend)
                .map_err(capability_failure)
        });
    });

    // ---- resource ---------------------------------------------------
    let abi_for_open = Arc::clone(abi);
    let session_for_open = Arc::clone(session);
    channel.handle("navop.resource.open", move |call| {
        let data = call.data().clone();
        let Some(reply) = call.into_reply() else {
            return;
        };
        if let Err(error) = gate(&abi_for_open, "navop.resource") {
            reject(reply, error);
            return;
        }
        if abi_for_open.cancelled() {
            reject(reply, cancelled());
            return;
        }
        let backend = payload_str(&data, "backend");
        let resource_type = payload_str(&data, "resourceType");
        let config = payload_value(&data, "config");
        let cancel = session_for_open.call_token();
        let session = Arc::clone(&session_for_open);
        session.tokio().spawn(async move {
            let Some((backend, resource_type)) = backend.zip(resource_type) else {
                reject(
                    reply,
                    invalid_argument("navop.resource.open 需要 backend 与 resourceType 参数"),
                );
                return;
            };
            settle(
                reply,
                session
                    .open_resource(&backend, resource_type, config, cancel)
                    .await,
            );
        });
    });

    for (name, pick) in [("invoke", "invoke"), ("ping", "ping"), ("close", "close")] {
        let abi_for = Arc::clone(abi);
        let session = Arc::clone(session);
        channel.handle(format!("navop.resource.{name}"), move |call| {
            let data = call.data().clone();
            let Some(reply) = call.into_reply() else {
                return;
            };
            if let Err(error) = gate(&abi_for, "navop.resource") {
                reject(reply, error);
                return;
            }
            if abi_for.cancelled() {
                reject(reply, cancelled());
                return;
            }
            let handle = payload_str(&data, "handle");
            let method = payload_str(&data, "method");
            let params = payload_value(&data, "params");
            let cancel = session.call_token();
            let session = Arc::clone(&session);
            session.tokio().spawn(async move {
                let Some(handle) = handle else {
                    reject(
                        reply,
                        invalid_argument(format!("navop.resource.{pick} 需要 handle 参数")),
                    );
                    return;
                };
                let result = match pick {
                    "invoke" => match method {
                        Some(method) => {
                            session
                                .invoke_resource(&handle, method, params, cancel)
                                .await
                        }
                        None => Err(invalid_argument("navop.resource.invoke 需要 method 参数")),
                    },
                    "ping" => session.ping_resource(&handle, cancel).await,
                    _ => session.close_resource(&handle, cancel).await,
                };
                settle(reply, result);
            });
        });
    }

    // ---- job --------------------------------------------------------
    let abi_for_start = Arc::clone(abi);
    let session_for_start = Arc::clone(session);
    channel.handle("navop.job.start", move |call| {
        let data = call.data().clone();
        let Some(reply) = call.into_reply() else {
            return;
        };
        if let Err(error) = gate(&abi_for_start, "navop.job") {
            reject(reply, error);
            return;
        }
        if abi_for_start.cancelled() {
            reject(reply, cancelled());
            return;
        }
        let handle = payload_str(&data, "handle");
        let method = payload_str(&data, "method");
        let params = payload_value(&data, "params");
        let cancel = session_for_start.call_token();
        let session = Arc::clone(&session_for_start);
        session.tokio().spawn(async move {
            let Some((handle, method)) = handle.zip(method) else {
                reject(
                    reply,
                    invalid_argument("navop.job.start 需要 handle 与 method 参数"),
                );
                return;
            };
            settle(
                reply,
                session.start_job(&handle, method, params, cancel).await,
            );
        });
    });
    for (name, pick) in [
        ("status", "status"),
        ("cancel", "cancel"),
        ("result", "result"),
        ("close", "close"),
    ] {
        let abi_for = Arc::clone(abi);
        let session = Arc::clone(session);
        channel.handle(format!("navop.job.{name}"), move |call| {
            let handle = payload_str(call.data(), "handle");
            let Some(reply) = call.into_reply() else {
                return;
            };
            if let Err(error) = gate(&abi_for, "navop.job") {
                reject(reply, error);
                return;
            }
            if abi_for.cancelled() {
                reject(reply, cancelled());
                return;
            }
            let cancel = session.call_token();
            let session = Arc::clone(&session);
            session.tokio().spawn(async move {
                let Some(handle) = handle else {
                    reject(
                        reply,
                        invalid_argument(format!("navop.job.{pick} 需要 handle 参数")),
                    );
                    return;
                };
                let result = match pick {
                    "status" => session.job_status(&handle, cancel).await,
                    "cancel" => session.cancel_job(&handle, cancel).await,
                    "result" => session.job_result(&handle, cancel).await,
                    _ => session.close_job(&handle, cancel).await,
                };
                settle(reply, result);
            });
        });
    }

    // ---- event ------------------------------------------------------
    let abi_for_subscribe = Arc::clone(abi);
    let session_for_subscribe = Arc::clone(session);
    let pump_for_subscribe = Arc::clone(pump);
    channel.handle("navop.event.subscribe", move |call| {
        let data = call.data().clone();
        let Some(reply) = call.into_reply() else {
            return;
        };
        if let Err(error) = gate(&abi_for_subscribe, "navop.event") {
            reject(reply, error);
            return;
        }
        if abi_for_subscribe.cancelled() {
            reject(reply, cancelled());
            return;
        }
        let handle = payload_str(&data, "handle");
        let kind = payload_str(&data, "kind");
        let since = payload_u64(&data, "since").unwrap_or(0);
        let cancel = session_for_subscribe.call_token();
        let pump = Arc::clone(&pump_for_subscribe);
        session_for_subscribe.tokio().spawn(async move {
            let (Some(handle), Some(kind)) = (handle, kind) else {
                reject(
                    reply,
                    invalid_argument("navop.event.subscribe 需要 handle 与 kind 参数"),
                );
                return;
            };
            settle(reply, pump.subscribe(handle, kind, since, cancel).await);
        });
    });

    let abi_for_unsubscribe = Arc::clone(abi);
    let session_for_unsubscribe = Arc::clone(session);
    let pump_for_unsubscribe = Arc::clone(pump);
    channel.handle("navop.event.unsubscribe", move |call| {
        let handle = payload_str(call.data(), "handle");
        let Some(reply) = call.into_reply() else {
            return;
        };
        if let Err(error) = gate(&abi_for_unsubscribe, "navop.event") {
            reject(reply, error);
            return;
        }
        if abi_for_unsubscribe.cancelled() {
            reject(reply, cancelled());
            return;
        }
        let cancel = session_for_unsubscribe.call_token();
        let session = Arc::clone(&session_for_unsubscribe);
        let pump = Arc::clone(&pump_for_unsubscribe);
        session.tokio().spawn(async move {
            let Some(handle) = handle else {
                reject(
                    reply,
                    invalid_argument("navop.event.unsubscribe 需要 handle 参数"),
                );
                return;
            };
            // 先掐泵再关流：泵的 `read_event` 随即因取消退出，不会为一条
            // 刚被页面退订的流再补一个错误帧。
            pump.unsubscribe(&handle);
            // 宿主事件没有 provider 侧的流可关：句柄一撤，泵就停了。
            if super::host_events::is_host_handle(&handle) {
                reply.resolve(json!({ "ok": true }));
                return;
            }
            let result = session.close_event(&handle, cancel).await;
            settle(reply, result);
        });
    });

    // ---- blob -------------------------------------------------------
    for (name, pick) in [("read", "read"), ("close", "close")] {
        let abi_for = Arc::clone(abi);
        let session = Arc::clone(session);
        channel.handle(format!("navop.blob.{name}"), move |call| {
            let data = call.data().clone();
            let Some(reply) = call.into_reply() else {
                return;
            };
            if let Err(error) = gate(&abi_for, "navop.blob") {
                reject(reply, error);
                return;
            }
            if abi_for.cancelled() {
                reject(reply, cancelled());
                return;
            }
            let handle = payload_str(&data, "handle");
            // `maxBytes` 是可选参数（spec §4.4）：省略时由宿主给一个默认
            // 分片大小，而不是让会话层把它当成"参数非法"。
            let max_bytes = payload_u32(&data, "maxBytes")
                .or(Some(extension_protocol::blob::MAX_BLOB_CHUNK_BYTES));
            let cancel = session.call_token();
            let session = Arc::clone(&session);
            session.tokio().spawn(async move {
                let Some(handle) = handle else {
                    reject(
                        reply,
                        invalid_argument(format!("navop.blob.{pick} 需要 handle 参数")),
                    );
                    return;
                };
                let result = match pick {
                    "read" => session
                        .read_blob(&handle, max_bytes, cancel)
                        .await
                        .and_then(blob_chunk),
                    _ => session.close_blob(&handle, cancel).await,
                };
                settle(reply, result);
            });
        });
    }
}

/// 把 `NavopError` 摊平成 handler 侧统一的失败类型。
fn capability_failure(error: crate::host_error::NavopError) -> HostFailure {
    error
}

/// `blob.read` 的应答整形（spec §4.4）。
///
/// 会话层给的是 `{ data: Base64, bytesRead, done }`（provider 协议为了避开
/// JSON 的 UTF-8 假设才这么传）；页面侧契约是 `{ data: number[], eof }`。
/// 转换放在 ABI 边界：页面直接拿到字节数组，不需要自己解码 Base64。
fn blob_chunk(chunk: Value) -> Result<Value, HostFailure> {
    use base64::Engine as _;
    let encoded = chunk
        .get("data")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let data = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| {
            let mut failure = HostFailure::new(
                crate::host_error::ErrorCode::ProtocolError,
                format!("provider 返回的 blob 数据不是合法 Base64: {error}"),
            );
            failure.details = Some(chunk.clone());
            failure
        })?;
    Ok(json!({
        "data": data,
        "eof": chunk.get("done").and_then(Value::as_bool).unwrap_or(false),
    }))
}

/// 在 tokio 线程上把结论回填给页面。
fn settle(reply: HostReply, result: Result<Value, crate::host_error::NavopError>) {
    match result {
        Ok(value) => reply.resolve(value),
        Err(error) => reply.reject_error(error.to_js_error()),
    }
}

pub(crate) fn reject(reply: HostReply, failure: HostFailure) {
    reply.reject_error(failure.to_js_error());
}

pub(crate) fn payload_str(data: &Value, field: &str) -> Option<String> {
    data.get(field)?.as_str().map(str::to_owned)
}

fn payload_value(data: &Value, field: &str) -> Value {
    data.get(field).cloned().unwrap_or(Value::Null)
}

pub(crate) fn payload_u32(data: &Value, field: &str) -> Option<u32> {
    data.get(field)?
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
}

fn payload_u64(data: &Value, field: &str) -> Option<u64> {
    data.get(field)?.as_u64()
}

/// 模块声明的门：没声明就 `PERMISSION_DENIED`（`details.module` 是短名）。
pub(crate) fn gate(abi: &HostAbi, module: &str) -> Result<(), HostFailure> {
    if abi.has_module(module) {
        return Ok(());
    }
    Err(permission_denied(
        module.strip_prefix("navop.").unwrap_or(module),
    ))
}

/// 一个 handler 的公共壳：检查模块声明、取消位，把结果或错误回给页面。
fn respond(
    call: HostCall,
    abi: &Arc<HostAbi>,
    module: &str,
    body: impl FnOnce(&HostAbi) -> Result<Value, HostFailure>,
) {
    if let Err(failure) = gate(abi, module) {
        call.reject_error(failure.to_js_error());
        return;
    }
    if abi.cancelled() {
        call.reject_error(cancelled().to_js_error());
        return;
    }
    match body(abi) {
        Ok(value) => call.resolve(value),
        Err(failure) => call.reject_error(failure.to_js_error()),
    }
}

/// `host.post("navop.log", …)` 的落点：写宿主日志，不答复。
///
/// message 上限 16 KiB（spec §5）：超出截断并 warn，而不是整条丢掉 —— 日志
/// 是排障用的，宁可少几个字也别把这次调用弄丢。
fn log(abi: &HostAbi, level: &str, message: Option<String>, fields: Option<Value>) {
    let Some(message) = message else {
        tracing::warn!(
            extension_id = %abi.extension_id,
            route = ?abi.route,
            "navop.log 缺少 message，日志丢弃"
        );
        return;
    };
    let message = if message.len() > 16 * 1024 {
        tracing::warn!(
            extension_id = %abi.extension_id,
            route = ?abi.route,
            "navop.log 的 message 超过 16 KiB，已截断"
        );
        let mut cut = 16 * 1024;
        while !message.is_char_boundary(cut) {
            cut -= 1;
        }
        message[..cut].to_owned()
    } else {
        message
    };
    let (extension_id, route) = (&abi.extension_id, &abi.route);
    match level {
        "debug" => {
            tracing::debug!(extension_id = %extension_id, route = ?route, fields = ?fields, "{message}")
        }
        "warn" => {
            tracing::warn!(extension_id = %extension_id, route = ?route, fields = ?fields, "{message}")
        }
        "error" => {
            tracing::error!(extension_id = %extension_id, route = ?route, fields = ?fields, "{message}")
        }
        _ => {
            tracing::info!(extension_id = %extension_id, route = ?route, fields = ?fields, "{message}")
        }
    }
}

/// 与 gpui-shell 平面同一个错误词汇表：统一走 [`crate::host_error`]。
type HostFailure = crate::host_error::NavopError;

pub(crate) fn cancelled() -> HostFailure {
    HostFailure::new(
        crate::host_error::ErrorCode::RequestCancelled,
        "页面已关闭，调用取消",
    )
}

pub(crate) fn invalid_argument(message: impl Into<String>) -> HostFailure {
    HostFailure::new(crate::host_error::ErrorCode::InvalidArgument, message)
}

fn permission_denied(module: &str) -> HostFailure {
    let mut error = HostFailure::new(
        crate::host_error::ErrorCode::PermissionDenied,
        format!("扩展没有声明 {module} 模块（或没有调用该能力的权限）"),
    );
    error.details = Some(json!({ "module": module }));
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_denied_names_the_missing_module() {
        let js = permission_denied("dev").to_js_error();
        assert_eq!(js.code(), Some("PERMISSION_DENIED"));
        let nsh_core::element::JsError::Fields(fields) = &js else {
            panic!("expected JsError::Fields");
        };
        assert_eq!(
            fields.field("details").cloned(),
            Some(json!({ "module": "dev" }))
        );
        assert_eq!(fields.field("retryable").cloned(), Some(Value::Bool(false)));
    }

    #[test]
    fn gate_strips_the_namespace_from_details() {
        let abi = HostAbi {
            extension_id: "dev.example".into(),
            route: Some("main".into()),
            backends: Default::default(),
            modules: Vec::new(),
            permissions: Vec::new(),
            cancelled: Mutex::new(false),
        };
        let error = gate(&abi, "navop.resource").expect_err("module not declared");
        let nsh_core::element::JsError::Fields(fields) = error.to_js_error() else {
            panic!("expected JsError::Fields");
        };
        assert_eq!(
            fields.field("details").cloned(),
            Some(json!({ "module": "resource" }))
        );
    }

    #[test]
    fn blob_chunk_turns_base64_into_bytes() {
        let chunk = json!({ "data": "AQID", "bytesRead": 3, "done": true });
        let out = blob_chunk(chunk).expect("合法的 Base64 应当能解开");
        assert_eq!(out["data"], json!([1, 2, 3]));
        assert_eq!(out["eof"], true);
    }

    #[test]
    fn blob_chunk_reports_broken_base64_as_protocol_error() {
        let error = blob_chunk(json!({ "data": "不是 Base64", "done": false }))
            .expect_err("坏掉的 Base64 不能当成功回给页面");
        assert_eq!(error.code, crate::host_error::ErrorCode::ProtocolError);
    }
}
