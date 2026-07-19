use crate::{DocumentExportRequest, DocumentExportTheme, DocumentExporterRuntime};

#[test]
fn installed_notes_exporter_component_exports_all_formats_when_fixture_is_provided() {
    let Ok(path) = std::env::var("NAVOP_NOTES_EXPORTER_WASM") else {
        return;
    };
    let runtime = DocumentExporterRuntime::from_file("notes-documents", path.as_ref()).unwrap();
    for (format, media_type, signature) in [
        ("html", "text/html", b"<!doctype html".as_slice()),
        ("pdf", "application/pdf", b"%PDF-1.7".as_slice()),
        (
            "docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            b"PK".as_slice(),
        ),
    ] {
        let output = futures::executor::block_on(runtime.export(DocumentExportRequest {
            exporter: "notes-documents".to_owned(),
            format: format.to_owned(),
            title: "导出测试".to_owned(),
            source: "# 标题\n\n正文".to_owned(),
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
