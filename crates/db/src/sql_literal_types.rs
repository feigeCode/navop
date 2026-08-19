pub(crate) fn normalize_base_type(data_type: &str) -> String {
    data_type
        .trim()
        .split_once('(')
        .map_or_else(|| data_type.trim(), |(base, _)| base)
        .trim()
        .to_ascii_uppercase()
}

pub(crate) fn normalize_mysql_base_type(data_type: &str) -> String {
    let normalized = normalize_base_type(data_type);
    let normalized = normalized
        .strip_prefix("MYSQL_TYPE_")
        .unwrap_or(&normalized);
    normalized
        .split_whitespace()
        .filter(|part| !matches!(*part, "UNSIGNED" | "ZEROFILL"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn unwrap_clickhouse_type(data_type: &str) -> String {
    let mut current = data_type.trim();
    loop {
        let Some((wrapper, inner)) = split_type_wrapper(current) else {
            return current.to_ascii_uppercase();
        };
        if matches_ignore_ascii_case(wrapper, &["Nullable", "LowCardinality"]) {
            current = inner.trim();
        } else {
            return current.to_ascii_uppercase();
        }
    }
}

fn split_type_wrapper(data_type: &str) -> Option<(&str, &str)> {
    let open = data_type.find('(')?;
    if !data_type.ends_with(')') {
        return None;
    }
    let mut depth = 0usize;
    for (index, byte) in data_type.bytes().enumerate().skip(open) {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 && index != data_type.len() - 1 {
                    return None;
                }
            }
            _ => {}
        }
    }
    (depth == 0).then_some((
        &data_type[..open],
        &data_type[open + 1..data_type.len() - 1],
    ))
}

fn matches_ignore_ascii_case(value: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
}

pub(crate) fn is_mysql_numeric_type(data_type: &str) -> bool {
    matches!(
        data_type,
        "TINYINT"
            | "SMALLINT"
            | "MEDIUMINT"
            | "INT"
            | "INTEGER"
            | "BIGINT"
            | "DECIMAL"
            | "NUMERIC"
            | "FLOAT"
            | "DOUBLE"
            | "REAL"
    )
}

pub(crate) fn is_postgres_numeric_type(data_type: &str) -> bool {
    matches!(
        data_type,
        "SMALLINT"
            | "INTEGER"
            | "INT"
            | "BIGINT"
            | "DECIMAL"
            | "NUMERIC"
            | "REAL"
            | "DOUBLE PRECISION"
            | "FLOAT4"
            | "FLOAT8"
            | "INT2"
            | "INT4"
            | "INT8"
            | "SERIAL"
            | "BIGSERIAL"
            | "SMALLSERIAL"
    )
}

pub(crate) fn is_mssql_numeric_type(data_type: &str) -> bool {
    matches!(
        data_type,
        "TINYINT"
            | "SMALLINT"
            | "INT"
            | "BIGINT"
            | "DECIMAL"
            | "NUMERIC"
            | "FLOAT"
            | "REAL"
            | "MONEY"
            | "SMALLMONEY"
    )
}

pub(crate) fn is_sqlite_numeric_type(data_type: &str) -> bool {
    matches!(
        data_type,
        "INTEGER"
            | "INT"
            | "TINYINT"
            | "SMALLINT"
            | "MEDIUMINT"
            | "BIGINT"
            | "UNSIGNED BIG INT"
            | "INT2"
            | "INT8"
            | "NUMERIC"
            | "DECIMAL"
            | "REAL"
            | "DOUBLE"
            | "DOUBLE PRECISION"
            | "FLOAT"
    )
}

pub(crate) fn is_duckdb_numeric_type(data_type: &str) -> bool {
    matches!(
        data_type,
        "TINYINT"
            | "SMALLINT"
            | "INTEGER"
            | "INT"
            | "BIGINT"
            | "HUGEINT"
            | "UTINYINT"
            | "USMALLINT"
            | "UINTEGER"
            | "UBIGINT"
            | "UHUGEINT"
            | "DECIMAL"
            | "NUMERIC"
            | "REAL"
            | "FLOAT"
            | "DOUBLE"
            | "DOUBLE PRECISION"
            | "FLOAT4"
            | "FLOAT8"
            | "INT1"
            | "INT2"
            | "INT4"
            | "INT8"
    )
}

pub(crate) fn is_oracle_numeric_type(data_type: &str) -> bool {
    matches!(
        data_type,
        "NUMBER"
            | "BINARY_FLOAT"
            | "BINARY_DOUBLE"
            | "INTEGER"
            | "INT"
            | "SMALLINT"
            | "DECIMAL"
            | "NUMERIC"
            | "FLOAT"
            | "DOUBLE PRECISION"
            | "REAL"
    )
}

pub(crate) fn is_clickhouse_numeric_type(data_type: &str) -> bool {
    matches!(
        data_type,
        "INT8"
            | "INT16"
            | "INT32"
            | "INT64"
            | "INT128"
            | "INT256"
            | "UINT8"
            | "UINT16"
            | "UINT32"
            | "UINT64"
            | "UINT128"
            | "UINT256"
            | "FLOAT32"
            | "FLOAT64"
            | "DECIMAL"
            | "DECIMAL32"
            | "DECIMAL64"
            | "DECIMAL128"
            | "DECIMAL256"
    )
}
