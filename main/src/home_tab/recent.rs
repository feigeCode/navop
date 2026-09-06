use super::connection_filter::{connection_matches_query, match_connection_type};
use super::{ConnectionType, StoredConnection};

/// Takes no workspace selection or global sort order, intentionally.
/// `limit` 为当前布局一行容量（卡片视图=共享网格列数），由调用方决定。
pub(super) fn recent_connections(
    connections: &[StoredConnection],
    filter: ConnectionType,
    query: &str,
    limit: usize,
) -> Vec<StoredConnection> {
    let mut recent: Vec<_> = connections
        .iter()
        .filter(|conn| conn.last_used_at.is_some())
        .filter(|conn| match_connection_type(filter, conn))
        .filter(|conn| connection_matches_query(conn, query))
        .cloned()
        .collect();
    recent.sort_by_key(|conn| (std::cmp::Reverse(conn.last_used_at), conn.id));
    recent.truncate(limit.max(1));
    recent
}

#[cfg(test)]
mod tests {
    use super::*;
    fn connection(id: i64, kind: ConnectionType, used: Option<i64>) -> StoredConnection {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": format!("Server {id}"), "connection_type": kind,
            "params": "{}", "workspace_id": id % 2, "last_used_at": used
        }))
        .unwrap()
    }
    #[test]
    fn recent_is_limited_in_descending_time_order() {
        let mut connections: Vec<_> = (1..7)
            .map(|id| connection(id, ConnectionType::Telnet, Some(id)))
            .collect();
        connections.push(connection(99, ConnectionType::Telnet, None));
        let recent = recent_connections(&connections, ConnectionType::All, "", 4);
        assert_eq!(
            recent.iter().map(|c| c.id.unwrap()).collect::<Vec<_>>(),
            vec![6, 5, 4, 3]
        );
        assert_eq!(
            recent_connections(&connections, ConnectionType::All, "", 2)
                .iter()
                .map(|c| c.id.unwrap())
                .collect::<Vec<_>>(),
            vec![6, 5]
        );
        assert_eq!(connections.len(), 7);
    }
    #[test]
    fn recent_applies_type_and_text_before_limiting_across_groups() {
        let connections = vec![
            connection(1, ConnectionType::Telnet, Some(1)),
            connection(2, ConnectionType::Redis, Some(2)),
            connection(3, ConnectionType::Telnet, Some(3)),
        ];
        assert_eq!(
            recent_connections(&connections, ConnectionType::Telnet, "", 4).len(),
            2
        );
        let recent = recent_connections(&connections, ConnectionType::All, "SERVER 2", 4);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, Some(2));
        assert!(recent_connections(&connections, ConnectionType::Redis, "missing", 4).is_empty());
    }
}
