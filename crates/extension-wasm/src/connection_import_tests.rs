use connection_import_protocol::{
    CandidateFile, HostAccessError, ImportRecordKind, Platform, SecretQuery, SecretResult,
};
use extension_component::{ExtensionConnectionImportHost, PermissionSet};
use std::{fs, process::Command};

use crate::{
    ConnectionImportComponentRuntime, ConnectionImportHostState, connection_import_bindings,
};

#[test]
fn generated_connection_import_host_reports_current_platform() {
    let mut state = ConnectionImportHostState::new(
        "ext",
        "navicat",
        TestImportHost {
            candidates: Vec::new(),
        },
        PermissionSet::new(["fs:read:data-sources.json"]),
    );

    let platform = futures::executor::block_on(
        connection_import_bindings::onet::extension::connection_import_host::Host::current_platform(
            &mut state,
        ),
    )
    .unwrap();

    assert_eq!(
        connection_import_bindings::onet::extension::connection_import::Platform::Macos,
        platform
    );
}

#[test]
fn generated_connection_import_host_rejects_undeclared_candidate_ids() {
    let mut state = ConnectionImportHostState::new(
        "ext",
        "navicat",
        TestImportHost {
            candidates: vec![CandidateFile {
                id: "navicat-conn".to_string(),
                platform: Some(Platform::Macos),
                path: "~/Library/Navicat/conn.plist".to_string(),
            }],
        },
        PermissionSet::new(["fs:read:~/Library/Navicat/conn.plist"]),
    );

    let result = futures::executor::block_on(
        connection_import_bindings::onet::extension::connection_import_host::Host::read_file(
            &mut state,
            "other".to_string(),
        ),
    )
    .unwrap();

    let error = result.expect_err("undeclared candidate id should fail");
    assert_eq!("undeclared_candidate", error.code);
}

#[test]
fn generated_connection_import_host_reads_nested_directory_entries() {
    let candidate = CandidateFile {
        id: "securecrt-config".to_string(),
        platform: Some(Platform::Macos),
        path: "~/Library/VanDyke/Config".to_string(),
    };
    let mut state = ConnectionImportHostState::new(
        "ext",
        "securecrt",
        TestImportHost {
            candidates: vec![candidate],
        },
        PermissionSet::new(["fs:read:~/Library/VanDyke/Config"]),
    );

    let entries = futures::executor::block_on(
        connection_import_bindings::onet::extension::connection_import_host::Host::read_candidate_directory(
            &mut state,
            "securecrt-config".to_string(),
            "Sessions/Production".to_string(),
        ),
    )
    .unwrap()
    .unwrap();

    assert_eq!(2, entries.len());
    assert_eq!("API.ini", entries[0].name);
    assert!(!entries[0].is_dir);
    assert_eq!("Database", entries[1].name);
    assert!(entries[1].is_dir);
}

#[test]
fn generated_connection_import_host_rejects_nested_directory_parent_escape() {
    let candidate = CandidateFile {
        id: "securecrt-config".to_string(),
        platform: Some(Platform::Macos),
        path: "~/Library/VanDyke/Config".to_string(),
    };
    let mut state = ConnectionImportHostState::new(
        "ext",
        "securecrt",
        TestImportHost {
            candidates: vec![candidate],
        },
        PermissionSet::new(["fs:read:~/Library/VanDyke/Config"]),
    );

    let result = futures::executor::block_on(
        connection_import_bindings::onet::extension::connection_import_host::Host::read_candidate_directory(
            &mut state,
            "securecrt-config".to_string(),
            "../Secrets".to_string(),
        ),
    )
    .unwrap();

    let error = result.expect_err("parent escape must be rejected");
    assert_eq!("permission_denied", error.code);
}

