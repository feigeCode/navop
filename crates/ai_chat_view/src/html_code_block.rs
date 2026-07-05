use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::ActiveTheme;
use gpui_component::text::TextView;
use html_preview::{HtmlPreviewDocument, HtmlPreviewTransformOutput};

pub struct HtmlCodeBlockView {
    view_id: SharedString,
    document: HtmlPreviewDocument,
    preview_visible: bool,
    action_status: Option<String>,
}

impl HtmlCodeBlockView {
    pub fn new(
        view_id: impl Into<SharedString>,
        document: HtmlPreviewDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = Self {
            view_id: view_id.into(),
            document,
            preview_visible: false,
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

    pub(crate) fn toggle_preview(&mut self, cx: &mut Context<Self>) {
        self.preview_visible = !self.preview_visible;
        cx.notify();
    }

    pub(crate) fn open_in_browser(&mut self, cx: &mut Context<Self>) {
        match html_preview::open_html_preview_document_in_browser(&self.document) {
            Ok(path) => {
                self.action_status = Some("已在浏览器打开".to_string());
                tracing::info!("HTML 预览已在浏览器打开: {}", path.display());
            }
            Err(error) => {
                self.action_status = Some("打开浏览器失败".to_string());
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
                self.action_status = Some(format!("已下载到 Downloads/{filename}"));
                tracing::info!("HTML 预览已下载到 {}", path.display());
            }
            Err(error) => {
                self.action_status = Some("下载失败".to_string());
                tracing::warn!("HTML 预览下载失败: {error}");
            }
        }
        cx.notify();
    }

    fn render_preview_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("html-code-block-preview-panel")
            .relative()
            .min_h(px(420.0))
            .max_h(px(640.0))
            .overflow_y_scroll()
            .bg(cx.theme().background)
            .p_3()
            .when_some(self.action_status.clone(), |this, status| {
                this.child(
                    div()
                        .id("html-code-block-action-status")
                        .absolute()
                        .top_2()
                        .right_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(status),
                )
            })
            .child(
                TextView::html(
                    SharedString::from(format!("{}/preview-content", self.view_id)),
                    self.preview_html(),
                )
                .selectable(true),
            )
    }

    fn preview_html(&self) -> SharedString {
        SharedString::from(self.document.render_html().to_string())
    }
}

impl Render for HtmlCodeBlockView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.preview_visible {
            return div().id("html-code-block-preview-hidden").hidden();
        }
        div()
            .id("html-code-block")
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .overflow_hidden()
            .child(self.render_preview_panel(cx))
    }
}

#[cfg(test)]
mod tests;
