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
        PermissionSet::default(),
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
fn dbeaver_wasm_fixture_returns_database_preview_record() {
    let runtime = runtime_from_core_wat("dbeaver", dbeaver_importer_core_wat());
    let state = ConnectionImportHostState::new(
        "com.onetcli.importer.dbeaver",
        "dbeaver",
        TestImportHost {
            candidates: Vec::new(),
        },
        PermissionSet::default(),
    );

    let preview = futures::executor::block_on(runtime.preview(state, true)).unwrap();

    assert_eq!(1, preview.len());
    assert_eq!(ImportRecordKind::Database, preview[0].kind);
    assert_eq!("DBeaver", preview[0].source_label);
    assert_eq!("prod-mysql", preview[0].display_name);
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

impl ExtensionConnectionImportHost for TestImportHost {
    fn current_platform(&self) -> Platform {
        Platform::Macos
    }

    fn list_candidate_files(&self, _importer_id: &str) -> Vec<CandidateFile> {
        self.candidates.clone()
    }

    fn read_file(&self, candidate_id: &str) -> Result<Vec<u8>, HostAccessError> {
        if candidate_id == "navicat-conn" {
            Ok(b"plist".to_vec())
        } else {
            Err(HostAccessError::UndeclaredCandidate(
                candidate_id.to_string(),
            ))
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

    fn read_secret(&self, _query: SecretQuery) -> SecretResult {
        SecretResult::Unsupported
    }

    fn log(&self, _level: &str, _message: &str) {}
}
