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
fn preview_is_hidden_initially() {
    let view = HtmlCodeBlockView {
        document: HtmlPreviewDocument::new("html", "<main>Original</main>"),
        action_status: None,
    };

    assert!(view.inline_preview_is_hidden());
}

#[test]
fn dialog_webview_content_uses_render_html() {
    let mut document = HtmlPreviewDocument::new("html", "<main>Original</main>");
    document.apply_transform(HtmlPreviewTransformOutput {
        html: "<section>Rendered</section>".to_string(),
        assets: vec![],
    });
    let view = HtmlPreviewDialogView {
        document,
        webview: None,
        webview_error: None,
    };

    let preview_html = view.webview_html();

    assert!(preview_html.contains("<body><section>Rendered</section></body>"));
    assert!(!preview_html.contains("<main>Original</main>"));
}
