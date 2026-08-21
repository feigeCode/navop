use one_core::storage::DatabaseType;
use regex::Regex;

pub fn apply_query_max_rows(db_type: &DatabaseType, sql: &str, max_rows: usize) -> String {
    let trimmed = sql.trim();
    if !trimmed.to_uppercase().starts_with("SELECT") {
        return sql.to_string();
    }

    match db_type {
        DatabaseType::MySQL
        | DatabaseType::PostgreSQL
        | DatabaseType::SQLite
        | DatabaseType::DuckDB
        | DatabaseType::ClickHouse => {
            if has_limit_keyword(sql) {
                sql.to_string()
            } else {
                append_limit(sql, max_rows)
            }
        }
        DatabaseType::External { driver_id } => {
            // 如果是 Oracle 驱动，由驱动层处理，不追加任何限制
            if driver_id.to_lowercase().trim().contains("oracle") {
                sql.to_string()
            } else {
                // 其他外部驱动默认支持 LIMIT
                if has_limit_keyword(sql) {
                    sql.to_string()
                } else {
                    append_limit(sql, max_rows)
                }
            }
        }
        DatabaseType::MSSQL => {
            if has_top_keyword(sql) {
                sql.to_string()
            } else {
                insert_top(sql, max_rows)
            }
        }
        DatabaseType::Oracle => {
            // Oracle 由驱动层限制，不追加
            sql.to_string()
        }
    }
}

// ------- 辅助函数 -------

fn has_limit_keyword(sql: &str) -> bool {
    let lower = sql.to_lowercase();
    lower.split_whitespace().any(|w| w == "limit")
}

fn append_limit(sql: &str, max_rows: usize) -> String {
    if let Some(semi_pos) = sql.rfind(';') {
        let before = &sql[..semi_pos];
        let after = &sql[semi_pos..];
        format!("{} LIMIT {}{}", before, max_rows, after)
    } else {
        format!("{} LIMIT {}", sql, max_rows)
    }
}

fn has_top_keyword(sql: &str) -> bool {
    sql.to_lowercase().split_whitespace().any(|w| w == "top")
}

fn insert_top(sql: &str, max_rows: usize) -> String {
    let re = Regex::new(r"(?i)^(\s*SELECT\s+)").unwrap();
    if let Some(caps) = re.captures(sql) {
        let prefix = caps.get(1).unwrap().as_str();
        let rest = &sql[prefix.len()..];
        format!("{}TOP {} {}", prefix, max_rows, rest)
    } else {
        sql.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mysql_limit() {
        let sql = "SELECT * FROM users";
        let result = apply_query_max_rows(&DatabaseType::MySQL, sql, 10);
        assert_eq!(result, "SELECT * FROM users LIMIT 10");
    }

    #[test]
    fn test_mssql_top() {
        let sql = "SELECT * FROM users";
        let result = apply_query_max_rows(&DatabaseType::MSSQL, sql, 10);
        assert_eq!(result, "SELECT TOP 10 * FROM users");
    }

    #[test]
    fn test_oracle_no_limit() {
        let sql = "SELECT * FROM users";
        let result = apply_query_max_rows(&DatabaseType::Oracle, sql, 10);
        // Oracle 由驱动层限制，不追加任何子句
        assert_eq!(result, sql);
    }

    #[test]
    fn test_already_has_limit() {
        let sql = "SELECT * FROM users LIMIT 5";
        let result = apply_query_max_rows(&DatabaseType::MySQL, sql, 10);
        assert_eq!(result, sql);
    }

    #[test]
    fn test_non_select() {
        let sql = "DELETE FROM users";
        let result = apply_query_max_rows(&DatabaseType::MySQL, sql, 10);
        assert_eq!(result, sql);
    }

    #[test]
    fn test_external_non_oracle() {
        let sql = "SELECT * FROM external_table";
        let db = DatabaseType::External {
            driver_id: "postgres".to_string(),
        };
        let result = apply_query_max_rows(&db, sql, 100);
        assert_eq!(result, "SELECT * FROM external_table LIMIT 100");
    }

    #[test]
    fn test_external_oracle() {
        let sql = "SELECT * FROM oracle_table";
        let db = DatabaseType::External {
            driver_id: "oracle".to_string(),
        };
        let result = apply_query_max_rows(&db, sql, 50);
        // 由驱动层限制，不追加
        assert_eq!(result, sql);
    }
}
