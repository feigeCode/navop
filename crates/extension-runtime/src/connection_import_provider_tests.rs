use std::fs;

use connection_import_protocol::{CandidateFile, HostAccessError, ImportRecordKind, Platform};
use extension_component::ExtensionConnectionImportHost;

use crate::connection_import_provider::{
    ManifestConnectionImportHost, ManualConnectionImportFile, candidates_for_platform,
    preview_manifest_connection_importers, scan_manifest_connection_importers,
};

mod fixtures;

use fixtures::{
    WasmImporterFixture, dbeaver_importer_core_wat, termius_importer_core_wat,
    write_broken_wasm_importer_extension, write_wasm_importer_extension,
};

#[test]
fn connection_import_provider_lists_manifest_importers_with_scoped_ids() {
    let tmp = tempfile::TempDir::new().unwrap();
    let extension_dir = tmp.path().join("navicat");
    fs::create_dir_all(&extension_dir).unwrap();
    fs::write(
        extension_dir.join("extension.json"),
        r#"{
            "schema_version": 1,
            "id": "com.onetcli.importer.navicat",
            "name": "Navicat Importer",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.7.0" },
            "runtime": {
                "wasm": [{
                    "id": "navicat-importer",
                    "module": "wasm/navicat_importer.wasm",
                    "kind": "component"
                }]
            },
            "contributes": {
                "connectionImporters": [{
                    "id": "navicat",
                    "runtimeId": "navicat-importer",
                    "displayName": "Navicat",
                    "outputKinds": ["database"],
                    "platforms": ["macos"],
                    "manualFilePick": {
                        "prompt": "选择 Navicat 导出的 connection.ncx 文件"
                    },
                    "candidateFiles": [{
                        "id": "navicat-conn",
                        "platform": "macos",
                        "path": "~/Library/Navicat/conn.plist"
                    }]
                }]
            }
        }"#,
    )
    .unwrap();

    let importers =
        crate::connection_import_provider::list_manifest_connection_importers(tmp.path()).unwrap();

    assert_eq!(1, importers.len());
    assert_eq!(
        "com.onetcli.importer.navicat/navicat",
        importers[0].descriptor.id
    );
    assert_eq!("Navicat", importers[0].descriptor.display_name);
    assert_eq!("navicat-importer", importers[0].runtime_id);
    assert_eq!(extension_dir, importers[0].extension_dir);
    assert!(
        importers[0]
            .descriptor
            .capabilities
            .supports_manual_file_pick
    );
    assert_eq!(
        Some("选择 Navicat 导出的 connection.ncx 文件"),
        importers[0]
            .descriptor
            .capabilities
            .manual_file_pick_prompt
            .as_deref()
    );
}

#[test]
fn connection_import_provider_previews_dbeaver_and_termius_wasm_fixtures() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_wasm_importer_extension(
        tmp.path(),
        WasmImporterFixture {
            extension_dir: "dbeaver",
            extension_id: "com.onetcli.importer.dbeaver",
            importer_id: "dbeaver",
            runtime_id: "dbeaver-importer",
            display_name: "DBeaver",
            output_kind: "database",
            component_name: "dbeaver.component.wasm",
            core_wat: dbeaver_importer_core_wat(),
        },
    );
    write_wasm_importer_extension(
        tmp.path(),
        WasmImporterFixture {
            extension_dir: "termius",
            extension_id: "com.onetcli.importer.termius",
            importer_id: "termius",
            runtime_id: "termius-importer",
            display_name: "Termius",
            output_kind: "ssh",
            component_name: "termius.component.wasm",
            core_wat: termius_importer_core_wat(),
        },
    );

    let report = futures::executor::block_on(preview_manifest_connection_importers(
        tmp.path(),
        &[
            "com.onetcli.importer.dbeaver/dbeaver".to_string(),
            "com.onetcli.importer.termius/termius".to_string(),
        ],
        true,
    ))
    .unwrap();

    assert_eq!(2, report.records.len());
    assert!(report.errors.is_empty());
    assert!(report.records.iter().any(|record| {
        record.kind == ImportRecordKind::Database && record.display_name == "prod-mysql"
    }));
    assert!(
        report
            .records
            .iter()
            .any(|record| record.kind == ImportRecordKind::Ssh && record.display_name == "prod-ssh")
    );
}

