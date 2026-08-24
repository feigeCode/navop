use std::cmp::Ordering;
use std::iter::Peekable;
use std::str::Chars;

use one_core::settings::ConnectionSortOrder;
use one_core::storage::StoredConnection;

/// 连接名称排序比较：数字段按数值比较（对 IP 地址友好，如 "172.29.13.2" < "172.29.13.100"），
/// 其余部分按字符比较（忽略大小写）。
pub(crate) fn connection_name_cmp(left: &str, right: &str) -> Ordering {
    natural_cmp(left, right)
}

/// 按设置中的排序方式对连接列表原地重排。
///
/// 后续要新增排序方式时，只需在 [`ConnectionSortOrder`] 中增加一个变体并在此处
/// 补充对应的比较分支即可。
pub(crate) fn sort_connections(connections: &mut [StoredConnection], order: ConnectionSortOrder) {
    match order {
        ConnectionSortOrder::Natural => {
            connections.sort_by(|left, right| connection_name_cmp(&left.name, &right.name));
        }
        ConnectionSortOrder::Lru => {
            connections.sort_by(lru_cmp);
        }
    }
}

/// LRU 比较：最近打开过的连接排最前；从未打开过的按更新时间/创建时间回退；
/// 仍相同时按 id 倒序保证排序稳定，与数据库 `list` 的 LRU 排序保持一致。
fn lru_cmp(left: &StoredConnection, right: &StoredConnection) -> Ordering {
    let left_key = lru_sort_key(
        left.last_used_at,
        left.updated_at,
        left.created_at,
        left.id,
    );
    let right_key = lru_sort_key(
        right.last_used_at,
        right.updated_at,
        right.created_at,
        right.id,
    );
    right_key.cmp(&left_key)
}

/// 供侧栏连接树等节点排序复用：最近使用时间优先，回退到更新/创建时间，再按 id。
pub(crate) fn lru_sort_key(
    last_used_at: Option<i64>,
    updated_at: Option<i64>,
    created_at: Option<i64>,
    id: Option<i64>,
) -> (i64, i64) {
    let primary = last_used_at.unwrap_or_else(|| updated_at.or(created_at).unwrap_or(0));
    (primary, id.unwrap_or(0))
}

fn natural_cmp(left: &str, right: &str) -> Ordering {
    let mut left_chars = left.chars().peekable();
    let mut right_chars = right.chars().peekable();
    loop {
        match (left_chars.peek().copied(), right_chars.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left), Some(right)) if left.is_ascii_digit() && right.is_ascii_digit() => {
                match compare_number_runs(&mut left_chars, &mut right_chars) {
                    Ordering::Equal => {}
                    other => return other,
                }
            }
            (Some(left), Some(right)) => {
                match left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase()) {
                    Ordering::Equal => {}
                    other => return other,
                }
                left_chars.next();
                right_chars.next();
            }
        }
    }
}

/// 比较两个连续数字段（如 IP 段 "2" 与 "100"），按数值大小比较。
fn compare_number_runs(
    left: &mut Peekable<Chars<'_>>,
    right: &mut Peekable<Chars<'_>>,
) -> Ordering {
    let left_run = take_number_run(left);
    let right_run = take_number_run(right);
    left_run
        .len
        .cmp(&right_run.len)
        .then(left_run.value.cmp(&right_run.value))
}

fn take_number_run(chars: &mut Peekable<Chars<'_>>) -> NumberRun {
    let mut len = 0usize;
    let mut value = 0u128;
    while let Some(&digit) = chars.peek() {
        if !digit.is_ascii_digit() {
            break;
        }
        len += 1;
        value = value
            .saturating_mul(10)
            .saturating_add(u128::from(digit.to_digit(10).unwrap_or(0)));
        chars.next();
    }
    NumberRun { len, value }
}

struct NumberRun {
    len: usize,
    value: u128,
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::{RedisMode, RedisParams};

