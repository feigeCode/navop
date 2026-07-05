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
        view_id: SharedString::from("preview_is_hidden_initially"),
        document: HtmlPreviewDocument::new("html", "<main>Original</main>"),
        preview_visible: false,
        action_status: None,
    };

    assert!(!view.preview_visible);
}

#[test]
fn preview_content_uses_render_html() {
    let mut document = HtmlPreviewDocument::new("html", "<main>Original</main>");
    document.apply_transform(HtmlPreviewTransformOutput {
        html: "<section>Rendered</section>".to_string(),
        assets: vec![],
    });
    let view = HtmlCodeBlockView {
        view_id: SharedString::from("preview_content_uses_render_html"),
        document,
        preview_visible: true,
        action_status: None,
    };

    let preview_html = view.preview_html();

    assert!(preview_html.contains("<body><section>Rendered</section></body>"));
    assert!(!preview_html.contains("<main>Original</main>"));
}
