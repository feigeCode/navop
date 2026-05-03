use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum SqlResult {
    Query(QueryResult),
    Exec(ExecResult),
    Error(SqlErrorInfo),
}

#[derive(Debug, Serialize)]
pub struct QueryResult {
    pub sql: String,
    pub columns: Vec<String>,
    pub column_meta: Vec<QueryColumnMeta>,
    pub rows: Vec<Vec<Option<String>>>,
    pub elapsed_ms: u128,
}

#[derive(Debug, Serialize)]
pub struct QueryColumnMeta {
    pub name: String,
    pub db_type: String,
    pub field_type: FieldType,
    pub nullable: bool,
}

#[derive(Debug, Serialize)]
pub struct ExecResult {
    pub sql: String,
    pub rows_affected: u64,
    pub elapsed_ms: u128,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SqlErrorInfo {
    pub sql: String,
    pub message: String,
}
#[derive(Debug, Serialize)]
pub enum FieldType {
    Integer,
    Decimal,
    Text,
    LongText,
    Boolean,
    Date,
    Time,
    DateTime,
    Binary,
    Json,
}

impl QueryColumnMeta {
    pub fn new(name: impl Into<String>, db_type: impl Into<String>) -> Self {
        let db_type = db_type.into();
        Self {
            name: name.into(),
            field_type: FieldType::from_db_type(&db_type),
            db_type,
            nullable: true,
        }
    }
}

impl FieldType {
    pub fn from_db_type(db_type: &str) -> Self {
        let upper = db_type.to_uppercase();
        let base_type = upper.split('(').next().unwrap_or(&upper).trim();
        match base_type {
            "INT" | "INTEGER" | "BIGINT" | "SMALLINT" | "TINYINT" | "MEDIUMINT" | "SERIAL"
            | "BIGSERIAL" | "SMALLSERIAL" => Self::Integer,
            "DECIMAL" | "NUMERIC" | "FLOAT" | "DOUBLE" | "REAL" | "DOUBLE PRECISION" | "MONEY" => {
                Self::Decimal
            }
            "BOOL" | "BOOLEAN" | "BIT" => Self::Boolean,
            "DATE" => Self::Date,
            "TIME" => Self::Time,
            "DATETIME" | "TIMESTAMP" | "TIMESTAMPTZ" => Self::DateTime,
            "CHAR" | "VARCHAR" | "NCHAR" | "NVARCHAR" | "CHARACTER VARYING" | "CHARACTER" => {
                Self::Text
            }
            "TEXT" | "LONGTEXT" | "MEDIUMTEXT" | "TINYTEXT" | "CLOB" | "NTEXT" => Self::LongText,
            "BLOB" | "LONGBLOB" | "MEDIUMBLOB" | "TINYBLOB" | "BINARY" | "VARBINARY" | "BYTEA"
            | "IMAGE" => Self::Binary,
            "JSON" | "JSONB" => Self::Json,
            _ => Self::Text,
        }
    }
}

pub fn format_message(sql: &str, rows_affected: u64) -> String {
    let trimmed = sql.trim().to_uppercase();

    if trimmed.starts_with("INSERT") {
        format!("Inserted {rows_affected} row(s)")
    } else if trimmed.starts_with("UPDATE") {
        format!("Updated {rows_affected} row(s)")
    } else if trimmed.starts_with("DELETE") {
        format!("Deleted {rows_affected} row(s)")
    } else if trimmed.starts_with("REPLACE") {
        format!("Replaced {rows_affected} row(s)")
    } else if trimmed.starts_with("CREATE") {
        "Object created successfully".to_string()
    } else if trimmed.starts_with("ALTER") {
        "Object altered successfully".to_string()
    } else if trimmed.starts_with("DROP") {
        "Object dropped successfully".to_string()
    } else if trimmed.starts_with("TRUNCATE") {
        "Table truncated successfully".to_string()
    } else {
        format!("Query executed successfully, {rows_affected} row(s) affected")
    }
}
