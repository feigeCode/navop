pub(crate) mod sql_dump_target;
pub mod sql_dump_view;
pub mod sql_run_view;
pub mod table_export_view;
pub mod table_import_view;

#[cfg(test)]
mod runtime_tests {
    const SQL_DUMP_VIEW: &str = include_str!("sql_dump_view.rs");
    const TABLE_EXPORT_VIEW: &str = include_str!("table_export_view.rs");
    const TABLE_IMPORT_VIEW: &str = include_str!("table_import_view.rs");
    const UNSAFE_EXPORT_API: &str = concat!("export_data_with_progress_", "sync");
    const UNSAFE_IMPORT_API: &str = concat!("import_data_with_progress_", "sync");
    const GPUI_BACKGROUND_SPAWN: &str = concat!("background_", "spawn");

    fn assert_safe_export_task(source: &str) {
        assert!(!source.contains(UNSAFE_EXPORT_API));
        assert!(!source.contains(GPUI_BACKGROUND_SPAWN));
        assert!(source.contains(".export_data_with_progress("));
    }

    #[test]
    fn database_transfer_views_delegate_runtime_to_global_state() {
        assert_safe_export_task(SQL_DUMP_VIEW);
        assert_safe_export_task(TABLE_EXPORT_VIEW);
        assert!(!TABLE_IMPORT_VIEW.contains(UNSAFE_IMPORT_API));
        assert!(!TABLE_IMPORT_VIEW.contains(GPUI_BACKGROUND_SPAWN));
        assert!(TABLE_IMPORT_VIEW.contains(".import_data_with_progress("));
    }
}
