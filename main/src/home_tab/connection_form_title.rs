use super::*;

pub(super) fn external_driver_id_for_connection_form(
    db_type: &DatabaseType,
    editing_conn: Option<&StoredConnection>,
) -> Option<String> {
    db_type
        .external_driver_id()
        .map(str::to_string)
        .or_else(|| {
            editing_conn
                .and_then(|connection| connection.to_db_connection().ok())
                .and_then(|config| {
                    config
                        .database_type
                        .external_driver_id()
                        .map(str::to_string)
                })
        })
}

pub(super) fn non_empty_name(name: &str) -> Option<&str> {
    let name = name.trim();
    if name.is_empty() { None } else { Some(name) }
}
