use super::{DatabasePlugin, SqlCompletionInfo};
use crate::plugin_manifest::{
    DatabaseCapabilities, DatabaseUiManifest, FormSelectOption, ReferenceDataKind,
};
use crate::streaming_parser::StreamingSqlParser;
use crate::types::{CharsetInfo, CollationInfo, ParsedColumnType};
use crate::{SqlSource, StatementType};
use one_core::storage::DatabaseType;
use sqlparser::dialect::Dialect;
use std::collections::HashMap;
use std::io;

/// Synchronous plugin identity, dialect, manifest, and static reference-data API.
///
/// This trait is intentionally backed by a blanket implementation for the current
/// `DatabasePlugin` contract. Future IPC adapters can implement this smaller
/// surface directly while the in-process plugins continue using `DatabasePlugin`.
pub trait DatabasePluginCore: Send + Sync {
    fn name(&self) -> DatabaseType;
    fn quote_identifier(&self, identifier: &str) -> String;
    fn get_completion_info(&self) -> SqlCompletionInfo;
    fn supports_rowid(&self) -> bool;
    fn rowid_column_name(&self) -> &'static str;
    fn sql_dialect(&self) -> Box<dyn Dialect>;
    fn create_parser(&self, source: SqlSource) -> io::Result<StreamingSqlParser>;
    fn format_sql(&self, sql: &str) -> String;
    fn is_query_statement(&self, sql: &str) -> bool;
    fn split_sql_statements(&self, sql: &str) -> Vec<String>;
    fn build_explain_statement(&self, sql: &str) -> String;
    fn is_explain_statement(&self, sql: &str) -> bool;
    fn build_explain_sql(&self, sql: &str) -> Option<String>;
    fn classify_statement(&self, sql: &str) -> StatementType;
    fn analyze_select_editability(&self, sql: &str) -> Option<String>;
    fn capabilities(&self) -> DatabaseCapabilities;
    fn ui_manifest(&self) -> DatabaseUiManifest;
    fn resolve_reference_data(
        &self,
        kind: ReferenceDataKind,
        context: &HashMap<String, String>,
    ) -> Vec<FormSelectOption>;
    fn get_charsets(&self) -> Vec<CharsetInfo>;
    fn get_collations(&self, charset: &str) -> Vec<CollationInfo>;
    fn engines(&self) -> Vec<String>;
    fn get_data_types(&self) -> &[(&'static str, &'static str)];
    fn parse_column_type(&self, type_str: &str) -> ParsedColumnType;
    fn is_enum_type(&self, type_name: &str) -> bool;
}

impl<T> DatabasePluginCore for T
where
    T: DatabasePlugin + ?Sized,
{
    fn name(&self) -> DatabaseType {
        DatabasePlugin::name(self)
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        DatabasePlugin::quote_identifier(self, identifier)
    }

    fn get_completion_info(&self) -> SqlCompletionInfo {
        DatabasePlugin::get_completion_info(self)
    }

    fn supports_rowid(&self) -> bool {
        DatabasePlugin::supports_rowid(self)
    }

    fn rowid_column_name(&self) -> &'static str {
        DatabasePlugin::rowid_column_name(self)
    }

    fn sql_dialect(&self) -> Box<dyn Dialect> {
        DatabasePlugin::sql_dialect(self)
    }

    fn create_parser(&self, source: SqlSource) -> io::Result<StreamingSqlParser> {
        DatabasePlugin::create_parser(self, source)
    }

    fn format_sql(&self, sql: &str) -> String {
        DatabasePlugin::format_sql(self, sql)
    }

    fn is_query_statement(&self, sql: &str) -> bool {
        DatabasePlugin::is_query_statement(self, sql)
    }

    fn split_sql_statements(&self, sql: &str) -> Vec<String> {
        DatabasePlugin::split_sql_statements(self, sql)
    }

    fn build_explain_statement(&self, sql: &str) -> String {
        DatabasePlugin::build_explain_statement(self, sql)
    }

    fn is_explain_statement(&self, sql: &str) -> bool {
        DatabasePlugin::is_explain_statement(self, sql)
    }

    fn build_explain_sql(&self, sql: &str) -> Option<String> {
        DatabasePlugin::build_explain_sql(self, sql)
    }

    fn classify_statement(&self, sql: &str) -> StatementType {
        DatabasePlugin::classify_statement(self, sql)
    }

    fn analyze_select_editability(&self, sql: &str) -> Option<String> {
        DatabasePlugin::analyze_select_editability(self, sql)
    }

    fn capabilities(&self) -> DatabaseCapabilities {
        DatabasePlugin::capabilities(self)
    }

    fn ui_manifest(&self) -> DatabaseUiManifest {
        DatabasePlugin::ui_manifest(self)
    }

    fn resolve_reference_data(
        &self,
        kind: ReferenceDataKind,
        context: &HashMap<String, String>,
    ) -> Vec<FormSelectOption> {
        DatabasePlugin::resolve_reference_data(self, kind, context)
    }

    fn get_charsets(&self) -> Vec<CharsetInfo> {
        DatabasePlugin::get_charsets(self)
    }

    fn get_collations(&self, charset: &str) -> Vec<CollationInfo> {
        DatabasePlugin::get_collations(self, charset)
    }

    fn engines(&self) -> Vec<String> {
        DatabasePlugin::engines(self)
    }

    fn get_data_types(&self) -> &[(&'static str, &'static str)] {
        DatabasePlugin::get_data_types(self)
    }

    fn parse_column_type(&self, type_str: &str) -> ParsedColumnType {
        DatabasePlugin::parse_column_type(self, type_str)
    }

    fn is_enum_type(&self, type_name: &str) -> bool {
        DatabasePlugin::is_enum_type(self, type_name)
    }
}
