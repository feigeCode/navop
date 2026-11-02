//! 页面可见的宿主 ABI：`host.call("navop.*", …)` 的分发表。
//!
//! gpui-shell 那一版把这些能力做成 `HostModule`（宿主在装配期把它们挂进
//! `Policy`，脚本 `import "navop.theme"`）；nsh 没有模块注册表，页面只有
//! 一条 `host.call(name, data) -> Promise`。所以这里把同一批能力摊平成
//! **带命名空间的调用名**，一个名字一个同步应答：
//!
//! ```text
//! navop.context.current      → { extensionId, viewId, backends, connection }
//! navop.log.info             → null（message 超过 16 KiB 拒绝）
//! navop.i18n.locale          → "zh-CN"
//! navop.theme.snapshot       → { revision, dark, colors: {…} }
//! ```
//!
//! 三条约定，与 gpui-shell 版保持一致，页面侧不用改协议：
//!
//! 1. **能力由 manifest 声明**：`nsh.json` 的 `capabilities.host` 列出允许的
//!    调用名（`navop.theme.*` 这种前缀通配也认），没声明的名字在引擎侧就
//!    回不去，宿主这里再按 `modules` 声明收一次。
//! 2. **错误是结构化 envelope**：`HostCall::reject` 只能带一句 message，
//!    稳定错误码因此编码成 `message\n__NAVOP_ERROR__<base64url-json>`，页面
//!    侧的 `decodeHostError` 拆出来（`error.rs` 同款格式）。
//! 3. **handler 跑在引擎线程上**：它必须 `Send + Sync`、不能阻塞、不能碰
//!    `App`。所以这一层不放 `Rc`/`RefCell` 状态：主题这类主线程数据由宿主
//!    算好推进 `HostAbi` 的缓存（`Mutex`），handler 只读缓存。资源句柄那种
//!    需要主线程的动作，走 nsh 的 rt 队列回到主线程（M4 的资源模块）。

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use nsh_engine::{HostCall, HostChannel, HostReply};
use serde_json::{Map, Value, json};

/// 宿主主题的一份投影（`navop.theme.snapshot` 的应答，也是推送事件的载荷）。
#[derive(Clone, Debug, Default)]
pub(crate) struct ThemeSnapshot {
    pub(crate) revision: u64,
    pub(crate) dark: bool,
    /// `gpui_component::Theme::colors` 的原始序列化结果：字段名即宿主 token 名，
    /// 值是 `#rrggbbaa`。不抄成匹配表 —— 组件库加色时这里自动跟上。
    pub(crate) colors: Map<String, Value>,
}

impl ThemeSnapshot {
    fn to_json(&self) -> Value {
        json!({
            "revision": self.revision,
            "dark": self.dark,
            "colors": Value::Object(self.colors.clone()),
        })
    }
}

/// 一次页面挂载的宿主 ABI。
///
/// 一个页面一个：身份（扩展 / view）、后端别名表、能力声明、主题缓存都在这里。
/// 生命周期与 `NshTab` 一致 —— tab 关掉，`host` 通道随 `AppView` 一起丢，
/// handler 不会再被调到。
pub(crate) struct HostAbi {
    extension_id: String,
    view_id: String,
    backends: BTreeMap<String, String>,
    modules: Vec<String>,
    theme: Mutex<ThemeSnapshot>,
    /// 连接型页面的连接事实（`navop.context.current().connection`）；
    /// 普通 tab 页面是 `null`。
    connection: Mutex<Value>,
    /// 取消位：随页面关闭置位，在途的 provider 调用据此 reject。
    cancelled: Mutex<bool>,
}

impl HostAbi {
    pub(crate) fn new(
        contribution: &extension_runtime::RegisteredShellViewContribution,
    ) -> Arc<Self> {
        Arc::new(Self {
            extension_id: contribution.extension_id.clone(),
            view_id: contribution.id.clone(),
            backends: contribution.backends.clone(),
            modules: contribution
                .modules
                .iter()
                .map(|module| module.module_specifier().to_owned())
                .collect(),
            theme: Mutex::new(ThemeSnapshot::default()),
            connection: Mutex::new(Value::Null),
            cancelled: Mutex::new(false),
        })
    }

    /// manifest 里声明的 host 模块（`contributes.shellViews[].modules`）。
    fn has_module(&self, module: &str) -> bool {
        self.modules.iter().any(|declared| declared == module)
    }

