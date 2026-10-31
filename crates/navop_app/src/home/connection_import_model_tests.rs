use std::collections::BTreeMap;

use connection_import_protocol::{
    DatabaseImportRecord, ImportDatabaseType, ImportRecord, ImportRecordKind, ImportScanReport,
    ImportWarning, ImporterAvailability, ImporterCapabilities, ImporterDescriptor,
    PasswordImportStatus, Platform, SshImportAuthMethod, SshImportRecord, WorkspaceImportRecord,
};
use extension_runtime::connection_import_provider::ImportPreviewError;

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
    assert!(
        state
            .source("dbeaver")
            .unwrap()
            .discovered_workspace_paths
            .is_empty()
    );
}

#[test]
fn scan_reports_store_discovered_workspace_groups_per_source() {
    let mut state = ImportCenterState::new(
        vec![
            descriptor("securecrt", vec![Platform::Macos]),
            descriptor("other", vec![Platform::Macos]),
        ],
        Platform::Macos,
    );
    let mut securecrt_report = scan_report(
        "securecrt",
        ImporterAvailability::Available {
            estimated_count: Some(4),
        },
    );
    securecrt_report.discovered_workspace_paths = vec![
        r"Production\Staging".to_string(),
        "Production / Staging".to_string(),
        "Operations".to_string(),
    ];
    let mut other_report = scan_report("other", ImporterAvailability::NoData);
    other_report.discovered_workspace_paths = vec!["Other".to_string()];

    state.apply_scan_reports(vec![securecrt_report, other_report]);

    assert_eq!(
        vec!["Operations", "Production/Staging"],
        state
            .source("securecrt")
            .unwrap()
            .discovered_workspace_paths
    );
    assert_eq!(
        vec!["Other"],
        state.source("other").unwrap().discovered_workspace_paths
    );
}

