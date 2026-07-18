use futures::FutureExt;
use html_preview::{
    HtmlPreviewAction, HtmlPreviewAssetResolver, HtmlPreviewDocument, HtmlPreviewTransformOutput,
    clear_html_preview_transform_provider, normalize_html_document, register_extension_asset_root,
    resolve_extension_asset_url, set_html_preview_transform_provider, system_browser_open_command,
    transform_html_preview_document, write_browser_html_preview_document_to_dir,
    write_html_preview_document_to_dir,
};

#[test]
fn normalizes_body_fragment_into_complete_document() {
    let normalized = normalize_html_document("<h1>Hello</h1><script src=\"app.js\"></script>");

    assert!(normalized.starts_with("<!doctype html>"));
    assert!(normalized.contains("<html"));
    assert!(normalized.contains("<head>"));
    assert!(normalized.contains("<body><h1>Hello</h1><script src=\"app.js\"></script></body>"));
    assert!(normalized.ends_with("</html>"));
}

#[test]
fn preserves_existing_complete_document() {
    let source = "<!doctype html><html><head><title>A</title></head><body>B</body></html>";

    assert_eq!(source, normalize_html_document(source));
}

#[test]
fn completes_missing_basic_tags_without_dropping_head_content() {
    let source = "<title>A</title></head><body><main>Partial";
    let normalized = normalize_html_document(source);

    assert!(normalized.contains("<head><title>A</title></head>"));
    assert!(normalized.contains("<body><main>Partial</main></body>"));
}

#[test]
fn preview_document_runs_transforms_before_normalization() {
    let transform = HtmlPreviewTransformOutput {
        html: "<section>Changed</section>".to_string(),
        assets: vec![],
    };

    let document = HtmlPreviewDocument::from_transform("html", transform);

    assert_eq!("html", document.language());
    assert!(document.source_html().contains("Changed"));
    assert!(
        document
            .render_html()
            .contains("<body><section>Changed</section></body>")
    );
}

#[test]
fn transform_provider_builds_document_before_normalization() {
    clear_html_preview_transform_provider();
    set_html_preview_transform_provider(|language, html| {
        assert_eq!("html", language);
        assert_eq!("<main>Original", html);
        async move {
            Ok(Some(HtmlPreviewTransformOutput {
                html: "<section>Changed</section>".to_string(),
                assets: vec![html_preview::HtmlPreviewAsset {
                    path: "app.css".to_string(),
                    url: "onet-extension://com.example.preview/app.css".to_string(),
                }],
            }))
        }
        .boxed()
    });

    let document =
        futures::executor::block_on(transform_html_preview_document("html", "<main>Original"))
            .unwrap();

    assert_eq!("html", document.language());
    assert_eq!("<main>Original", document.source_html());
    assert!(
        document
            .render_html()
            .contains("<body><section>Changed</section></body>")
    );
    assert_eq!(1, document.assets().len());

    clear_html_preview_transform_provider();
}

#[test]
fn applying_transform_preserves_original_source_html() {
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
fn asset_resolver_rewrites_extension_asset_urls_and_rejects_escape() {
    let resolver =
        HtmlPreviewAssetResolver::new("com.example.preview", "/extensions/com.example.preview");

    assert_eq!(
        "onet-extension://com.example.preview/assets/app.css",
        resolver.resolve("assets/app.css").unwrap()
    );
    assert!(resolver.resolve("../secret.css").is_none());
    assert!(resolver.resolve("https://example.com/app.css").is_none());
}

#[test]
fn registered_extension_asset_roots_resolve_safe_protocol_urls() {
    let assets_dir = tempfile::TempDir::new().unwrap();
    let extension_id = "com.example.preview.registered-root";
    register_extension_asset_root(extension_id, assets_dir.path());

    assert_eq!(
        assets_dir.path().join("app.css"),
        resolve_extension_asset_url(&format!("onet-extension://{extension_id}/app.css")).unwrap()
    );
    assert!(
        resolve_extension_asset_url(&format!("onet-extension://{extension_id}/../secret"))
            .is_none()
    );
    assert!(resolve_extension_asset_url(&format!("https://{extension_id}/app.css")).is_none());
}

#[test]
fn writes_complete_html_preview_document_without_overwriting_existing_file() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("onetcli-html-preview.html"), "existing").unwrap();
    let document = HtmlPreviewDocument::new("html", "<main>Download</main>");

    let path = write_html_preview_document_to_dir(&document, dir.path()).unwrap();

    assert_eq!(dir.path().join("onetcli-html-preview-1.html"), path);
    let saved = std::fs::read_to_string(path).unwrap();
    assert!(saved.starts_with("<!doctype html>"));
    assert!(saved.contains("<body><main>Download</main></body>"));
    assert_eq!(
        "existing",
        std::fs::read_to_string(dir.path().join("onetcli-html-preview.html")).unwrap()
    );
}

#[test]
fn writes_browser_ready_html_with_extension_assets_rewritten_to_file_urls() {
    let assets_dir = tempfile::TempDir::new().unwrap();
    std::fs::write(assets_dir.path().join("app.css"), "body{}").unwrap();
    std::fs::write(assets_dir.path().join("chart.js"), "window.loaded=true").unwrap();
    register_extension_asset_root("com.example.preview", assets_dir.path());
    let document = HtmlPreviewDocument::from_transform(
        "html",
        HtmlPreviewTransformOutput {
            html: r#"<link rel="stylesheet" href="onet-extension://com.example.preview/app.css"><script src="onet-extension://com.example.preview/chart.js"></script>"#.to_string(),
            assets: vec![
                html_preview::HtmlPreviewAsset {
                    path: "app.css".to_string(),
                    url: "onet-extension://com.example.preview/app.css".to_string(),
                },
                html_preview::HtmlPreviewAsset {
                    path: "chart.js".to_string(),
                    url: "onet-extension://com.example.preview/chart.js".to_string(),
                },
            ],
        },
    );
    let output_dir = tempfile::TempDir::new().unwrap();

    let path = write_browser_html_preview_document_to_dir(&document, output_dir.path()).unwrap();

    let saved = std::fs::read_to_string(path).unwrap();
    assert!(!saved.contains("onet-extension://"));
    assert!(saved.contains("href=\"file://"));
    assert!(saved.contains("src=\"file://"));
    assert!(saved.contains("app.css"));
    assert!(saved.contains("chart.js"));
}

#[test]
fn system_browser_open_command_targets_platform_default_opener() {
    let command = system_browser_open_command(std::path::Path::new("/tmp/onetcli-preview.html"));

    if cfg!(target_os = "macos") {
        assert_eq!("open", command.program);
        assert_eq!(vec!["/tmp/onetcli-preview.html"], command.args);
    } else if cfg!(target_os = "windows") {
        assert_eq!("cmd", command.program);
        assert_eq!(
            vec!["/C", "start", "", "/tmp/onetcli-preview.html"],
            command.args
        );
    } else {
        assert_eq!("xdg-open", command.program);
        assert_eq!(vec!["/tmp/onetcli-preview.html"], command.args);
    }
}

#[test]
fn action_ids_are_stable_and_ordered_for_html_toolbar() {
    assert_eq!(
        vec![
            "html-preview",
            "html-source",
            "html-copy",
            "html-download",
            "html-open-window"
        ],
        HtmlPreviewAction::toolbar_ids()
    );
}
