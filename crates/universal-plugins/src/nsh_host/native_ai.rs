//! `navop.ai` 原生组件：把 AI 聊天面板嵌进扩展页面。
//!
//! 宿主只提供组件本身，**位置和尺寸由页面决定**（放右栏、放抽屉、放弹窗都行）：
//!
//! ```tsx
//! const Ai = nativeComponent("navop.ai");
//! <View style={{ width: 360, height: "100%" }}>
//!   <Ai context={{ route: "containers", container: id }} />
//! </View>
//! ```
//!
//! `context` 是可选 prop：对象直接给，字符串按 JSON 解析。宿主把它当参考塞进
//! 系统指令（带注入防护），不做数据授权——AI 能干什么由 AI 工具与连接权限决定。

use std::{
    rc::Rc,
    sync::{Arc, Mutex, OnceLock},
};

use gpui::{App, AppContext as _, IntoElement, ParentElement, Styled, Window, div};
use nsh_platform_gpui::{ComponentContext, NativeComponent};
use serde_json::Value;

use super::native::{NativeViewContext, register_native_component};
use super::plugin_ai_panel::PluginAiPanel;
use crate::universal_plugins::UniversalPluginService;

/// 页面里 `nativeComponent("navop.ai")` 引用的名字。
pub(crate) const AI: &str = "navop.ai";

/// 已挂载的 AI 面板（扩展 id → 弱引用）。`navop.ai.context` 靠它把上下文
/// 转给同扩展的实例；页面没挂组件时表里没有条目，上下文自然丢弃。
fn panels() -> &'static Mutex<Vec<(String, gpui::WeakEntity<PluginAiPanel>)>> {
    static PANELS: OnceLock<Mutex<Vec<(String, gpui::WeakEntity<PluginAiPanel>)>>> =
        OnceLock::new();
    PANELS.get_or_init(|| Mutex::new(Vec::new()))
}

/// 把页面上下文转给该扩展已挂载的所有 AI 面板（主线程）。
pub(crate) fn publish_context(extension_id: &str, context: Value, cx: &mut App) {
    let targets: Vec<gpui::Entity<PluginAiPanel>> = {
        let mut registered = panels().lock().unwrap_or_else(|e| e.into_inner());
        registered.retain(|(_, panel)| panel.upgrade().is_some());
        registered
            .iter()
            .filter(|(id, _)| id == extension_id)
            .filter_map(|(_, panel)| panel.upgrade())
            .collect()
    };
    for panel in targets {
        panel.update(cx, |panel, cx| panel.set_page_context(context.clone(), cx));
    }
}

/// 注册组件（`NshHost` 初始化时调一次，闭包里钉住服务句柄）。
pub(crate) fn install(service: UniversalPluginService, cx: &mut App) {
    register_native_component(
        AI,
        Rc::new(move |context: NativeViewContext| {
            Box::new(AiChatComponent::new(service.clone(), context)) as Box<dyn NativeComponent>
        }),
        cx,
    );
}

struct AiChatComponent {
    service: UniversalPluginService,
    component: ComponentContext,
    extension_id: Arc<str>,
    route: Option<Arc<str>>,
    chat: Option<gpui::Entity<PluginAiPanel>>,
    /// 页面给过 `context` prop 但面板还没建：建完再补一次。
    pending_context: Option<Value>,
    error: Option<String>,
}

impl AiChatComponent {
    fn new(service: UniversalPluginService, context: NativeViewContext) -> Self {
        Self {
            service,
            component: context.component,
            extension_id: context.extension_id,
            route: context.route,
            chat: None,
            pending_context: None,
            error: None,
        }
    }

    /// 建面板（只建一次）。标题从 catalog 取，取不到就用扩展 id。
    fn ensure_started(&mut self, window: &mut Window, cx: &mut App) {
        if self.chat.is_some() || self.error.is_some() {
            return;
        }
        let title: Arc<str> = self
            .service
            .shell_app(self.extension_id.as_ref())
            .map(|app| Arc::<str>::from(app.title.as_str()))
            .unwrap_or_else(|| Arc::clone(&self.extension_id));
        let service = self.service.clone();
        let extension_id = Arc::clone(&self.extension_id);
        let route = self.route.clone();
        let chat = cx.new(|cx| {
            PluginAiPanel::new_embedded(&service, extension_id, title, route, None, window, cx)
        });
        if let Some(context) = self.pending_context.take() {
            chat.update(cx, |panel, cx| panel.set_page_context(context, cx));
        }
        self.chat = Some(chat.clone());
        // 登记给 `navop.ai.context` 用；面板随组件销毁，下一轮会剔除。
        {
            let mut registered = panels().lock().unwrap_or_else(|e| e.into_inner());
            registered.retain(|(_, panel)| panel.upgrade().is_some());
            registered.push((self.extension_id.to_string(), chat.downgrade()));
        }
        self.component.redraw();
    }
}

impl NativeComponent for AiChatComponent {
    fn render(&mut self, window: &mut Window, cx: &mut App) -> Option<gpui::AnyElement> {
        self.ensure_started(window, cx);
        if let Some(chat) = &self.chat {
            return Some(
                div()
                    .size_full()
                    .child(chat.clone())
                    .into_any_element(),
            );
        }
        let message = self
            .error
            .clone()
            .unwrap_or_else(|| "AI 面板准备中…".to_owned());
        Some(
            div()
                .p_3()
                .text_sm()
                .child(gpui::SharedString::from(message))
                .into_any_element(),
        )
    }

    fn set_prop(
        &mut self,
        name: &str,
        value: Option<&Value>,
        _window: &mut Window,
        cx: &mut App,
    ) {
        if name != "context" {
            return;
        }
        let Some(context) = value else {
            return;
        };
        // 对象直接收；字符串按 JSON 解析（页面里写成模板字符串比较方便）。
        let parsed = match context {
            Value::String(raw) => match serde_json::from_str::<Value>(raw) {
                Ok(parsed) => parsed,
                Err(error) => {
                    self.error = Some(format!("context 不是合法 JSON：{error}"));
                    self.component.redraw();
                    return;
                }
            },
            other => other.clone(),
        };
        self.error = None;
        match &self.chat {
            Some(chat) => {
                chat.update(cx, |panel, cx| panel.set_page_context(parsed, cx));
            }
            None => self.pending_context = Some(parsed),
        }
        self.component.redraw();
    }
}
