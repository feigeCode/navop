use std::borrow::Cow;

use gpui::{
    AnyElement, App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{ActiveTheme, WindowExt};
use html_preview::{HtmlPreviewDocument, HtmlPreviewTransformOutput, resolve_extension_asset_url};
use rust_i18n::t;
use wry::WebViewBuilder;
use wry::http::{Request, Response, StatusCode};

pub struct HtmlCodeBlockView {
    document: HtmlPreviewDocument,
    action_status: Option<String>,
}

struct HtmlPreviewDialogView {
    document: HtmlPreviewDocument,
    webview: Option<Entity<gpui_wry::WebView>>,
    webview_error: Option<String>,
}

impl HtmlCodeBlockView {
    pub fn new(
        _view_id: impl Into<SharedString>,
        document: HtmlPreviewDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = Self {
            document,
            action_status: None,
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
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.document.apply_transform(transform);
        cx.notify();
    }

    pub(crate) fn open_preview_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let document = self.document.clone();
        let preview = cx.new(|cx| HtmlPreviewDialogView::new(document, window, cx));
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let preview = preview.clone();
            dialog
                .title(t!("HtmlPreview.dialog_title").to_string())
                .w(px(920.0))
                .content(move |content, _window, _cx| content.child(preview.clone()))
        });
        self.action_status = Some(t!("HtmlPreview.dialog_opened").to_string());
        cx.notify();
    }

    pub(crate) fn open_in_browser(&mut self, cx: &mut Context<Self>) {
        match html_preview::open_html_preview_document_in_browser(&self.document) {
            Ok(path) => {
                self.action_status = Some(t!("HtmlPreview.browser_opened").to_string());
                tracing::info!("HTML 预览已在浏览器打开: {}", path.display());
            }
            Err(error) => {
                self.action_status = Some(t!("HtmlPreview.browser_open_failed").to_string());
                tracing::warn!("HTML 预览打开浏览器失败: {error}");
            }
        }
        cx.notify();
    }

    pub(crate) fn download_html(&mut self, cx: &mut Context<Self>) {
        match html_preview::download_html_preview_document(&self.document) {
            Ok(path) => {
                let filename = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("onetcli-html-preview.html");
                self.action_status =
                    Some(t!("HtmlPreview.downloaded", filename = filename).to_string());
                tracing::info!("HTML 预览已下载到 {}", path.display());
            }
            Err(error) => {
                self.action_status = Some(t!("HtmlPreview.download_failed").to_string());
                tracing::warn!("HTML 预览下载失败: {error}");
            }
        }
        cx.notify();
    }

    #[cfg(test)]
    fn inline_preview_is_hidden(&self) -> bool {
        true
    }
}

impl Render for HtmlCodeBlockView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("html-code-block-preview-hidden")
            .hidden()
            .when_some(self.action_status.clone(), |this, status| {
                this.child(
                    div()
                        .id("html-code-block-action-status")
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(status),
                )
            })
    }
}

impl HtmlPreviewDialogView {
    fn new(document: HtmlPreviewDocument, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (webview, webview_error) = create_webview(&document, window, cx);
        let view = Self {
            document,
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
                    tracing::warn!("HTML 预览弹窗 transform 失败: {error}");
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

    fn render_preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let frame = div()
            .id("html-preview-dialog-webview")
            .h(px(560.0))
            .w_full()
            .overflow_hidden()
            .bg(cx.theme().background);
        if let Some(webview) = &self.webview {
            frame.child(webview.clone()).into_any_element()
        } else {
            frame
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(
                    self.webview_error
                        .clone()
                        .unwrap_or_else(|| t!("HtmlPreview.webview_unavailable").to_string()),
                )
                .into_any_element()
        }
    }

    #[cfg(test)]
    fn webview_html(&self) -> SharedString {
        SharedString::from(self.document.render_html().to_string())
    }
}

impl Render for HtmlPreviewDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("html-preview-dialog")
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .overflow_hidden()
            .child(self.render_preview(cx))
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
        Err(error) => (
            None,
            Some(t!("HtmlPreview.webview_create_failed", error = error).to_string()),
        ),
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
mod tests;
