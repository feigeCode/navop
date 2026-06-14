use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SqlAccess {
    Read,
    Write,
    Schema,
    Admin,
}

pub fn classify_sql(sql: &str) -> SqlAccess {
    let mut highest = SqlAccess::Read;
    for statement in sql.split(';') {
        let Some(first) = first_keyword(statement) else {
            continue;
        };
        highest = max_access(highest, classify_keyword(first));
    }
    highest
}

impl SqlAccess {
    pub fn rank(self) -> u8 {
        match self {
            Self::Read => 0,
            Self::Write => 1,
            Self::Schema => 2,
            Self::Admin => 3,
        }
    }
}

fn first_keyword(statement: &str) -> Option<&str> {
    statement
        .trim_start()
        .split(|ch: char| !ch.is_ascii_alphabetic())
        .find(|token| !token.is_empty())
}

fn classify_keyword(keyword: &str) -> SqlAccess {
    match keyword.to_ascii_lowercase().as_str() {
        "select" | "with" | "show" | "describe" | "explain" => SqlAccess::Read,
        "insert" | "update" | "delete" | "merge" | "copy" | "replace" => SqlAccess::Write,
        "create" | "alter" | "drop" | "truncate" | "rename" => SqlAccess::Schema,
        _ => SqlAccess::Admin,
    }
}

fn max_access(left: SqlAccess, right: SqlAccess) -> SqlAccess {
    if access_rank(right) > access_rank(left) {
        right
    } else {
        left
    }
}

fn access_rank(access: SqlAccess) -> u8 {
    access.rank()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_sql_access() {
        assert_eq!(SqlAccess::Read, classify_sql("select * from t"));
        assert_eq!(SqlAccess::Write, classify_sql("insert into t values (1)"));
        assert_eq!(SqlAccess::Schema, classify_sql("drop table t"));
        assert_eq!(SqlAccess::Admin, classify_sql("begin; select 1; commit"));
    }
}
