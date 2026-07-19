use crate::{
    DocumentExportAsset, DocumentExportRequest, DocumentExportTheme, DocumentExporterRuntime,
};

#[test]
fn installed_notes_exporter_components_export_their_formats_when_provided() {
    for (environment, exporter, format, media_type, signature) in [
        (
            "NAVOP_NOTES_HTML_EXPORTER_WASM",
            "notes-html",
            "html",
            "text/html",
            b"<!doctype html".as_slice(),
        ),
        (
            "NAVOP_NOTES_PDF_EXPORTER_WASM",
            "notes-pdf",
            "pdf",
            "application/pdf",
            b"%PDF-1.7".as_slice(),
        ),
        (
            "NAVOP_NOTES_WORD_EXPORTER_WASM",
            "notes-word",
            "docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            b"PK".as_slice(),
        ),
    ] {
        let Ok(path) = std::env::var(environment) else {
            continue;
        };
        let runtime = DocumentExporterRuntime::from_file(exporter, path.as_ref()).unwrap();
        let (source, assets) = if format == "pdf" {
            (
                "![Board](board.svg)".to_owned(),
                vec![DocumentExportAsset {
                    path: "board.svg".to_owned(),
                    media_type: "image/svg+xml".to_owned(),
                    bytes: br#"<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="1200"><rect width="1600" height="1200" fill="red"/></svg>"#.to_vec(),
                }],
            )
        } else {
            ("# 标题\n\n正文".to_owned(), Vec::new())
        };
        let output = futures::executor::block_on(runtime.export(DocumentExportRequest {
            exporter: exporter.to_owned(),
            format: format.to_owned(),
            title: "导出测试".to_owned(),
            source,
            assets,
            theme: DocumentExportTheme {
                dark: false,
                background: 0xffffff,
                foreground: 0x222222,
                border: 0xdddddd,
                muted: 0x777777,
                accent: 0x2563eb,
                danger: 0xdc2626,
                font_family: String::new(),
            },
        }))
        .unwrap();

        assert_eq!(media_type, output.media_type);
        assert!(output.bytes.starts_with(signature));
    }
}
