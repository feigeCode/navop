use std::{fs, path::PathBuf, process::Command};

use db::GlobalDbState;
use db_view::extension_menu::DbTreeExtensionActionContext;
use one_core::storage::DatabaseType;

use crate::ExtensionRuntimeCatalog;

#[test]
fn runtime_catalog_loads_wasm_coverage_fixture_manifest() {
    let fixture_dir = coverage_fixture_dir();
    let manifest = crate::extension::manifest::load_from_dir(&fixture_dir).unwrap();

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let registry = catalog.db_tree_menu_registry();
    let table_items = registry.items_for_node(db::DbNodeType::Table);

    assert!(
        catalog
            .component_permissions_for_command("coverage.run")
            .is_ok()
    );
    assert_eq!(1, table_items.len());
    assert_eq!("coverage.run", table_items[0].command_id);
}

#[test]
fn db_tree_action_runs_registered_wasm_component() {
    let fixture_dir = coverage_fixture_dir();
    let manifest = crate::extension::manifest::load_from_dir(&fixture_dir).unwrap();
    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();

    futures::executor::block_on(catalog.run_db_tree_component_action(
        DbTreeExtensionActionContext {
            extension_id: "com.onetcli.coverage-wasm".to_string(),
            command_id: "coverage.run".to_string(),
            node_id: "table-1".to_string(),
            node_name: "users".to_string(),
            node_type: db::DbNodeType::Table,
            database_type: DatabaseType::PostgreSQL,
            connection_id: "conn1".to_string(),
        },
        GlobalDbState::new(),
    ))
    .unwrap();
}

#[test]
fn html_preview_transform_runs_registered_wasm_component() {
    let root = tempfile::TempDir::new().unwrap();
    let extension_dir = root.path().join("com.example.html");
    let wasm_dir = extension_dir.join("wasm");
    fs::create_dir_all(&wasm_dir).unwrap();
    fs::write(
        wasm_dir.join("plugin.wasm"),
        html_preview_transform_component_bytes(),
    )
    .unwrap();
    fs::write(
        extension_dir.join("extension.json"),
        r#"{
            "schema_version": 1,
            "id": "com.example.html",
            "name": "HTML Preview",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.1.0" },
            "runtime": {
                "wasm": [{
                    "id": "main",
                    "module": "./wasm/plugin.wasm",
                    "kind": "component"
                }]
            },
            "contributes": {
                "htmlPreviewTransforms": [{
                    "id": "html.decorate",
                    "runtimeId": "main",
                    "languages": ["html"],
                    "assets": "assets"
                }]
            }
        }"#,
    )
    .unwrap();

    let manifest = crate::extension::manifest::load_from_dir(&extension_dir).unwrap();
    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let result = futures::executor::block_on(catalog.transform_html_preview("html", "<main>Hi"))
        .unwrap()
        .unwrap();

    assert_eq!("<main>Changed</main>", result.html);
}