#[test]
fn connection_import_provider_keeps_preview_records_when_one_importer_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_wasm_importer_extension(
        tmp.path(),
        WasmImporterFixture {
            extension_dir: "dbeaver",
            extension_id: "com.onetcli.importer.dbeaver",
            importer_id: "dbeaver",
            runtime_id: "dbeaver-importer",
            display_name: "DBeaver",
            output_kind: "database",
            component_name: "dbeaver.component.wasm",
            core_wat: dbeaver_importer_core_wat(),
        },
    );
    write_broken_wasm_importer_extension(tmp.path());

    let report = futures::executor::block_on(preview_manifest_connection_importers(
        tmp.path(),
        &[
            "com.onetcli.importer.dbeaver/dbeaver".to_string(),
            "com.onetcli.importer.broken/broken".to_string(),
        ],
        true,
    ))
    .unwrap();

    assert_eq!(1, report.records.len());
    assert_eq!(
        "com.onetcli.importer.dbeaver/dbeaver",
        report.records[0].importer_id
    );
    assert_eq!("prod-mysql", report.records[0].display_name);
    assert_eq!(1, report.errors.len());
    assert_eq!(
        "com.onetcli.importer.broken/broken",
        report.errors[0].importer_id
    );
    assert!(!report.errors[0].message.is_empty());
}

#[test]
fn connection_import_provider_scans_each_selected_manifest_importer() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_wasm_importer_extension(
        tmp.path(),
        WasmImporterFixture {
            extension_dir: "dbeaver",
            extension_id: "com.onetcli.importer.dbeaver",
            importer_id: "dbeaver",
            runtime_id: "dbeaver-importer",
            display_name: "DBeaver",
            output_kind: "database",
            component_name: "dbeaver.component.wasm",
            core_wat: dbeaver_importer_core_wat(),
        },
    );

    let reports = futures::executor::block_on(scan_manifest_connection_importers(
        tmp.path(),
        &["com.onetcli.importer.dbeaver/dbeaver".to_string()],
    ))
    .unwrap();

    assert_eq!(1, reports.len());
    assert_eq!(
        "com.onetcli.importer.dbeaver/dbeaver",
        reports[0].importer_id
    );
    assert!(matches!(
        reports[0].availability,
        connection_import_protocol::ImporterAvailability::Available {
            estimated_count: Some(1)
        }
    ));
}

#[test]
fn manifest_connection_import_host_requires_manifest_fs_read_permission() {
    let tmp = tempfile::TempDir::new().unwrap();
    let candidate_path = tmp.path().join("connections.json");
    fs::write(&candidate_path, "{}").unwrap();
    let host = ManifestConnectionImportHost::new(
        vec![CandidateFile {
            id: "connections".to_string(),
            platform: Some(Platform::Macos),
            path: candidate_path.to_string_lossy().to_string(),
        }],
        Vec::<String>::new(),
    );

    let error = host
        .read_file("connections")
        .expect_err("manifest permission must gate file reads");

    assert_eq!(
        HostAccessError::PermissionDenied(candidate_path.to_string_lossy().to_string()),
        error
    );
}

