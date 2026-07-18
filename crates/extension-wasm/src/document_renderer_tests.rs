use crate::{DocumentRenderRequest, DocumentRenderTheme, DocumentRendererRuntime};

#[test]
fn real_mermaid_component_renders_svg_when_fixture_is_provided() {
    let Ok(path) = std::env::var("NAVOP_MERMAID_COMPONENT") else {
        return;
    };
    let runtime = DocumentRendererRuntime::from_file("mermaid", path.as_ref()).unwrap();
    let output = futures::executor::block_on(runtime.render(DocumentRenderRequest {
        renderer: "mermaid".to_owned(),
        source: "graph TD\n  A[开始] --> B[处理]\n  B --> C[结束]".to_owned(),
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

#[test]
fn real_math_component_renders_visible_svg_paths_when_fixture_is_provided() {
    let Ok(path) = std::env::var("NAVOP_MATH_COMPONENT") else {
        return;
    };
    let runtime = DocumentRendererRuntime::from_file("math", path.as_ref()).unwrap();
    let output = futures::executor::block_on(runtime.render(DocumentRenderRequest {
        renderer: "math".to_owned(),
        source: r"\frac{-b \pm \sqrt{b^2 - 4ac}}{2a}".to_owned(),
        theme: DocumentRenderTheme {
            dark: false,
            background: 0xffffff,
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
    assert!(
        svg.contains("<path"),
        "embedded math fonts must produce visible glyph paths"
    );
    assert!(
        svg.contains("<rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>"),
        "math SVG must paint the requested theme background"
    );
    assert!(!svg.to_ascii_lowercase().contains("<foreignobject"));
}
