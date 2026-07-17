use crate::{DocumentRenderRequest, DocumentRenderTheme, DocumentRendererRuntime};

#[test]
fn real_mermaid_component_renders_svg_when_fixture_is_provided() {
    let Ok(path) = std::env::var("NAVOP_MERMAID_COMPONENT") else {
        return;
    };
    let runtime = DocumentRendererRuntime::from_file("mermaid", path.as_ref()).unwrap();
    let output = futures::executor::block_on(runtime.render(DocumentRenderRequest {
        renderer: "mermaid".to_owned(),
        source: "flowchart LR\n  A --> B".to_owned(),
        theme: DocumentRenderTheme {
            dark: false,
            background: 0xf7f6f3,
            foreground: 0x37352f,
            border: 0xd8d8d6,
            muted: 0x9b9a97,
            accent: 0x2383e2,
            danger: 0xeb5757,
            font_family: "Inter, sans-serif".to_owned(),
        },
        available_width: 720.0,
        scale_factor: 1.0,
    }))
    .unwrap();
    assert_eq!("image/svg+xml", output.media_type);
    let svg = String::from_utf8(output.bytes).unwrap();
    assert!(svg.contains("<svg"));
    assert!(!svg.to_ascii_lowercase().contains("<foreignobject"));
}
