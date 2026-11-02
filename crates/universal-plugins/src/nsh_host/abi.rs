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

use base64::Engine as _;
use nsh_engine::{HostCall, HostChannel};
use serde_json::{Map, Value, json};

/// SDK 用来定位错误信封的 marker（与 `shell_plugin_host::error` 同一个字符串：
/// 页面侧的 `decodeHostError` 只认这个值，两套宿主不能各说各的。gpui-shell
/// 那套删掉后，这个常量就只剩这里一处）。
const ERROR_MARKER: &str = "__NAVOP_ERROR__";

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

    fn cancelled(&self) -> bool {
        *self.cancelled.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// 把 ABI 注册到页面的通道上。
///
/// 每个模块单独判断 manifest 是否声明 —— 与 gpui-shell 版 `with_host_module`
/// 的开关语义一致：没声明就不挂，页面调用会得到"宿主没有处理这个名字"。
pub(crate) fn register(channel: &HostChannel, abi: Arc<HostAbi>) {
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
                    return Err(HostFailure::invalid_argument(
                        "navop.theme.color 需要 name 参数",
                    ));
                };
                let theme = abi.theme.lock().unwrap_or_else(|e| e.into_inner());
                match theme.colors.get(&name) {
                    Some(color) => Ok(color.clone()),
                    None => Ok(Value::Null),
                }
            });
        });
    }
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
        call.reject(render_error(HostFailure::cancelled()));
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
        "connection": Value::Null,
    })
}

fn log(abi: &HostAbi, level: &str, message: Option<String>) -> Result<Value, HostFailure> {
    let Some(message) = message else {
        return Err(HostFailure::invalid_argument(format!(
            "navop.log.{level} 需要 message 参数"
        )));
    };
    if message.len() > 16 * 1024 {
        return Err(HostFailure::invalid_argument("log message 超过 16 KiB"));
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

/// 一条 ABI 错误：稳定 code + message（+ 可选 details）。
struct HostFailure {
    code: &'static str,
    message: String,
    retryable: bool,
    details: Option<Value>,
}

impl HostFailure {
    fn invalid_argument(message: impl Into<String>) -> Self {
        Self {
            code: "INVALID_ARGUMENT",
            message: message.into(),
            retryable: false,
            details: None,
        }
    }

    fn cancelled() -> Self {
        Self {
            code: "REQUEST_CANCELLED",
            message: "页面已关闭，调用取消".into(),
            retryable: false,
            details: None,
        }
    }
}

/// 与 `shell_plugin_host::error` 同一个信封：`message\n__NAVOP_ERROR__<base64url-json>`。
fn render_error(failure: HostFailure) -> String {
    let message = failure.message;
    let mut envelope = json!({
        "code": failure.code,
        "message": message,
        "retryable": failure.retryable,
    });
    if let Some(details) = failure.details {
        envelope["details"] = details;
    }
    let encoded =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(envelope.to_string().as_bytes());
    format!("{message}\n{ERROR_MARKER}{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_envelope_round_trips() {
        let rendered = render_error(HostFailure::invalid_argument("缺少 name"));
        let (message, envelope) = rendered
            .split_once(ERROR_MARKER)
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
