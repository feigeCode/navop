use std::collections::BTreeMap;

use connection_import_protocol::{
    DatabaseImportRecord, ImportDatabaseType, ImportRecord, ImportRecordKind, ImportScanReport,
    ImportWarning, ImporterAvailability, ImporterCapabilities, ImporterDescriptor,
    PasswordImportStatus, Platform,
};

use super::connection_import_model::{
    ImportCenterState, ImportRowSaveStatus, previewable_source_ids_after_scan,
};

#[test]
fn available_sources_start_selected_and_unsupported_sources_do_not() {
    let state = ImportCenterState::new(
        vec![
            descriptor("dbeaver", vec![Platform::Macos]),
            descriptor("windows-only", vec![Platform::Windows]),
        ],
        Platform::Macos,
    );

    assert_eq!(vec!["dbeaver".to_string()], state.selected_source_ids());
    assert!(!state.source("windows-only").unwrap().selectable);
}

#[test]
fn scan_reports_are_scoped_to_the_matching_source() {
    let mut state = ImportCenterState::new(
        vec![descriptor("dbeaver", vec![Platform::Macos])],
        Platform::Macos,
    );

    state.apply_scan_reports(vec![scan_report("dbeaver", ImporterAvailability::NoData)]);

    assert!(matches!(
        state.source("dbeaver").unwrap().availability,
        ImporterAvailability::NoData
    ));
}

#[test]
fn preview_records_become_selected_pending_rows() {
    let mut state = ImportCenterState::empty_for_tests();

    state.apply_preview_records(vec![database_record("db")]);

    let row = state.rows().first().unwrap();
    assert!(row.selected);
    assert_eq!(ImportRowSaveStatus::Pending, row.save_status);
}

#[test]
fn saved_and_failed_results_are_kept_per_row() {
    let mut state = ImportCenterState::empty_for_tests();
    state.apply_preview_records(vec![database_record("db"), database_record("other")]);

    state.mark_saved("db", Some(42));
    state.mark_failed("other", "端口必须是 1-65535".to_string());

    assert_eq!(
        ImportRowSaveStatus::Saved {
            connection_id: Some(42)
        },
        state.row("db").unwrap().save_status
    );
    assert_eq!(
        ImportRowSaveStatus::Failed {
            message: "端口必须是 1-65535".to_string()
        },
        state.row("other").unwrap().save_status
    );
}

#[test]
fn scan_errors_do_not_block_other_preview_sources() {
    let selected_ids = vec!["broken".to_string(), "xshell".to_string()];

    let preview_ids = previewable_source_ids_after_scan(
        &selected_ids,
        &[
            scan_report(
                "broken",
                ImporterAvailability::Error {
                    message: "timeout".to_string(),
                },
            ),
            scan_report(
                "xshell",
                ImporterAvailability::Available {
                    estimated_count: Some(1),
                },
            ),
        ],
    );

    assert_eq!(vec!["xshell".to_string()], preview_ids);
}

#[test]
fn next_save_candidate_after_skips_current_saved_and_unselected_rows() {
    let mut state = ImportCenterState::empty_for_tests();
    state.apply_preview_records(vec![
        database_record("a"),
        database_record("b"),
        database_record("c"),
    ]);
    state.mark_saved("a", Some(1));
    state.toggle_row("b");

    assert_eq!(
        Some("c".to_string()),
        state.next_save_candidate_row_id_after("a")
    );
}

fn descriptor(id: &str, platforms: Vec<Platform>) -> ImporterDescriptor {
    ImporterDescriptor {
        id: id.to_string(),
        display_name: id.to_string(),
        description: None,
        icon: None,
        vendor: None,
        supported_platforms: platforms,
        output_kinds: vec![ImportRecordKind::Database],
        capabilities: ImporterCapabilities {
            supports_scan: true,
            supports_password_import: false,
            supports_manual_file_pick: true,
            supports_incremental_preview: false,
        },
    }
}

fn scan_report(importer_id: &str, availability: ImporterAvailability) -> ImportScanReport {
    ImportScanReport {
        importer_id: importer_id.to_string(),
        availability,
        discovered_files: Vec::new(),
        warnings: Vec::<ImportWarning>::new(),
    }
}

fn database_record(name: &str) -> ImportRecord {
    ImportRecord {
        id: name.to_string(),
        importer_id: "dbeaver".to_string(),
        source_label: "DBeaver".to_string(),
        source_id: None,
        kind: ImportRecordKind::Database,
        display_name: name.to_string(),
        database: Some(DatabaseImportRecord {
            database_type: ImportDatabaseType::MySql,
            name: name.to_string(),
            host: "mysql.example.test".to_string(),
            port: Some(3306),
            username: "root".to_string(),
            password: None,
            database: Some("app".to_string()),
            extra_params: BTreeMap::new(),
        }),
        ssh: None,
        port_forwarding: None,
        password_status: PasswordImportStatus::Unsupported,
        warnings: Vec::new(),
    }
}
