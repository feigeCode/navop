use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(super) struct WasmImporterFixture<'a> {
    pub(super) extension_dir: &'a str,
    pub(super) extension_id: &'a str,
    pub(super) importer_id: &'a str,
    pub(super) runtime_id: &'a str,
    pub(super) display_name: &'a str,
    pub(super) output_kind: &'a str,
    pub(super) component_name: &'a str,
    pub(super) core_wat: &'a str,
}

pub(super) fn write_wasm_importer_extension(
    composite_root: &Path,
    fixture: WasmImporterFixture<'_>,
) {
    let extension_dir = composite_root.join(fixture.extension_dir);
    let wasm_dir = extension_dir.join("wasm");
    fs::create_dir_all(&wasm_dir).unwrap();
    write_component_from_core_wat(&wasm_dir.join(fixture.component_name), fixture.core_wat);
    fs::write(
        extension_dir.join("extension.json"),
        format!(
            r#"{{
                "schema_version": 1,
                "id": "{extension_id}",
                "name": "{display_name} Importer",
                "version": "0.1.0",
                "engines": {{ "onetcli": ">=0.7.0" }},
                "runtime": {{
                    "wasm": [{{
                        "id": "{runtime_id}",
                        "module": "wasm/{component_name}",
                        "kind": "component"
                    }}]
                }},
                "contributes": {{
                    "connectionImporters": [{{
                        "id": "{importer_id}",
                        "runtimeId": "{runtime_id}",
                        "displayName": "{display_name}",
                        "outputKinds": ["{output_kind}"],
                        "platforms": ["macos"]
                    }}]
                }}
            }}"#,
            extension_id = fixture.extension_id,
            display_name = fixture.display_name,
            runtime_id = fixture.runtime_id,
            component_name = fixture.component_name,
            importer_id = fixture.importer_id,
            output_kind = fixture.output_kind,
        ),
    )
    .unwrap();
}

pub(super) fn write_broken_wasm_importer_extension(composite_root: &Path) {
    let extension_dir = composite_root.join("broken");
    fs::create_dir_all(&extension_dir).unwrap();
    fs::write(
        extension_dir.join("extension.json"),
        r#"{
            "schema_version": 1,
            "id": "com.onetcli.importer.broken",
            "name": "Broken Importer",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.7.0" },
            "runtime": {
                "wasm": [{
                    "id": "broken-importer",
                    "module": "wasm/missing.component.wasm",
                    "kind": "component"
                }]
            },
            "contributes": {
                "connectionImporters": [{
                    "id": "broken",
                    "runtimeId": "broken-importer",
                    "displayName": "Broken",
                    "outputKinds": ["database"],
                    "platforms": ["macos"]
                }]
            }
        }"#,
    )
    .unwrap();
}

fn write_component_from_core_wat(component: &Path, wat: &str) {
    let dir = tempfile::TempDir::new().unwrap();
    let core_wat = dir.path().join("importer.wat");
    let embedded = dir.path().join("embedded.wasm");
    fs::write(&core_wat, wat).unwrap();

    let wit_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extension-api/wit");
    let embed_output = Command::new("wasm-tools")
        .args([
            "component",
            "embed",
            wit_dir.to_str().unwrap(),
            "--world",
            "connection-importer",
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
}

pub(super) fn dbeaver_importer_core_wat() -> &'static str {
    include_str!("../../../extension-wasm/fixtures/connection-import/dbeaver_importer_core.wat")
}

pub(super) fn termius_importer_core_wat() -> &'static str {
    include_str!("../../../extension-wasm/fixtures/connection-import/termius_importer_core.wat")
}