#[test]
fn installed_mermaid_extension_registers_and_renders_when_fixture_is_provided() {
    let Ok(dir) = std::env::var("NAVOP_INSTALLED_MERMAID_EXTENSION") else {
        return;
    };
    let manifest = crate::extension::manifest::load_from_dir(std::path::Path::new(&dir)).unwrap();
    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let renderer = catalog.document_renderer_for_kind("mermaid").unwrap();
    assert_eq!("Mermaid", renderer.display_name);
    catalog.prewarm_document_renderers().unwrap();
    assert_eq!(
        1,
        catalog.document_renderer_runtimes.lock().unwrap().len(),
        "renderer prewarming must retain the compiled component"
    );
    let output = futures::executor::block_on(catalog.render_document(
        extension_wasm::DocumentRenderRequest {
            renderer: "mermaid".to_owned(),
            source: "graph TD\n A[开始] --> B[处理]\n B --> C[结束]".to_owned(),
            theme: extension_wasm::DocumentRenderTheme {
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
        },
    ))
    .unwrap()
    .unwrap();
    assert_eq!("image/svg+xml", output.media_type);
    assert!(String::from_utf8(output.bytes).unwrap().contains("<svg"));
    assert_eq!(1, catalog.document_renderer_runtimes.lock().unwrap().len());
}

#[test]
fn installed_math_extension_registers_and_renders_when_fixture_is_provided() {
    let Ok(dir) = std::env::var("NAVOP_INSTALLED_MATH_EXTENSION") else {
        return;
    };
    let manifest = crate::extension::manifest::load_from_dir(std::path::Path::new(&dir)).unwrap();
    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    assert_eq!(
        "Math",
        catalog
            .document_renderer_for_kind("math")
            .unwrap()
            .display_name
    );
    let output = futures::executor::block_on(catalog.render_document(
        extension_wasm::DocumentRenderRequest {
            renderer: "math".to_owned(),
            source: r"\frac{a}{b}".to_owned(),
            theme: extension_wasm::DocumentRenderTheme {
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
        },
    ))
    .unwrap()
    .unwrap();
    let svg = String::from_utf8(output.bytes).unwrap();
    assert_eq!("image/svg+xml", output.media_type);
    assert!(svg.contains("<path"));
}

#[test]
fn installed_notes_exporter_registers_and_exports_all_formats_when_fixture_is_provided() {
    let Ok(dir) = std::env::var("NAVOP_INSTALLED_NOTES_EXPORTER_EXTENSION") else {
        return;
    };
    let manifest = crate::extension::manifest::load_from_dir(std::path::Path::new(&dir)).unwrap();
    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();

    assert_eq!(
        "notes-documents",
        catalog.document_exporter_for_format("docx").unwrap().id
    );
    catalog.prewarm_document_exporters().unwrap();
    for (format, signature) in [
        ("html", b"<!doctype html".as_slice()),
        ("pdf", b"%PDF-1.7".as_slice()),
        ("docx", b"PK".as_slice()),
    ] {
        let output = futures::executor::block_on(catalog.export_document(
            extension_wasm::DocumentExportRequest {
                exporter: String::new(),
                format: format.to_owned(),
                title: "导出测试".to_owned(),
                source: "# 标题\n\n正文".to_owned(),
                assets: Vec::new(),
                theme: extension_wasm::DocumentExportTheme {
                    dark: false,
                    background: 0xffffff,
                    foreground: 0x222222,
                    border: 0xdddddd,
                    muted: 0x777777,
                    accent: 0x2563eb,
                    danger: 0xdc2626,
                    font_family: String::new(),
                },
            },
        ))
        .unwrap()
        .unwrap();
        assert!(output.bytes.starts_with(signature));
    }
}

fn coverage_fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../extension-wasm/fixtures/coverage-plugin")
        .canonicalize()
        .unwrap()
}

fn html_preview_transform_component_bytes() -> Vec<u8> {
    let dir = tempfile::TempDir::new().unwrap();
    let core_wat = dir.path().join("html_transform.wat");
    let embedded = dir.path().join("embedded.wasm");
    let component = dir.path().join("html_transform.component.wasm");
    fs::write(
        &core_wat,
        r#"
(module
  (memory (export "cm32p2_memory") 1)
  (data (i32.const 512) "\00\00\00\00\00\04\00\00\14\00\00\00\00\00\00\00\00\00\00\00")
  (data (i32.const 1024) "<main>Changed</main>")
  (func (export "cm32p2_realloc") (param i32 i32 i32 i32) (result i32)
    i32.const 2048)
  (func (export "cm32p2_initialize"))
  (func (export "cm32p2||transform-html") (param i32 i32 i32 i32) (result i32)
    i32.const 512)
  (func (export "cm32p2||transform-html_post") (param i32))
)
"#,
    )
    .unwrap();
    let wit_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extension-api/wit");
    let embed_output = Command::new("wasm-tools")
        .args([
            "component",
            "embed",
            wit_dir.to_str().unwrap(),
            "--world",
            "html-preview-transform",
            core_wat.to_str().unwrap(),
            "-o",
            embedded.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        embed_output.status.success(),
        "component embed failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&embed_output.stdout),
        String::from_utf8_lossy(&embed_output.stderr)
    );
    let new_output = Command::new("wasm-tools")
        .args([
            "component",
            "new",
            embedded.to_str().unwrap(),
            "-o",
            component.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        new_output.status.success(),
        "component new failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&new_output.stdout),
        String::from_utf8_lossy(&new_output.stderr)
    );
    fs::read(component).unwrap()
}