    /// 主题/明暗变化：更新 `navop.theme.snapshot` 的缓存。
    ///
    /// 返回"该推给页面的载荷"——变了才有值。推送本身留给调用方（它持有通道），
    /// 这样这一层不依赖引擎，测试台子也能直接断言。
    pub(crate) fn set_theme(&self, snapshot: ThemeSnapshot) -> Option<Value> {
        let mut theme = self.theme.lock().unwrap_or_else(|e| e.into_inner());
        if theme.revision == snapshot.revision && theme.dark == snapshot.dark {
            return None;
        }
        *theme = snapshot;
        Some(theme.to_json())
    }

    /// 页面关闭：置取消位，让在途调用按 `REQUEST_CANCELLED` 退出。
    pub(crate) fn cancel(&self) {
        *self.cancelled.lock().unwrap_or_else(|e| e.into_inner()) = true;
    }

    pub(crate) fn set_connection(&self, connection: Value) {
        if let Ok(mut slot) = self.connection.lock() {
            *slot = connection;
        }
    }

    fn cancelled(&self) -> bool {
        *self.cancelled.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// 把 ABI 注册到页面的通道上。
///
/// 每个模块单独判断 manifest 是否声明 —— 与 gpui-shell 版 `with_host_module`
/// 的开关语义一致：没声明就不挂，页面调用会得到"宿主没有处理这个名字"。
pub(crate) fn register(
    channel: &HostChannel,
    abi: Arc<HostAbi>,
    session: Arc<super::session::NshSession>,
    dispatcher: &super::dispatch::MainThread,
) {
    if abi.has_module("navop.context") {
        let abi_for_current = Arc::clone(&abi);
        channel.handle("navop.context.current", move |call| {
            respond(call, &abi_for_current, |abi| Ok(context_current(abi)));
        });
    }
    if abi.has_module("navop.log") {
        for level in ["debug", "info", "warn", "error"] {
            let abi_for_log = Arc::clone(&abi);
            channel.handle(format!("navop.log.{level}"), move |call| {
                let message = call_data_str(call.data(), "message");
                respond(call, &abi_for_log, |abi| log(abi, level, message));
            });
        }
    }
    if abi.has_module("navop.i18n") {
        let abi_for_i18n = Arc::clone(&abi);
        channel.handle("navop.i18n.locale", move |call| {
            respond(call, &abi_for_i18n, |_| {
                Ok(json!(rust_i18n::locale().to_string()))
            });
        });
        let abi_for_i18n = Arc::clone(&abi);
        channel.handle("navop.i18n.available", move |call| {
            respond(call, &abi_for_i18n, |_| Ok(json!(AVAILABLE_LOCALES)));
        });
    }
    if abi.has_module("navop.theme") {
        let abi_for_theme = Arc::clone(&abi);
        channel.handle("navop.theme.snapshot", move |call| {
            respond(call, &abi_for_theme, |abi| {
                let theme = abi.theme.lock().unwrap_or_else(|e| e.into_inner());
                Ok(theme.to_json())
            });
        });
        let abi_for_theme = Arc::clone(&abi);
        channel.handle("navop.theme.color", move |call| {
            let name = call_data_str(call.data(), "name");
            respond(call, &abi_for_theme, |abi| {
                let Some(name) = name else {
                    return Err(invalid_argument("navop.theme.color 需要 name 参数"));
                };
                let theme = abi.theme.lock().unwrap_or_else(|e| e.into_inner());
                match theme.colors.get(&name) {
                    Some(color) => Ok(color.clone()),
                    None => Ok(Value::Null),
                }
            });
        });
    }
    register_capability_plane(channel, &abi, &session);
    register_workbench(channel, &abi, &session);
    register_dev(channel, &abi, &session, dispatcher);
}

/// 开发者工具的能力：`navop.dev.*`。
///
/// 这些操作都要读写 GPUI 全局（注册表、窗口、文件选择器），所以在引擎
/// 线程上只做参数搬运，真正的执行投到主线程（[`super::dispatch`]）。
fn register_dev(
    channel: &HostChannel,
    abi: &Arc<HostAbi>,
    session: &Arc<super::session::NshSession>,
    dispatcher: &super::dispatch::MainThread,
) {
    if !abi.has_module("navop.dev") {
        return;
    }
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
            // 参数都在这里取成拥有所有权的字符串：闭包要跨线程。
            // `openView` 认的是扩展 id（宿主内部再补 dev 前缀），其余操作认的是
            // 项目根路径 —— 所以按操作决定先读哪个字段，不能一律 `root` 优先。
            let first = match op {
                DevOp::OpenView => payload_str(&data, "extensionId")
                    .or_else(|| payload_str(&data, "root"))
                    .or_else(|| data.as_str().map(str::to_owned)),
                _ => payload_str(&data, "root")
                    .or_else(|| payload_str(&data, "extensionId"))
                    .or_else(|| data.as_str().map(str::to_owned)),
            };
            let second = payload_str(&data, "viewId");
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
                    Arguments::Two => match first.zip(second) {
                        Some((first, second)) => (first, second),
                        None => {
                            reject(reply, invalid_argument(format!("{name} 需要两个参数")));
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
    ("navop.dev.openView", DevOp::OpenView),
    ("navop.dev.pickDirectory", DevOp::PickStart),
    ("navop.dev.pickResult", DevOp::PickResult),
];

#[derive(Clone, Copy)]
enum Arguments {
    None,
    One,
    Two,
}

#[derive(Clone, Copy)]
enum DevOp {
    List,
    Open,
    Reload,
    Watch,
    Remove,
    Logs,
    OpenView,
    PickStart,
    PickResult,
}

impl DevOp {
    fn arguments(self) -> Arguments {
        match self {
            Self::List | Self::PickStart | Self::PickResult => Arguments::None,
            Self::Open | Self::Reload | Self::Watch | Self::Remove | Self::Logs => Arguments::One,
            Self::OpenView => Arguments::Two,
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
            Self::OpenView => super::dev::op(cx, |ops| ops.open_view.clone())?(cx, first, second),
            Self::PickStart => super::dev::op(cx, |ops| ops.pick_start.clone())?(cx),
            Self::PickResult => super::dev::op(cx, |ops| ops.pick_result.clone())?(cx),
        }
    }
}

/// dev 操作的失败只有一句话（没有稳定 code），按参数错误回给页面。
fn settle_dev(reply: HostReply, result: Result<Value, String>) {
    match result {
        Ok(value) => reply.resolve(value),
        Err(message) => reply.reject(invalid_argument(message).render()),
    }
}

/// 资源工作台嵌入页的通道：`current()` 是纯读，`dispatch` 要等 provider。
fn register_workbench(
    channel: &HostChannel,
    abi: &Arc<HostAbi>,
    session: &Arc<super::session::NshSession>,
) {
    if !abi.has_module("navop.workbench") {
        return;
    }
    let abi_for_current = Arc::clone(abi);
    let session_for_current = Arc::clone(session);
    channel.handle("navop.workbench.current", move |call| {
        let session = Arc::clone(&session_for_current);
        respond(call, &abi_for_current, |_| {
            session.workbench_current().map_err(capability_failure)
        });
    });
    let abi_for_dispatch = Arc::clone(abi);
    let session_for_dispatch = Arc::clone(session);
    channel.handle("navop.workbench.dispatch", move |call| {
        let data = call.data().clone();
        let Some(reply) = call.into_reply() else {
            return;
        };
        if abi_for_dispatch.cancelled() {
            reject(reply, cancelled());
            return;
        }
        let operation = payload_str(&data, "operationId");
        let input = data.get("input").cloned().unwrap_or(Value::Null);
        // `dispatch(op, input, { confirmed })`；也接受旧的裸 `confirmed` 字段。
        let confirmed = data
            .get("options")
            .and_then(|options| options.get("confirmed"))
            .or_else(|| data.get("confirmed"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let cancel = session_for_dispatch.call_token();
        let session = Arc::clone(&session_for_dispatch);
        session.tokio().spawn(async move {
            let Some(operation) = operation else {
                reject(
                    reply,
                    invalid_argument("navop.workbench.dispatch 需要 operationId 参数"),
                );
                return;
            };
            settle(
                reply,
                session
                    .workbench_dispatch(&operation, input, confirmed, cancel)
                    .await,
            );
        });
    });
}

/// 能力平面（provider 侧资源）的 handler。
///
/// 这些调用都要等 provider 进程，绝不能占着引擎线程：所以先把应答句柄
/// `into_reply()` 取出来（`HostReply` 是 `Send`），再搬到 tokio 线程上
/// 跑，结论照 envelope 回填。
fn register_capability_plane(
    channel: &HostChannel,
    abi: &Arc<HostAbi>,
    session: &Arc<super::session::NshSession>,
) {
    if abi.has_module("navop.runtime") {
        let abi_for_runtime = Arc::clone(abi);
        let session = Arc::clone(&session);
        channel.handle("navop.runtime.info", move |call| {
            let backend = payload_str(call.data(), "backend");
            respond(call, &abi_for_runtime, |_| {
                let Some(backend) = backend else {
                    return Err(invalid_argument("navop.runtime.info 需要 backend 参数"));
                };
                session.runtime_info(&backend).map_err(capability_failure)
            });
        });
    }
    if abi.has_module("navop.resource") {
        let abi_for = Arc::clone(abi);
        let session_for_open = Arc::clone(&session);
        channel.handle("navop.resource.open", move |call| {
            let data = call.data().clone();
            let Some(reply) = call.into_reply() else {
                return;
            };
            if abi_for.cancelled() {
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
            let session = Arc::clone(&session);
            channel.handle(format!("navop.resource.{name}"), move |call| {
                let data = call.data().clone();
                let Some(reply) = call.into_reply() else {
                    return;
                };
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
    }
    if abi.has_module("navop.job") {
        let abi_for = Arc::clone(abi);
        let session_for_start = Arc::clone(&session);
        channel.handle("navop.job.start", move |call| {
            let data = call.data().clone();
            let Some(reply) = call.into_reply() else {
                return;
            };
            if abi_for.cancelled() {
                reject(reply, cancelled());
                return;
            }
            let resource = payload_str(&data, "resource");
            let method = payload_str(&data, "method");
            let params = payload_value(&data, "params");
            let cancel = session_for_start.call_token();
            let session = Arc::clone(&session_for_start);
            session.tokio().spawn(async move {
                let Some((resource, method)) = resource.zip(method) else {
                    reject(
                        reply,
                        invalid_argument("navop.job.start 需要 resource 与 method 参数"),
                    );
                    return;
                };
                settle(
                    reply,
                    session.start_job(&resource, method, params, cancel).await,
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
            let session = Arc::clone(&session);
            channel.handle(format!("navop.job.{name}"), move |call| {
                let handle = payload_str(call.data(), "handle");
                let Some(reply) = call.into_reply() else {
                    return;
                };
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
    }
    if abi.has_module("navop.event") {
        let abi_for = Arc::clone(abi);
        let session_for_open = Arc::clone(&session);
        channel.handle("navop.event.open", move |call| {
            let data = call.data().clone();
            let Some(reply) = call.into_reply() else {
                return;
            };
            if abi_for.cancelled() {
                reject(reply, cancelled());
                return;
            }
            let resource = payload_str(&data, "resource");
            let kind = payload_str(&data, "kind");
            let capacity = payload_u32(&data, "capacity");
            let cancel = session_for_open.call_token();
            let session = Arc::clone(&session_for_open);
            session.tokio().spawn(async move {
                let Some((resource, kind)) = resource.zip(kind) else {
                    reject(
                        reply,
                        invalid_argument("navop.event.open 需要 resource 与 kind 参数"),
                    );
                    return;
                };
                settle(
                    reply,
                    session.open_event(&resource, kind, capacity, cancel).await,
                );
            });
        });
        for (name, pick) in [("read", "read"), ("close", "close")] {
            let abi_for = Arc::clone(abi);
            let session = Arc::clone(&session);
            channel.handle(format!("navop.event.{name}"), move |call| {
                let data = call.data().clone();
                let Some(reply) = call.into_reply() else {
                    return;
                };
                if abi_for.cancelled() {
                    reject(reply, cancelled());
                    return;
                }
                let handle = payload_str(&data, "handle");
                let max_events = payload_u32(&data, "maxEvents");
                let wait_ms = payload_u32(&data, "waitMs");
                let cancel = session.call_token();
                let session = Arc::clone(&session);
                session.tokio().spawn(async move {
                    let Some(handle) = handle else {
                        reject(
                            reply,
                            invalid_argument(format!("navop.event.{pick} 需要 handle 参数")),
                        );
                        return;
                    };
                    let result = match pick {
                        "read" => {
                            session
                                .read_event(&handle, max_events, wait_ms, cancel)
                                .await
                        }
                        _ => session.close_event(&handle, cancel).await,
                    };
                    settle(reply, result);
                });
            });
        }
    }
    if abi.has_module("navop.blob") {
        for (name, pick) in [("read", "read"), ("close", "close")] {
            let abi_for = Arc::clone(abi);
            let session = Arc::clone(&session);
            channel.handle(format!("navop.blob.{name}"), move |call| {
                let data = call.data().clone();
                let Some(reply) = call.into_reply() else {
                    return;
                };
                if abi_for.cancelled() {
                    reject(reply, cancelled());
                    return;
                }
                let handle = payload_str(&data, "handle");
                let max_bytes = payload_u32(&data, "maxBytes");
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
                        "read" => session.read_blob(&handle, max_bytes, cancel).await,
                        _ => session.close_blob(&handle, cancel).await,
                    };
                    settle(reply, result);
                });
            });
        }
    }
}

/// 把 `NavopError` 摊平成 handler 侧统一的失败类型。
fn capability_failure(error: crate::host_error::NavopError) -> HostFailure {
    error
}

/// 在 tokio 线程上把结论回填给页面。
fn settle(reply: HostReply, result: Result<Value, crate::host_error::NavopError>) {
    match result {
        Ok(value) => reply.resolve(value),
        Err(error) => reply.reject(error.render()),
    }
}

fn reject(reply: HostReply, failure: HostFailure) {
    reply.reject(failure.render());
}

fn payload_str(data: &Value, field: &str) -> Option<String> {
    data.get(field)?.as_str().map(str::to_owned)
}

fn payload_value(data: &Value, field: &str) -> Value {
    data.get(field).cloned().unwrap_or(Value::Null)
}

fn payload_u32(data: &Value, field: &str) -> Option<u32> {
    data.get(field)?
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
}

/// 宿主可能处在的语言，与 `one_core` 的常量同源（设置页下拉项就是这一组）。
const AVAILABLE_LOCALES: &[&str] = &[
    one_core::settings::LOCALE_EN,
    one_core::settings::LOCALE_ZH_CN,
    one_core::settings::LOCALE_ZH_HK,
];

/// 一个 handler 的公共壳：检查取消位、把结果或错误按 envelope 回给页面。
fn respond(
    call: HostCall,
    abi: &Arc<HostAbi>,
    body: impl FnOnce(&HostAbi) -> Result<Value, HostFailure>,
) {
    if abi.cancelled() {
        call.reject(render_error(cancelled()));
        return;
    }
    match body(abi) {
        Ok(value) => call.resolve(value),
        Err(failure) => call.reject(render_error(failure)),
    }
}

fn context_current(abi: &HostAbi) -> Value {
    json!({
        "extensionId": abi.extension_id,
        "viewId": abi.view_id,
        "backends": abi.backends.keys().collect::<Vec<_>>(),
        "connection": abi
            .connection
            .lock()
            .map(|connection| connection.clone())
            .unwrap_or(Value::Null),
    })
}

fn log(abi: &HostAbi, level: &str, message: Option<String>) -> Result<Value, HostFailure> {
    let Some(message) = message else {
        return Err(invalid_argument(format!(
            "navop.log.{level} 需要 message 参数"
        )));
    };
    if message.len() > 16 * 1024 {
        return Err(invalid_argument("log message 超过 16 KiB"));
    }
    let (extension_id, view_id) = (&abi.extension_id, &abi.view_id);
    match level {
        "debug" => tracing::debug!(extension_id, view_id, "{message}"),
        "warn" => tracing::warn!(extension_id, view_id, "{message}"),
        "error" => tracing::error!(extension_id, view_id, "{message}"),
        _ => tracing::info!(extension_id, view_id, "{message}"),
    }
    Ok(Value::Null)
}

/// `data.message`（也接受裸字符串 —— `host.call("navop.log.info", "…")`）。
fn call_data_str(data: &Value, field: &str) -> Option<String> {
    match data {
        Value::String(text) => Some(text.clone()),
        value => value.get(field)?.as_str().map(str::to_owned),
    }
}

/// 与 gpui-shell 平面同一个信封、同一套稳定 code：统一走
/// [`crate::host_error`]，避免两个 UI 平面的错误词汇表漂移。
type HostFailure = crate::host_error::NavopError;

fn cancelled() -> HostFailure {
    HostFailure::new(
        crate::host_error::ErrorCode::RequestCancelled,
        "页面已关闭，调用取消",
    )
}

fn invalid_argument(message: impl Into<String>) -> HostFailure {
    HostFailure::new(crate::host_error::ErrorCode::InvalidArgument, message)
}

/// `message\n__NAVOP_ERROR__<base64url-json>`。
fn render_error(failure: HostFailure) -> String {
    failure.render()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn error_envelope_round_trips() {
        let rendered = render_error(invalid_argument("缺少 name"));
        let (message, envelope) = rendered
            .split_once(crate::host_error::ERROR_MARKER)
            .expect("envelope marker present");
        assert_eq!(message.trim(), "缺少 name");
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(envelope)
            .expect("base64url payload");
        let value: Value = serde_json::from_slice(&decoded).expect("json payload");
        assert_eq!(value["code"], "INVALID_ARGUMENT");
        assert_eq!(value["message"], "缺少 name");
        assert_eq!(value["retryable"], false);
    }

    #[test]
    fn call_data_accepts_string_and_object() {
        assert_eq!(
            call_data_str(&json!("hi"), "message").as_deref(),
            Some("hi")
        );
        assert_eq!(
            call_data_str(&json!({ "message": "hi" }), "message").as_deref(),
            Some("hi")
        );
        assert_eq!(call_data_str(&json!({}), "message"), None);
    }
}