#[test]
fn dbeaver_wasm_fixture_returns_database_preview_record() {
    let runtime = runtime_from_core_wat("dbeaver", dbeaver_importer_core_wat());
    let state = ConnectionImportHostState::new(
        "com.onetcli.importer.dbeaver",
        "dbeaver",
        TestImportHost {
            candidates: Vec::new(),
        },
        PermissionSet::new(["fs:read:data-sources.json"]),
    );

    let preview = futures::executor::block_on(runtime.preview(state, true)).unwrap();

    assert_eq!(1, preview.len());
    assert_eq!(ImportRecordKind::Database, preview[0].kind);
    assert_eq!("DBeaver", preview[0].source_label);
    assert_eq!("prod-mysql", preview[0].display_name);
}

#[test]
fn wasip2_importer_component_returns_preview_record() {
    let runtime = ConnectionImportComponentRuntime::from_bytes_for_tests(
        "dbeaver",
        include_bytes!("../fixtures/connection-import/dbeaver_importer_wasip2.wasm"),
    )
    .unwrap();
    let state = ConnectionImportHostState::new(
        "com.onetcli.importer.dbeaver",
        "dbeaver",
        TestImportHost {
            candidates: vec![CandidateFile {
                id: "dbeaver-data-sources".to_string(),
                platform: Some(Platform::Macos),
                path: "data-sources.json".to_string(),
            }],
        },
        PermissionSet::new(["fs:read:data-sources.json"]),
    );

    let preview = futures::executor::block_on(runtime.preview(state, true)).unwrap();

    assert_eq!(1, preview.len());
    assert_eq!("Prod MySQL", preview[0].display_name);
}

#[test]
fn termius_wasm_fixture_returns_ssh_preview_record() {
    let runtime = runtime_from_core_wat("termius", termius_importer_core_wat());
    let state = ConnectionImportHostState::new(
        "com.onetcli.importer.termius",
        "termius",
        TestImportHost {
            candidates: Vec::new(),
        },
        PermissionSet::default(),
    );

    let preview = futures::executor::block_on(runtime.preview(state, false)).unwrap();

    assert_eq!(1, preview.len());
    assert_eq!(ImportRecordKind::Ssh, preview[0].kind);
    assert_eq!("Termius", preview[0].source_label);
    assert_eq!("prod-ssh", preview[0].display_name);
}

#[test]
fn wasm_preview_rejects_record_kind_payload_mismatch() {
    let runtime = runtime_from_core_wat("broken-shape", malformed_record_shape_importer_core_wat());
    let state = ConnectionImportHostState::new(
        "com.onetcli.importer.broken-shape",
        "broken-shape",
        TestImportHost {
            candidates: Vec::new(),
        },
        PermissionSet::default(),
    );

    let error = futures::executor::block_on(runtime.preview(state, false))
        .expect_err("a mismatched kind and payload must be rejected");

    assert!(matches!(error, crate::WasmError::ProtocolDecode(_)));
}

struct TestImportHost {
    candidates: Vec<CandidateFile>,
}

fn runtime_from_core_wat(id: &str, wat: &str) -> ConnectionImportComponentRuntime {
    let dir = tempfile::TempDir::new().unwrap();
    let core_wat = dir.path().join("importer.wat");
    let embedded = dir.path().join("embedded.wasm");
    let component = dir.path().join("importer.component.wasm");
    fs::write(&core_wat, wat).unwrap();

    let wit_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extension-api/wit");
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

    let bytes = fs::read(component).unwrap();
    ConnectionImportComponentRuntime::from_bytes_for_tests(id, &bytes).unwrap()
}

fn dbeaver_importer_core_wat() -> &'static str {
    include_str!("../fixtures/connection-import/dbeaver_importer_core.wat")
}

fn termius_importer_core_wat() -> &'static str {
    include_str!("../fixtures/connection-import/termius_importer_core.wat")
}

