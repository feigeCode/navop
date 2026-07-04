use std::fs;

use connection_import_protocol::{CandidateFile, HostAccessError, ImportRecordKind, Platform};
use extension_component::ExtensionConnectionImportHost;

use crate::connection_import_provider::{
    ManifestConnectionImportHost, preview_manifest_connection_importers,
    scan_manifest_connection_importers,
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

    let records = futures::executor::block_on(preview_manifest_connection_importers(
        tmp.path(),
        &[
            "com.onetcli.importer.dbeaver/dbeaver".to_string(),
            "com.onetcli.importer.termius/termius".to_string(),
        ],
        true,
    ))
    .unwrap();

    assert_eq!(2, records.len());
    assert!(records.iter().any(|record| {
        record.kind == ImportRecordKind::Database && record.display_name == "prod-mysql"
    }));
    assert!(
        records
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

    let records = futures::executor::block_on(preview_manifest_connection_importers(
        tmp.path(),
        &[
            "com.onetcli.importer.dbeaver/dbeaver".to_string(),
            "com.onetcli.importer.broken/broken".to_string(),
        ],
        true,
    ))
    .unwrap();

    assert_eq!(1, records.len());
    assert_eq!(
        "com.onetcli.importer.dbeaver/dbeaver",
        records[0].importer_id
    );
    assert_eq!("prod-mysql", records[0].display_name);
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
