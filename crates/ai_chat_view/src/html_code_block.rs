use std::borrow::Cow;

use gpui::{
    AnyElement, App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, IconName, Root, Sizable,
    button::{Button, ButtonVariants},
    clipboard::Clipboard,
    h_flex,
};
use html_preview::{HtmlPreviewDocument, HtmlPreviewTransformOutput, resolve_extension_asset_url};
use wry::WebViewBuilder;
use wry::http::{Request, Response, StatusCode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HtmlCodeBlockMode {
    Preview,
    Source,
}

pub struct HtmlCodeBlockView {
    document: HtmlPreviewDocument,
    mode: HtmlCodeBlockMode,
    webview: Option<Entity<gpui_wry::WebView>>,
    webview_error: Option<String>,
}

impl HtmlCodeBlockView {
    pub fn new(document: HtmlPreviewDocument, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (webview, webview_error) = create_webview(&document, window, cx);
        let view = Self {
            document,
            mode: HtmlCodeBlockMode::Preview,
            webview,
            webview_error,
        };
        view.spawn_transform(window, cx);
        view
    }

    fn spawn_transform(&self, window: &mut Window, cx: &mut Context<Self>) {
        let language = self.document.language().to_string();
        let source_html = self.document.source_html().to_string();
        cx.spawn_in(
            window,
            async move |this, cx| match html_preview::transform_html_preview(language, source_html)
                .await
            {
                Ok(Some(transform)) => {
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.apply_transform(transform, window, cx);
                    });
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!("HTML 预览 transform 失败: {error}");
                }
            },
        )
        .detach();
    }

    fn apply_transform(
        &mut self,
        transform: HtmlPreviewTransformOutput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.document.apply_transform(transform);
        let (webview, webview_error) = create_webview(&self.document, window, cx);
        self.webview = webview;
        self.webview_error = webview_error;
        cx.notify();
    }

    fn set_mode(&mut self, mode: HtmlCodeBlockMode, cx: &mut Context<Self>) {
        self.mode = mode;
        cx.notify();
    }

    fn open_new_window(&self, cx: &mut App) {
        let document = self.document.clone();
        let _ = cx.open_window(Default::default(), move |window, cx| {
            let view = cx.new(|cx| HtmlCodeBlockView::new(document.clone(), window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        });
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .id("html-code-block-toolbar")
            .gap_1()
            .items_center()
            .child(self.preview_button(cx))
            .child(self.source_button(cx))
            .child(
                Clipboard::new("html-copy")
                    .value(SharedString::from(self.document.source_html().to_string()))
                    .tooltip("复制 HTML"),
            )
            .child(self.open_window_button(cx))
            .into_any_element()
    }

    fn preview_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new("html-preview")
            .icon(IconName::Eye)
            .ghost()
            .xsmall()
            .tooltip("预览")
            .on_click(cx.listener(|this, _, _, cx| this.set_mode(HtmlCodeBlockMode::Preview, cx)))
    }

    fn source_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new("html-source")
            .icon(IconName::SquareTerminal)
            .ghost()
            .xsmall()
            .tooltip("源码")
            .on_click(cx.listener(|this, _, _, cx| this.set_mode(HtmlCodeBlockMode::Source, cx)))
    }

    fn open_window_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new("html-open-window")
            .icon(IconName::ExternalLink)
            .ghost()
            .xsmall()
            .tooltip("打开新窗口")
            .on_click(cx.listener(|this, _, _, cx| this.open_new_window(cx)))
    }

    fn render_preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let frame = div()
            .id("html-code-block-webview-frame")
            .h(px(260.))
            .w_full()
            .bg(cx.theme().background);
        if let Some(webview) = &self.webview {
            frame.child(webview.clone()).into_any_element()
        } else {
            frame
                .p_3()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(
                    self.webview_error.clone().unwrap_or_else(|| {
                        "无法创建 HTML 预览 webview，已保留源码视图。".to_string()
                    }),
                )
                .into_any_element()
        }
    }

    fn render_source(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("html-code-block-source")
            .p_3()
            .font_family(cx.theme().mono_font_family.clone())
            .text_size(cx.theme().mono_font_size)
            .text_color(cx.theme().foreground)
            .bg(cx.theme().muted)
            .child(self.document.source_html().to_string())
            .into_any_element()
    }
}

impl Render for HtmlCodeBlockView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("html-code-block")
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .overflow_hidden()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .child(self.render_toolbar(cx)),
            )
            .child(match self.mode {
                HtmlCodeBlockMode::Preview => self.render_preview(cx),
                HtmlCodeBlockMode::Source => self.render_source(cx),
            })
    }
}

fn create_webview(
    document: &HtmlPreviewDocument,
    window: &mut Window,
    cx: &mut App,
) -> (Option<Entity<gpui_wry::WebView>>, Option<String>) {
    match WebViewBuilder::new()
        .with_custom_protocol("onet-extension".to_string(), |_id, request| {
            extension_asset_response(request)
        })
        .with_html(document.render_html().to_string())
        .build_as_child(window)
    {
        Ok(webview) => (
            Some(cx.new(|cx| gpui_wry::WebView::new(webview, window, cx))),
            None,
        ),
        Err(error) => (None, Some(format!("创建 HTML 预览 webview 失败: {error}"))),
    }
}

fn extension_asset_response(request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let Some(path) = resolve_extension_asset_url(&request.uri().to_string()) else {
        return empty_response(StatusCode::NOT_FOUND);
    };
    match std::fs::read(&path) {
        Ok(bytes) => response(StatusCode::OK, mime_for_path(&path), Cow::Owned(bytes)),
        Err(_) => empty_response(StatusCode::NOT_FOUND),
    }
}

fn empty_response(status: StatusCode) -> Response<Cow<'static, [u8]>> {
    response(status, "text/plain; charset=utf-8", Cow::Borrowed(&[]))
}

fn response(
    status: StatusCode,
    content_type: &'static str,
    body: Cow<'static, [u8]>,
) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header("content-type", content_type)
        .body(body)
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[])))
}

fn mime_for_path(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("json") => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use html_preview::HtmlPreviewTransformOutput;

    #[test]
    fn transform_output_updates_preview_without_replacing_source() {
        let mut document = HtmlPreviewDocument::new("html", "<main>Original");

        document.apply_transform(HtmlPreviewTransformOutput {
            html: "<section>Changed</section>".to_string(),
            assets: vec![],
        });

        assert_eq!("<main>Original", document.source_html());
        assert!(
            document
                .render_html()
                .contains("<body><section>Changed</section></body>")
        );
    }
}