fn malformed_record_shape_importer_core_wat() -> &'static str {
    r#"(module
  (memory (export "cm32p2_memory") 1)
  (data (i32.const 512) "\00\04\00\00\10\01\00\00")
  (data (i32.const 520) "\00\08\00\00\70\00\00\00")
  (data (i32.const 528) "\00\0c\00\00\bc\00\00\00")
  (data (i32.const 1024) "{\"id\":\"broken-shape\",\"display_name\":\"Broken Shape\",\"description\":null,\"icon\":null,\"vendor\":null,\"supported_platforms\":[\"macos\"],\"output_kinds\":[\"ssh\"],\"capabilities\":{\"supports_scan\":true,\"supports_password_import\":false,\"supports_manual_file_pick\":false,\"supports_incremental_preview\":false}}")
  (data (i32.const 2048) "{\"importer_id\":\"broken-shape\",\"availability\":{\"available\":{\"estimated_count\":1}},\"discovered_files\":[],\"warnings\":[]}")
  (data (i32.const 3072) "[{\"id\":\"broken:ssh\",\"importer_id\":\"broken-shape\",\"source_label\":\"Broken\",\"kind\":\"ssh\",\"display_name\":\"Broken SSH\",\"database\":null,\"ssh\":null,\"password_status\":\"unsupported\",\"warnings\":[]}]")
  (func (export "realloc") (param i32 i32 i32 i32) (result i32)
    i32.const 4096)
  (func (export "cm32p2_realloc") (param i32 i32 i32 i32) (result i32)
    i32.const 4096)
  (func (export "cm32p2_initialize"))
  (func (export "cm32p2||descriptor") (result i32)
    i32.const 512)
  (func (export "cm32p2||descriptor_post") (param i32))
  (func (export "cm32p2||scan") (result i32)
    i32.const 520)
  (func (export "cm32p2||scan_post") (param i32))
  (func (export "cm32p2||preview") (param i32) (result i32)
    i32.const 528)
  (func (export "cm32p2||preview_post") (param i32))
)"#
}

impl ExtensionConnectionImportHost for TestImportHost {
    fn current_platform(&self) -> Platform {
        Platform::Macos
    }

    fn list_candidate_files(&self, _importer_id: &str) -> Vec<CandidateFile> {
        self.candidates.clone()
    }

    fn read_file(&self, candidate_id: &str) -> Result<Vec<u8>, HostAccessError> {
        match candidate_id {
            "navicat-conn" => Ok(b"plist".to_vec()),
            "dbeaver-data-sources" => Ok(dbeaver_data_sources_json().to_vec()),
            _ => Err(HostAccessError::UndeclaredCandidate(
                candidate_id.to_string(),
            )),
        }
    }

    fn read_directory(
        &self,
        candidate_id: &str,
    ) -> Result<Vec<connection_import_protocol::DirectoryEntry>, HostAccessError> {
        Err(HostAccessError::UndeclaredCandidate(
            candidate_id.to_string(),
        ))
    }

    fn read_candidate_directory(
        &self,
        candidate_id: &str,
        relative_path: &str,
    ) -> Result<Vec<connection_import_protocol::DirectoryEntry>, HostAccessError> {
        if candidate_id == "securecrt-config" && relative_path == "Sessions/Production" {
            return Ok(vec![
                connection_import_protocol::DirectoryEntry {
                    candidate_id: candidate_id.to_string(),
                    name: "API.ini".to_string(),
                    is_dir: false,
                },
                connection_import_protocol::DirectoryEntry {
                    candidate_id: candidate_id.to_string(),
                    name: "Database".to_string(),
                    is_dir: true,
                },
            ]);
        }
        Err(HostAccessError::NotFound(relative_path.to_string()))
    }

    fn read_secret(&self, _query: SecretQuery) -> SecretResult {
        SecretResult::Unsupported
    }

    fn log(&self, _level: &str, _message: &str) {}
}

fn dbeaver_data_sources_json() -> &'static [u8] {
    br#"{
      "connections": {
        "mysql-prod": {
          "provider": "mysql",
          "name": "Prod MySQL",
          "configuration": {
            "host": "db.example.com",
            "port": "3307",
            "database": "app",
            "user": "root"
          }
        }
      }
    }"#
}
