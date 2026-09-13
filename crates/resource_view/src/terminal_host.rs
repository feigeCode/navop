//! 终端组件宿主契约:资源工作台向宿主暴露的可嵌入终端接口。
//!
//! 接口由 resource_view 定义、宿主(navop main)以原生 `TerminalView` 实现。
//! 职责边界刻意收窄:工作台只负责「按 manifest 的 terminal 页面声明算出一个
//! 命令行,启动终端,并把视图嵌进页面」;终端本身的实现(PTY / SSH / 序列口)
//! 以及进程生命周期都归宿主,resource_view 不依赖任何终端 crate。
//!
//! 这与 `custom_page_host` 是同一种接缝:契约留在消费方,实现留在宿主,
//! 未注入 host 的构建(如测试、无终端能力的构建)渲染「此构建不可用」。

use gpui::{AnyView, App, Window};

/// 终端启动请求:由 manifest 的 terminal 页面声明 + 当前路由插值得到。
pub struct TerminalMountRequest {
    /// 终端标题(如 `exec · headroom`),供宿主做 tab/工具条标注。
    pub title: String,
    /// 可执行程序(如 `docker`)。
    pub command: String,
    /// 参数列表(占位符已插值完成)。
    pub args: Vec<String>,
    /// 追加的环境变量。
    pub env: Vec<(String, String)>,
    /// 工作目录。
    pub working_dir: Option<String>,
}

/// 挂载产物:可嵌入的终端视图 + 释放回调。
pub struct TerminalMount {
    pub view: AnyView,
    dispose: Option<Box<dyn FnOnce(&mut App)>>,
}

impl TerminalMount {
    pub fn new(view: AnyView, dispose: impl FnOnce(&mut App) + 'static) -> Self {
        Self {
            view,
            dispose: Some(Box::new(dispose)),
        }
    }

    /// 释放本 mount:只回收终端实体,不影响连接主会话。
    pub fn dispose(mut self, cx: &mut App) {
        if let Some(dispose) = self.dispose.take() {
            dispose(cx);
        }
    }
}

/// 终端宿主。实现方持有真实的终端实现(gpui 应用是单线程 UI 模型,
/// 这里不要求 `Send + Sync`)。
pub trait TerminalHost {
    /// 启动并挂载一个终端。失败时工作台渲染可读的错误而不是空白。
    fn mount(
        &self,
        request: TerminalMountRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<TerminalMount, TerminalMountError>;

    /// 处置一个挂载:回收终端实体与订阅。
    fn dispose(&self, mount: TerminalMount, cx: &mut App);
}

#[derive(Debug, thiserror::Error)]
pub enum TerminalMountError {
    #[error("terminal component is unavailable in this build")]
    Unavailable,
    #[error("terminal launch failed: {0}")]
    LaunchFailed(String),
}

/// 供宿主启动时注入的终端 host 全局。
pub struct GlobalTerminalHost {
    pub host: std::rc::Rc<dyn TerminalHost>,
}

impl gpui::Global for GlobalTerminalHost {}

/// 读取当前注入的终端 host(未注入时为 None)。
pub fn terminal_host(cx: &App) -> Option<std::rc::Rc<dyn TerminalHost>> {
    cx.try_global::<GlobalTerminalHost>()
        .map(|global| global.host.clone())
}

/// 按路由与 session metadata 插值命令行占位符。
///
/// 支持三种写法,前两种等价:`{{id}}`、`{{route.id}}` 取当前路由;
/// `{{session.docker_host}}` 取 resource/open metadata,由 provider 声明
/// 本连接的真实目标(如 Docker daemon socket),宿主据此保证终端与
/// 查询/操作命中同一 daemon。查不到的键原样保留(便于在界面上暴露
/// manifest 写错,而不是静默变成空串)。路由值按 JSON 语义转成字符串:
/// 字符串取原文,数字/布尔取字面量,对象/数组取紧凑 JSON,Null 视为未绑定。
pub fn interpolate_route(template: &str, route: &serde_json::Value) -> String {
    interpolate_with_session(template, route, &serde_json::Value::Null)
}

/// 同 [`interpolate_route`],叠加 session metadata 查找(`session.` 前缀)。
pub fn interpolate_with_session(
    template: &str,
    route: &serde_json::Value,
    session: &serde_json::Value,
) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            // 没有闭合括号:原样输出剩余内容。
            out.push_str(&rest[start..]);
            return out;
        };
        let key = after[..end].trim();
        let resolved = if let Some(lookup) = key.strip_prefix("session.") {
            session.get(lookup)
        } else {
            let lookup = key.strip_prefix("route.").unwrap_or(key);
            route.get(lookup)
        };
        match resolved {
            Some(serde_json::Value::Null) | None => {
                out.push_str(&rest[start..start + 2 + end + 2]);
            }
            Some(value) => out.push_str(&route_value_text(value)),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

fn route_value_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn route() -> serde_json::Value {
        json!({"id": "352ecf6ac7f7", "name": "headroom", "count": 3})
    }

    #[test]
    fn substitutes_bare_and_prefixed_keys() {
        assert_eq!(
            interpolate_route("exec -it {{id}} sh", &route()),
            "exec -it 352ecf6ac7f7 sh"
        );
        assert_eq!(
            interpolate_route("exec -it {{route.id}} sh", &route()),
            "exec -it 352ecf6ac7f7 sh"
        );
    }

    #[test]
    fn keeps_unknown_placeholder_verbatim() {
        assert_eq!(
            interpolate_route("use {{missing}}", &route()),
            "use {{missing}}"
        );
    }

    #[test]
    fn renders_non_string_values() {
        assert_eq!(interpolate_route("{{count}}", &route()), "3");
    }

    #[test]
    fn tolerates_unclosed_placeholder() {
        assert_eq!(interpolate_route("a {{id", &route()), "a {{id");
    }

    #[test]
    fn empty_route_leaves_template_untouched() {
        let template = "docker ps";
        assert_eq!(
            interpolate_route(template, &serde_json::Value::Null),
            template
        );
    }

    #[test]
    fn session_prefix_resolves_from_open_metadata() {
        let session = json!({"docker_host": "unix:///custom/docker.sock"});
        assert_eq!(
            interpolate_with_session("exec -it {{id}} sh", &route(), &session,),
            "exec -it 352ecf6ac7f7 sh"
        );
        assert_eq!(
            interpolate_with_session("DOCKER_HOST={{session.docker_host}}", &route(), &session,),
            "DOCKER_HOST=unix:///custom/docker.sock"
        );
    }

    #[test]
    fn missing_session_key_is_left_verbatim() {
        let session = json!({});
        assert_eq!(
            interpolate_with_session("{{session.docker_host}}", &route(), &session),
            "{{session.docker_host}}"
        );
    }
}
