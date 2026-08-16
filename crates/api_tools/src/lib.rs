//! 接口工具：接口测试 + JSON 格式化（参考 verve 的实现移植，按 notes crate 方式集成）。
//!
//! - [`ApiTestView`]：目录/请求树，参数、路径变量、请求头、请求体、鉴权、Cookie、
//!   预执行脚本、Tests 脚本，以及响应体/响应头/Cookies 面板
//! - [`JsonFormatterView`]：可折叠树形 JSON 格式化器

rust_i18n::i18n!("locales", fallback = "en");

mod api_test_view;
mod collection_io;
mod grpc_web;
#[cfg(test)]
mod grpc_web_tests;
mod history;
mod http;
mod json_view;
mod mock;
mod mock_server;
mod multipart;
mod protocol;
mod request_debug;
mod request_store;
mod schema_io;
pub mod scripting;
mod socket_io;
#[cfg(test)]
mod socket_io_tests;
mod sse;
mod sse_parser;
#[cfg(test)]
mod sse_tests;
mod tab_content;
mod tcp;
#[cfg(test)]
mod tcp_tests;
#[cfg(test)]
mod tcp_transport_tests;
mod tree_model;
mod variable_resolver;
mod websocket;

#[cfg(test)]
mod websocket_tests;
#[cfg(test)]
mod websocket_transport_tests;

pub use api_test_view::ApiTestView;
pub use http::{HttpResponse, KeyValue, RequestMethod};
pub use json_view::JsonFormatterView;
pub use mock::{CompiledMockRule, MockRequestLike, MockRule, MockRuleSet, PathPattern, url_decode};
pub use mock_server::{MockRequestLog, MockServer, MockServerState};
pub use protocol::Protocol;
pub use request_store::StoredRequest;
pub use scripting::{ScriptResult, SideEffect, VarScope};

/// 初始化接口工具子系统（目前无全局状态，保留入口以便后续扩展）。
pub fn init(_cx: &mut gpui::App) {}

/// HTTP 方法的主题适配颜色（移植自 verve 的 `method_colors.rs`，Postman 配色）。
pub fn method_color(method: RequestMethod) -> gpui::Hsla {
    use gpui::hsla;
    let (h, s) = method_hue_and_saturation(method);
    hsla(h, s, 0.55, 1.0)
}

/// 用于浅色/深色主题中方法文字与描边徽标的高对比度颜色。
pub fn method_badge_color(method: RequestMethod, cx: &gpui::App) -> gpui::Hsla {
    use gpui::hsla;
    use gpui_component::ActiveTheme as _;

    let (h, s) = method_hue_and_saturation(method);
    let lightness = if cx.theme().is_dark() { 0.72 } else { 0.4 };
    hsla(h, s, lightness, 1.0)
}

/// 用于发送按钮实色背景的方法颜色。
pub fn method_fill_color(method: RequestMethod, cx: &gpui::App) -> gpui::Hsla {
    use gpui::hsla;
    use gpui_component::ActiveTheme as _;

    let (h, s) = method_hue_and_saturation(method);
    let lightness = if cx.theme().is_dark() { 0.56 } else { 0.46 };
    hsla(h, s, lightness, 1.0)
}

fn method_hue_and_saturation(method: RequestMethod) -> (f32, f32) {
    let (h, s) = match method {
        RequestMethod::Get => (0.33, 0.62),     // green
        RequestMethod::Post => (0.10, 0.78),    // amber/orange
        RequestMethod::Put => (0.58, 0.62),     // blue
        RequestMethod::Patch => (0.13, 0.65),   // yellow-amber
        RequestMethod::Delete => (0.00, 0.72),  // red
        RequestMethod::Head => (0.72, 0.18),    // gray-purple
        RequestMethod::Options => (0.78, 0.45), // purple
        RequestMethod::Trace => (0.52, 0.38),   // cyan-blue
    };
    (h, s)
}
