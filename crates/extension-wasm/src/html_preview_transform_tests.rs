use crate::HtmlPreviewTransformRuntime;
use std::{fs, process::Command};

#[test]
fn html_preview_transform_component_returns_modified_html() {
    let runtime = runtime_from_core_wat(
        "html-transform",
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
    );

    let result = futures::executor::block_on(runtime.transform_html("html", "<main>Hi</main>"))
        .expect("transform runs");

    assert_eq!("<main>Changed</main>", result.html);
}

fn runtime_from_core_wat(id: &str, wat: &str) -> HtmlPreviewTransformRuntime {
    let dir = tempfile::TempDir::new().unwrap();
    let core_wat = dir.path().join("html_transform.wat");
    let embedded = dir.path().join("embedded.wasm");
    let component = dir.path().join("html_transform.component.wasm");
    fs::write(&core_wat, wat).unwrap();

    let wit_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extension-api/wit");
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

    let bytes = fs::read(component).unwrap();
    HtmlPreviewTransformRuntime::from_bytes_for_tests(id, &bytes).unwrap()
}