#[test]
fn manifest_connection_import_host_filters_candidates_by_platform() {
    let candidates = vec![
        CandidateFile {
            id: "macos".to_string(),
            platform: Some(Platform::Macos),
            path: "/tmp/macos".to_string(),
        },
        CandidateFile {
            id: "windows".to_string(),
            platform: Some(Platform::Windows),
            path: "C:\\Users\\me\\securecrt".to_string(),
        },
        CandidateFile {
            id: "manual".to_string(),
            platform: None,
            path: "/tmp/manual".to_string(),
        },
    ];

    let visible = candidates_for_platform(&candidates, Platform::Macos);

    assert_eq!(
        vec!["macos", "manual"],
        visible
            .iter()
            .map(|candidate| candidate.id.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn manifest_connection_import_host_rejects_reads_for_other_platform_candidates() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("windows-only.ini");
    fs::write(&path, "session").unwrap();
    let path = path.to_string_lossy().to_string();
    let host = ManifestConnectionImportHost::new(
        vec![CandidateFile {
            id: "windows-only".to_string(),
            platform: Some(Platform::Windows),
            path: path.clone(),
        }],
        [format!("fs:read:{path}")],
    );

    assert_eq!(
        HostAccessError::UndeclaredCandidate("windows-only".to_string()),
        host.read_file("windows-only")
            .expect_err("a hidden candidate must not be readable")
    );
    assert_eq!(
        HostAccessError::UndeclaredCandidate("windows-only".to_string()),
        host.read_directory("windows-only")
            .expect_err("a hidden candidate directory must not be readable")
    );
    assert_eq!(
        HostAccessError::UndeclaredCandidate("windows-only".to_string()),
        host.read_candidate_directory("windows-only", "Sessions")
            .expect_err("a hidden candidate child directory must not be readable")
    );
    assert_eq!(
        HostAccessError::UndeclaredCandidate("windows-only".to_string()),
        host.read_candidate_child_file("windows-only", "Sessions/test.ini")
            .expect_err("a hidden candidate child file must not be readable")
    );
}

#[test]
fn manifest_connection_import_host_reads_nested_directory_entries() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = tmp.path().join("Config");
    let production = config.join("Sessions/Production");
    fs::create_dir_all(production.join("Database")).unwrap();
    fs::write(production.join("API.ini"), "session").unwrap();
    let config_path = config.to_string_lossy().to_string();
    let host = ManifestConnectionImportHost::new(
        vec![CandidateFile {
            id: "securecrt-config".to_string(),
            platform: Some(Platform::Macos),
            path: config_path.clone(),
        }],
        [format!("fs:read:{config_path}")],
    );

    let mut entries = host
        .read_candidate_directory("securecrt-config", "Sessions/Production")
        .unwrap();
    entries.sort_by(|left, right| left.name.cmp(&right.name));

    assert_eq!(2, entries.len());
    assert_eq!("API.ini", entries[0].name);
    assert!(!entries[0].is_dir);
    assert_eq!("Database", entries[1].name);
    assert!(entries[1].is_dir);
}

#[test]
fn manifest_connection_import_host_rejects_nested_directory_parent_escape() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().to_string_lossy().to_string();
    let host = ManifestConnectionImportHost::new(
        vec![CandidateFile {
            id: "securecrt-config".to_string(),
            platform: Some(Platform::Macos),
            path: config_path.clone(),
        }],
        [format!("fs:read:{config_path}")],
    );

    let error = host
        .read_candidate_directory("securecrt-config", "../Secrets")
        .expect_err("parent escape must be rejected");

    assert_eq!(
        HostAccessError::PermissionDenied("../Secrets".to_string()),
        error
    );
}

#[test]
fn manifest_connection_import_host_reads_user_selected_manual_files() {
    let tmp = tempfile::TempDir::new().unwrap();
    let manual_path = tmp.path().join("connection.ncx");
    fs::write(&manual_path, "<Connections/>").unwrap();
    let importer_id = "com.onetcli.importer.navicat/navicat";
    let host = ManifestConnectionImportHost::new(Vec::new(), Vec::<String>::new())
        .with_manual_files(
            importer_id,
            &[ManualConnectionImportFile::new(
                importer_id,
                manual_path.clone(),
            )],
        );

    let candidates = host.list_candidate_files("navicat");

    assert_eq!(1, candidates.len());
    assert_eq!("manual-file-0", candidates[0].id);
    assert_eq!(manual_path.to_string_lossy(), candidates[0].path);
    assert_eq!(
        b"<Connections/>",
        host.read_file("manual-file-0").unwrap().as_slice()
    );
}
