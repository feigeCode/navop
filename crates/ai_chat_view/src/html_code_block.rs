use std::borrow::Cow;

use gpui::{
    AnyElement, App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, Styled, Window, div, prelude::FluentBuilder, px,
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
    view_id: SharedString,
    document: HtmlPreviewDocument,
    mode: HtmlCodeBlockMode,
    webview: Option<Entity<gpui_wry::WebView>>,
    webview_error: Option<String>,
    download_status: Option<String>,
}

impl HtmlCodeBlockView {
    pub fn new(
        view_id: impl Into<SharedString>,
        document: HtmlPreviewDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (webview, webview_error) = create_webview(&document, window, cx);
        let view = Self {
            view_id: view_id.into(),
            document,
            mode: HtmlCodeBlockMode::Preview,
            webview,
            webview_error,
            download_status: None,
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
        self.set_webview_visible(matches!(self.mode, HtmlCodeBlockMode::Preview), cx);
        cx.notify();
    }

    fn set_mode(&mut self, mode: HtmlCodeBlockMode, cx: &mut Context<Self>) {
        self.mode = mode;
        self.set_webview_visible(matches!(mode, HtmlCodeBlockMode::Preview), cx);
        cx.notify();
    }

    fn set_webview_visible(&mut self, visible: bool, cx: &mut App) {
        if let Some(webview) = &self.webview {
            webview.update(cx, |webview, _| {
                if visible {
                    webview.show();
                } else {
                    webview.hide();
                }
            });
        }
    }

    fn open_new_window(&self, cx: &mut App) {
        let view_id = SharedString::from(format!("{}-window", self.view_id));
        let document = self.document.clone();
        let _ = cx.open_window(Default::default(), move |window, cx| {
            let view =
                cx.new(|cx| HtmlCodeBlockView::new(view_id.clone(), document.clone(), window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        });
    }

    fn download_html(&mut self, cx: &mut Context<Self>) {
        match html_preview::download_html_preview_document(&self.document) {
            Ok(path) => {
                let filename = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("onetcli-html-preview.html");
                self.download_status = Some(format!("已下载到 Downloads/{filename}"));
                tracing::info!("HTML 预览已下载到 {}", path.display());
            }
            Err(error) => {
                self.download_status = Some("下载失败".to_string());
                tracing::warn!("HTML 预览下载失败: {error}");
            }
        }
        cx.notify();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .id("html-code-block-toolbar")
            .gap_1()
            .items_center()
            .child(self.preview_button(cx))
            .child(self.source_button(cx))
            .child(
                Clipboard::new(self.action_id("copy"))
                    .value(SharedString::from(self.document.source_html().to_string()))
                    .tooltip("复制 HTML"),
            )
            .child(self.download_button(cx))
            .child(self.open_window_button(cx))
            .when_some(self.download_status.clone(), |this, status| {
                this.child(
                    div()
                        .id(self.action_id("download-status"))
                        .ml_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(status),
                )
            })
            .into_any_element()
    }

    fn preview_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new(self.action_id("preview"))
            .icon(IconName::Eye)
            .ghost()
            .xsmall()
            .tooltip("预览")
            .on_click(cx.listener(|this, _, _, cx| this.set_mode(HtmlCodeBlockMode::Preview, cx)))
    }

    fn source_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new(self.action_id("source"))
            .icon(IconName::SquareTerminal)
            .ghost()
            .xsmall()
            .tooltip("源码")
            .on_click(cx.listener(|this, _, _, cx| this.set_mode(HtmlCodeBlockMode::Source, cx)))
    }

    fn open_window_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new(self.action_id("open-window"))
            .icon(IconName::ExternalLink)
            .ghost()
            .xsmall()
            .tooltip("打开新窗口")
            .on_click(cx.listener(|this, _, _, cx| this.open_new_window(cx)))
    }

    fn download_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new(self.action_id("download"))
            .icon(IconName::File)
            .ghost()
            .xsmall()
            .tooltip("下载 HTML")
            .on_click(cx.listener(|this, _, _, cx| this.download_html(cx)))
    }

    fn action_id(&self, action: &str) -> SharedString {
        SharedString::from(format!("{}-{action}", self.view_id))
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

    #[test]
    fn action_ids_are_scoped_to_view() {
        let view = HtmlCodeBlockView {
            view_id: SharedString::from("html-code-block-view-abc"),
            document: HtmlPreviewDocument::new("html", "<main>Original</main>"),
            mode: HtmlCodeBlockMode::Preview,
            webview: None,
            webview_error: None,
            download_status: None,
        };

        assert_eq!(
            SharedString::from("html-code-block-view-abc-source"),
            view.action_id("source")
        );
        assert_eq!(
            SharedString::from("html-code-block-view-abc-download"),
            view.action_id("download")
        );
    }
}