    fn sorted(mut names: Vec<&str>) -> Vec<&str> {
        names.sort_by(|left, right| connection_name_cmp(left, right));
        names
    }

    #[test]
    fn ip_addresses_sort_numerically_by_segment() {
        let names = sorted(vec![
            "172.29.13.100",
            "172.29.13.2",
            "172.29.9.1",
            "172.29.13.200",
        ]);

        assert_eq!(
            vec![
                "172.29.9.1",
                "172.29.13.2",
                "172.29.13.100",
                "172.29.13.200"
            ],
            names,
        );
    }

    #[test]
    fn hostnames_with_numbers_sort_naturally() {
        let names = sorted(vec!["host-10", "host-2", "host-1"]);

        assert_eq!(vec!["host-1", "host-2", "host-10"], names);
    }

    #[test]
    fn plain_names_sort_lexically_case_insensitive() {
        let names = sorted(vec!["Redis", "db-server", "App Server", "Db-Server"]);

        assert_eq!(vec!["App Server", "db-server", "Db-Server", "Redis"], names);
    }

    #[test]
    fn identical_names_are_equal() {
        assert_eq!(Ordering::Equal, connection_name_cmp("db-1", "db-1"));
    }

    fn connection(id: i64, name: &str, last_used_at: Option<i64>) -> StoredConnection {
        let params = RedisParams {
            host: "localhost".to_string(),
            port: 6379,
            password: None,
            username: None,
            credential_reference: None,
            db_index: 0,
            mode: RedisMode::Standalone,
            use_tls: false,
            connect_timeout: None,
            sentinel: None,
            cluster: None,
            ssh_tunnel: None,
        };
        let mut connection = StoredConnection::new_redis(name.to_string(), params, None);
        connection.id = Some(id);
        connection.last_used_at = last_used_at;
        connection
    }

    fn sorted_connection_names(
        mut connections: Vec<StoredConnection>,
        order: ConnectionSortOrder,
    ) -> Vec<String> {
        sort_connections(&mut connections, order);
        connections.into_iter().map(|connection| connection.name).collect()
    }

    #[test]
    fn natural_order_sorts_connections_by_name() {
        let connections = vec![
            connection(1, "host-10", Some(200)),
            connection(2, "host-2", Some(300)),
            connection(3, "host-1", None),
        ];

        assert_eq!(
            vec!["host-1", "host-2", "host-10"],
            sorted_connection_names(connections, ConnectionSortOrder::Natural),
        );
    }

    #[test]
    fn lru_order_puts_most_recently_used_first() {
        let connections = vec![
            connection(1, "old", Some(100)),
            connection(2, "never", None),
            connection(3, "new", Some(300)),
            connection(4, "mid", Some(200)),
        ];

        assert_eq!(
            vec!["new", "mid", "old", "never"],
            sorted_connection_names(connections, ConnectionSortOrder::Lru),
        );
    }

    #[test]
    fn lru_order_falls_back_to_updated_at_when_unused() {
        // 从未打开过的连接按更新时间回退（与数据库 COALESCE(last_used_at, updated_at, created_at) 一致）
        let mut edited_later = connection(1, "edited-later", None);
        edited_later.updated_at = Some(200);
        let mut edited_earlier = connection(2, "edited-earlier", None);
        edited_earlier.updated_at = Some(100);
        let untouched = connection(3, "untouched", None);

        assert_eq!(
            vec!["edited-later", "edited-earlier", "untouched"],
            sorted_connection_names(
                vec![untouched, edited_earlier, edited_later],
                ConnectionSortOrder::Lru,
            ),
        );
    }

    #[test]
    fn lru_order_breaks_ties_by_id_desc_for_stability() {
        let connections = vec![
            connection(5, "fifth", None),
            connection(1, "first", None),
            connection(9, "ninth", None),
        ];

        assert_eq!(
            vec!["ninth", "fifth", "first"],
            sorted_connection_names(connections, ConnectionSortOrder::Lru),
        );
    }
}