#[test]
fn preview_errors_are_scoped_to_the_matching_source_and_can_be_cleared() {
    let mut state = ImportCenterState::new(
        vec![
            descriptor("broken", vec![Platform::Macos]),
            descriptor("healthy", vec![Platform::Macos]),
        ],
        Platform::Macos,
    );

    state.apply_preview_errors(
        &["broken".to_string(), "healthy".to_string()],
        vec![ImportPreviewError {
            importer_id: "broken".to_string(),
            message: "preview failed".to_string(),
        }],
    );

    assert_eq!(
        Some("preview failed"),
        state.source("broken").unwrap().preview_error.as_deref()
    );
    assert!(state.source("healthy").unwrap().preview_error.is_none());

    state.apply_preview_errors(&["broken".to_string()], Vec::new());

    assert!(state.source("broken").unwrap().preview_error.is_none());
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
fn preview_records_report_distinct_normalized_ssh_workspace_groups() {
    let mut state = ImportCenterState::empty_for_tests();
    state.apply_preview_records(vec![
        ssh_record("api", Some("Production / Staging")),
        ssh_record("worker", Some(r" Production\Staging ")),
        ssh_record("ungrouped", None),
        ssh_record("ops", Some("Operations")),
        workspace_record("empty", "Empty"),
        database_record("db"),
    ]);

    assert_eq!(
        vec!["Empty", "Operations", "Production/Staging"],
        state.workspace_group_paths()
    );
}

#[test]
fn workspace_group_paths_only_include_selected_rows() {
    let mut state = ImportCenterState::empty_for_tests();
    state.apply_preview_records(vec![
        workspace_record("empty", "Empty"),
        ssh_record("api", Some("Production")),
    ]);

    state.toggle_row("empty");

    assert_eq!(vec!["Production"], state.workspace_group_paths());
}

#[test]
fn preview_records_store_discovered_workspace_groups_per_source() {
    let mut state = ImportCenterState::new(
        vec![
            descriptor("securecrt", vec![Platform::Macos]),
            descriptor("other", vec![Platform::Macos]),
        ],
        Platform::Macos,
    );
    state.apply_preview_records(vec![
        ssh_record("api", Some("Production / Staging")),
        ssh_record("worker", Some(r"Production\Staging")),
        workspace_record("empty", "Empty"),
        workspace_record_for("other", "other-group", "Other"),
        ssh_record("ungrouped", None),
    ]);

    assert_eq!(
        vec!["Empty", "Production/Staging"],
        state
            .source("securecrt")
            .unwrap()
            .discovered_workspace_paths
    );
    assert_eq!(
        vec!["Other"],
        state.source("other").unwrap().discovered_workspace_paths
    );
}

#[test]
fn discovered_workspace_groups_are_scan_results_not_row_selection() {
    let mut state = ImportCenterState::new(
        vec![descriptor("securecrt", vec![Platform::Macos])],
        Platform::Macos,
    );
    state.apply_preview_records(vec![workspace_record("empty", "Empty")]);

    state.toggle_row("empty");

    assert_eq!(
        vec!["Empty"],
        state
            .source("securecrt")
            .unwrap()
            .discovered_workspace_paths
    );
    assert!(state.workspace_group_paths().is_empty());
}

#[test]
fn preview_workspace_groups_replace_scan_workspace_groups() {
    let mut state = ImportCenterState::new(
        vec![descriptor("securecrt", vec![Platform::Macos])],
        Platform::Macos,
    );
    state.apply_scan_reports(vec![{
        let mut report = scan_report(
            "securecrt",
            ImporterAvailability::Available {
                estimated_count: Some(1),
            },
        );
        report.discovered_workspace_paths = vec!["Empty".to_string()];
        report
    }]);

    state.apply_preview_records(vec![workspace_record("empty", "Replacement")]);

    assert_eq!(
        vec!["Replacement"],
        state
            .source("securecrt")
            .unwrap()
            .discovered_workspace_paths
    );
}

#[test]
fn empty_preview_keeps_workspace_groups_discovered_by_scan() {
    let mut state = ImportCenterState::new(
        vec![descriptor("securecrt", vec![Platform::Macos])],
        Platform::Macos,
    );
    let mut report = scan_report(
        "securecrt",
        ImporterAvailability::Available {
            estimated_count: Some(1),
        },
    );
    report.discovered_workspace_paths = vec!["Production".to_string()];

    state.apply_scan_reports(vec![report]);
    state.apply_preview_records(Vec::new());

    assert_eq!(
        vec!["Production"],
        state
            .source("securecrt")
            .unwrap()
            .discovered_workspace_paths
    );
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
            supports_manual_directory_pick: false,
            manual_file_pick_prompt: None,
            manual_directory_pick_prompt: None,
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
        discovered_workspace_paths: Vec::new(),
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
        quick_command: None,
        workspace: None,
        password_status: PasswordImportStatus::Unsupported,
        warnings: Vec::new(),
    }
}

fn ssh_record(name: &str, group_path: Option<&str>) -> ImportRecord {
    ImportRecord {
        id: name.to_string(),
        importer_id: "securecrt".to_string(),
        source_label: "SecureCRT".to_string(),
        source_id: None,
        kind: ImportRecordKind::Ssh,
        display_name: name.to_string(),
        database: None,
        ssh: Some(SshImportRecord {
            name: name.to_string(),
            host: "ssh.example.test".to_string(),
            port: Some(22),
            username: "deploy".to_string(),
            group_path: group_path.map(str::to_string),
            auth_method: SshImportAuthMethod::Password { password: None },
            init_script: None,
            jump_server: None,
            proxy: None,
        }),
        port_forwarding: None,
        quick_command: None,
        workspace: None,
        password_status: PasswordImportStatus::Missing,
        warnings: Vec::new(),
    }
}

fn workspace_record(id: &str, path: &str) -> ImportRecord {
    workspace_record_for("securecrt", id, path)
}

fn workspace_record_for(importer_id: &str, id: &str, path: &str) -> ImportRecord {
    ImportRecord {
        id: id.to_string(),
        importer_id: importer_id.to_string(),
        source_label: "SecureCRT".to_string(),
        source_id: Some(format!("Sessions/{path}")),
        kind: ImportRecordKind::Workspace,
        display_name: path.rsplit('/').next().unwrap_or(path).to_string(),
        database: None,
        ssh: None,
        port_forwarding: None,
        quick_command: None,
        workspace: Some(WorkspaceImportRecord {
            path: path.to_string(),
        }),
        password_status: PasswordImportStatus::Missing,
        warnings: Vec::new(),
    }
}
