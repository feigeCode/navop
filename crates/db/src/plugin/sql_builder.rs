use super::{DatabaseOperationRequest, DatabasePlugin};
use crate::types::{
    ColumnDefinition, ColumnInfo, CopySqlRequest, TableDesign, TableRowChange, TableSaveRequest,
};

/// Synchronous SQL generation and dialect-specific formatting operations.
pub trait DatabaseSqlBuilder: Send + Sync {
    fn build_column_definition(&self, column: &ColumnInfo, include_name: bool) -> String;
    fn build_create_database_sql(&self, request: &DatabaseOperationRequest) -> String;
    fn build_modify_database_sql(&self, request: &DatabaseOperationRequest) -> String;
    fn build_drop_database_sql(&self, database_name: &str) -> String;
    fn build_create_schema_sql(&self, schema_name: &str) -> String;
    fn build_drop_schema_sql(&self, schema_name: &str) -> String;
    fn build_comment_schema_sql(&self, schema_name: &str, comment: &str) -> Option<String>;
    fn format_pagination(&self, limit: usize, offset: usize, order_clause: &str) -> String;
    fn format_table_reference(&self, database: &str, schema: Option<&str>, table: &str) -> String;
    fn format_export_table_reference(
        &self,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> String;
    fn generate_table_changes_sql(&self, request: &TableSaveRequest) -> String;
    fn generate_copy_insert_sql(&self, request: &CopySqlRequest) -> String;
    fn generate_copy_insert_with_comments_sql(&self, request: &CopySqlRequest) -> String;
    fn generate_copy_update_sql(&self, request: &CopySqlRequest) -> String;
    fn generate_copy_delete_sql(&self, request: &CopySqlRequest) -> String;
    fn format_copy_table_name(&self, schema: Option<&str>, table: &str) -> String;
    fn format_copy_value(&self, value: &Option<String>, col_info: Option<&ColumnInfo>) -> String;
    fn is_numeric_type(&self, data_type: &str) -> bool;
    fn is_boolean_type(&self, data_type: &str) -> bool;
    fn is_binary_type(&self, data_type: &str) -> bool;
    fn format_boolean_value(&self, value: &str) -> String;
    fn format_binary_value(&self, value: &str) -> String;
    fn escape_copy_string(&self, value: &str) -> String;
    fn generate_copy_where_clause(
        &self,
        request: &CopySqlRequest,
        row: &[Option<String>],
    ) -> String;
    fn build_table_change_sql(
        &self,
        request: &TableSaveRequest,
        change: &TableRowChange,
    ) -> Option<String>;
    fn build_limit_clause(&self) -> String;
    fn build_where_and_limit_clause(
        &self,
        request: &TableSaveRequest,
        original_data: &[String],
    ) -> (String, String);
    fn build_table_change_where_clause(
        &self,
        request: &TableSaveRequest,
        original_data: &[String],
    ) -> String;
    fn drop_database(&self, database: &str) -> String;
    fn drop_table(&self, database: &str, schema: Option<&str>, table: &str) -> String;
    fn truncate_table(&self, database: &str, table: &str) -> String;
    fn rename_table(&self, database: &str, old_name: &str, new_name: &str) -> String;
    fn build_backup_table_sql(
        &self,
        database: &str,
        schema: Option<&str>,
        source_table: &str,
        target_table: &str,
    ) -> String;
    fn drop_view(&self, database: &str, view: &str) -> String;
    fn build_column_def(&self, col: &ColumnDefinition) -> String;
    fn build_create_table_sql(&self, design: &TableDesign) -> String;
    fn build_alter_table_sql(&self, original: &TableDesign, new: &TableDesign) -> String;
    fn build_column_rename_sql(
        &self,
        table_name: &str,
        old_name: &str,
        new_name: &str,
        new_column: Option<&ColumnDefinition>,
    ) -> String;
    fn build_alter_table_sql_with_renames(
        &self,
        original: &TableDesign,
        new: &TableDesign,
        column_renames: &[(String, String)],
    ) -> String;
    fn column_changed(&self, original: &ColumnDefinition, new: &ColumnDefinition) -> bool;
    fn build_type_string(&self, col: &ColumnDefinition) -> String;
}

impl<T> DatabaseSqlBuilder for T
where
    T: DatabasePlugin + ?Sized,
{
    fn build_column_definition(&self, column: &ColumnInfo, include_name: bool) -> String {
        DatabasePlugin::build_column_definition(self, column, include_name)
    }

    fn build_create_database_sql(&self, request: &DatabaseOperationRequest) -> String {
        DatabasePlugin::build_create_database_sql(self, request)
    }

    fn build_modify_database_sql(&self, request: &DatabaseOperationRequest) -> String {
        DatabasePlugin::build_modify_database_sql(self, request)
    }

    fn build_drop_database_sql(&self, database_name: &str) -> String {
        DatabasePlugin::build_drop_database_sql(self, database_name)
    }

    fn build_create_schema_sql(&self, schema_name: &str) -> String {
        DatabasePlugin::build_create_schema_sql(self, schema_name)
    }

    fn build_drop_schema_sql(&self, schema_name: &str) -> String {
        DatabasePlugin::build_drop_schema_sql(self, schema_name)
    }

    fn build_comment_schema_sql(&self, schema_name: &str, comment: &str) -> Option<String> {
        DatabasePlugin::build_comment_schema_sql(self, schema_name, comment)
    }

    fn format_pagination(&self, limit: usize, offset: usize, order_clause: &str) -> String {
        DatabasePlugin::format_pagination(self, limit, offset, order_clause)
    }

    fn format_table_reference(&self, database: &str, schema: Option<&str>, table: &str) -> String {
        DatabasePlugin::format_table_reference(self, database, schema, table)
    }

    fn format_export_table_reference(
        &self,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> String {
        DatabasePlugin::format_export_table_reference(self, database, schema, table)
    }

    fn generate_table_changes_sql(&self, request: &TableSaveRequest) -> String {
        DatabasePlugin::generate_table_changes_sql(self, request)
    }

    fn generate_copy_insert_sql(&self, request: &CopySqlRequest) -> String {
        DatabasePlugin::generate_copy_insert_sql(self, request)
    }

    fn generate_copy_insert_with_comments_sql(&self, request: &CopySqlRequest) -> String {
        DatabasePlugin::generate_copy_insert_with_comments_sql(self, request)
    }

    fn generate_copy_update_sql(&self, request: &CopySqlRequest) -> String {
        DatabasePlugin::generate_copy_update_sql(self, request)
    }

    fn generate_copy_delete_sql(&self, request: &CopySqlRequest) -> String {
        DatabasePlugin::generate_copy_delete_sql(self, request)
    }

    fn format_copy_table_name(&self, schema: Option<&str>, table: &str) -> String {
        DatabasePlugin::format_copy_table_name(self, schema, table)
    }

    fn format_copy_value(&self, value: &Option<String>, col_info: Option<&ColumnInfo>) -> String {
        DatabasePlugin::format_copy_value(self, value, col_info)
    }

    fn is_numeric_type(&self, data_type: &str) -> bool {
        DatabasePlugin::is_numeric_type(self, data_type)
    }

    fn is_boolean_type(&self, data_type: &str) -> bool {
        DatabasePlugin::is_boolean_type(self, data_type)
    }

    fn is_binary_type(&self, data_type: &str) -> bool {
        DatabasePlugin::is_binary_type(self, data_type)
    }

    fn format_boolean_value(&self, value: &str) -> String {
        DatabasePlugin::format_boolean_value(self, value)
    }

    fn format_binary_value(&self, value: &str) -> String {
        DatabasePlugin::format_binary_value(self, value)
    }

    fn escape_copy_string(&self, value: &str) -> String {
        DatabasePlugin::escape_copy_string(self, value)
    }

    fn generate_copy_where_clause(
        &self,
        request: &CopySqlRequest,
        row: &[Option<String>],
    ) -> String {
        DatabasePlugin::generate_copy_where_clause(self, request, row)
    }

    fn build_table_change_sql(
        &self,
        request: &TableSaveRequest,
        change: &TableRowChange,
    ) -> Option<String> {
        DatabasePlugin::build_table_change_sql(self, request, change)
    }

    fn build_limit_clause(&self) -> String {
        DatabasePlugin::build_limit_clause(self)
    }

    fn build_where_and_limit_clause(
        &self,
        request: &TableSaveRequest,
        original_data: &[String],
    ) -> (String, String) {
        DatabasePlugin::build_where_and_limit_clause(self, request, original_data)
    }

    fn build_table_change_where_clause(
        &self,
        request: &TableSaveRequest,
        original_data: &[String],
    ) -> String {
        DatabasePlugin::build_table_change_where_clause(self, request, original_data)
    }

    fn drop_database(&self, database: &str) -> String {
        DatabasePlugin::drop_database(self, database)
    }

    fn drop_table(&self, database: &str, schema: Option<&str>, table: &str) -> String {
        DatabasePlugin::drop_table(self, database, schema, table)
    }

    fn truncate_table(&self, database: &str, table: &str) -> String {
        DatabasePlugin::truncate_table(self, database, table)
    }

    fn rename_table(&self, database: &str, old_name: &str, new_name: &str) -> String {
        DatabasePlugin::rename_table(self, database, old_name, new_name)
    }

    fn build_backup_table_sql(
        &self,
        database: &str,
        schema: Option<&str>,
        source_table: &str,
        target_table: &str,
    ) -> String {
        DatabasePlugin::build_backup_table_sql(self, database, schema, source_table, target_table)
    }

    fn drop_view(&self, database: &str, view: &str) -> String {
        DatabasePlugin::drop_view(self, database, view)
    }

    fn build_column_def(&self, col: &ColumnDefinition) -> String {
        DatabasePlugin::build_column_def(self, col)
    }

    fn build_create_table_sql(&self, design: &TableDesign) -> String {
        DatabasePlugin::build_create_table_sql(self, design)
    }

    fn build_alter_table_sql(&self, original: &TableDesign, new: &TableDesign) -> String {
        DatabasePlugin::build_alter_table_sql(self, original, new)
    }

    fn build_column_rename_sql(
        &self,
        table_name: &str,
        old_name: &str,
        new_name: &str,
        new_column: Option<&ColumnDefinition>,
    ) -> String {
        DatabasePlugin::build_column_rename_sql(self, table_name, old_name, new_name, new_column)
    }

    fn build_alter_table_sql_with_renames(
        &self,
        original: &TableDesign,
        new: &TableDesign,
        column_renames: &[(String, String)],
    ) -> String {
        DatabasePlugin::build_alter_table_sql_with_renames(self, original, new, column_renames)
    }

    fn column_changed(&self, original: &ColumnDefinition, new: &ColumnDefinition) -> bool {
        DatabasePlugin::column_changed(self, original, new)
    }

    fn build_type_string(&self, col: &ColumnDefinition) -> String {
        DatabasePlugin::build_type_string(self, col)
    }
}
