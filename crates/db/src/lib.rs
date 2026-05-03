rust_i18n::i18n!("locales", fallback = "en");

pub mod cache;
pub mod cache_manager;
pub mod connection;
pub mod ddl_invalidator;
pub mod executor;
pub mod import_export;
pub mod ipc;
pub mod manager;
mod manifest_helpers;
pub mod metadata_cache;
pub mod plugin;
pub mod plugin_manifest;
pub mod rustls_provider;
pub mod sql_format;
pub mod ssh_tunnel;
pub mod streaming_parser;
pub mod types;

// Database implementations
pub mod clickhouse;
pub mod duckdb;
pub mod mssql;
pub mod mysql;
pub mod oracle;
pub mod postgresql;
pub mod sql_editor;
pub mod sqlite;

// Re-exports
pub use cache::*;
pub use cache_manager::*;
pub use connection::*;
pub use ddl_invalidator::*;
pub use executor::*;
pub use import_export::*;
pub use manager::*;
pub use metadata_cache::*;
pub use plugin::*;
pub use plugin_manifest::*;
pub use rustls_provider::*;
pub use sql_format::*;
pub use ssh_tunnel::*;
pub use streaming_parser::*;
pub use types::*;

pub fn translate_or_raw_for_locale(locale: &str, key_or_text: &str) -> String {
    let translated = _rust_i18n_translate(locale, key_or_text).into_owned();
    let missing_with_locale = format!("{locale}.{key_or_text}");

    if translated == key_or_text || translated == missing_with_locale {
        key_or_text.to_string()
    } else {
        translated
    }
}

pub fn translate_connection_title_for_locale(
    locale: &str,
    is_editing: bool,
    db_type: &str,
) -> String {
    let key = if is_editing {
        "Connection.edit"
    } else {
        "Connection.new"
    };

    translate_or_raw_for_locale(locale, key).replace("%{db_type}", db_type)
}

pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_or_raw_for_locale_keeps_literal_placeholder() {
        assert_eq!(translate_or_raw_for_locale("zh-CN", "28800"), "28800");
    }

    #[test]
    fn translate_connection_title_for_locale_formats_db_type() {
        assert_eq!(
            translate_connection_title_for_locale("zh-CN", false, "MySQL"),
            "新建 MySQL 连接"
        );
        assert_eq!(
            translate_connection_title_for_locale("zh-CN", true, "MySQL"),
            "编辑 MySQL 连接"
        );
    }
}
