use crate::QueryResult;
use one_core::storage::DatabaseType;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

/// SQL value type for parameter binding
#[derive(Debug, Clone)]
pub enum SqlValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    Json(serde_json::Value),
}

/// Database tree node types for hierarchical display
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum DbNodeType {
    #[default]
    Connection,
    Database,
    Schema,
    TablesFolder,
    Table,
    ColumnsFolder,
    Column,
    IndexesFolder,
    Index,
    ForeignKeysFolder,
    ForeignKey,
    TriggersFolder,
    Trigger,
    ChecksFolder,
    Check,
    ViewsFolder,
    View,
    FunctionsFolder,
    Function,
    ProceduresFolder,
    Procedure,
    SequencesFolder,
    Sequence,
    QueriesFolder,
    QueryFolder,
    NamedQuery,
}

impl fmt::Display for DbNodeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbNodeType::Connection => write!(f, "Connection"),
            DbNodeType::Database => write!(f, "Database"),
            DbNodeType::Schema => write!(f, "Schema"),
            DbNodeType::TablesFolder => write!(f, "Tables"),
            DbNodeType::Table => write!(f, "Table"),
            DbNodeType::ColumnsFolder => write!(f, "Columns"),
            DbNodeType::Column => write!(f, "Column"),
            DbNodeType::IndexesFolder => write!(f, "Indexes"),
            DbNodeType::Index => write!(f, "Index"),
            DbNodeType::ForeignKeysFolder => write!(f, "Foreign Keys"),
            DbNodeType::ForeignKey => write!(f, "Foreign Key"),
            DbNodeType::TriggersFolder => write!(f, "Triggers"),
            DbNodeType::Trigger => write!(f, "Trigger"),
            DbNodeType::ChecksFolder => write!(f, "Checks"),
            DbNodeType::Check => write!(f, "Check"),
            DbNodeType::ViewsFolder => write!(f, "Views"),
            DbNodeType::View => write!(f, "View"),
            DbNodeType::FunctionsFolder => write!(f, "Functions"),
            DbNodeType::Function => write!(f, "Function"),
            DbNodeType::ProceduresFolder => write!(f, "Procedures"),
            DbNodeType::Procedure => write!(f, "Procedure"),
            DbNodeType::QueriesFolder => write!(f, "Queries"),
            DbNodeType::QueryFolder => write!(f, "Query Folder"),
            DbNodeType::NamedQuery => write!(f, "Query"),
            DbNodeType::SequencesFolder => write!(f, "Sequences"),
            DbNodeType::Sequence => write!(f, "Sequence"),
        }
    }
}

/// Database tree node for lazy-loading hierarchical display
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DbNode {
    pub id: String,
    pub name: String,
    pub node_type: DbNodeType,
    pub database_type: DatabaseType,
    pub children_loaded: bool,
    pub children: Vec<DbNode>,
    pub metadata: HashMap<String, String>,
    pub connection_id: String,
    pub parent_context: Option<String>,
}

impl PartialEq for DbNode {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for DbNode {}

impl PartialOrd for DbNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DbNode {
    fn cmp(&self, other: &Self) -> Ordering {
        let type_ordering = self.node_type.cmp(&other.node_type);
        if type_ordering != Ordering::Equal {
            return type_ordering;
        }
        let name_ordering = self.name.to_lowercase().cmp(&other.name.to_lowercase());
        if name_ordering != Ordering::Equal {
            return name_ordering;
        }
        self.id.cmp(&other.id)
    }
}

impl DbNode {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        node_type: DbNodeType,
        connection_id: String,
        database_type: DatabaseType,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            node_type,
            children_loaded: false,
            children: Vec::new(),
            metadata: HashMap::new(),
            connection_id,
            parent_context: None,
            database_type,
        }
    }

    pub fn with_children_loaded(mut self, children_loaded: bool) -> Self {
        self.children_loaded = children_loaded;
        self
    }

    pub fn with_metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn with_parent_context(mut self, context: impl Into<String>) -> Self {
        self.parent_context = Some(context.into());
        self
    }

    pub fn sort_children(&mut self) {
        self.children.sort();
    }

    pub fn set_children(&mut self, children: Vec<DbNode>) {
        self.children = children;
        self.children_loaded = true;
    }

    pub fn sort_children_recursive(&mut self) {
        self.children.sort();
        for child in &mut self.children {
            child.sort_children_recursive();
        }
    }

    pub fn get_database_name(&self) -> Option<String> {
        if self.node_type == DbNodeType::Database {
            Some(self.name.clone())
        } else {
            self.metadata.get("database").cloned()
        }
    }

    pub fn get_schema_name(&self) -> Option<String> {
        if self.node_type == DbNodeType::Schema {
            Some(self.name.clone())
        } else {
            self.metadata.get("schema").cloned()
        }
    }

    pub fn get_table_name(&self) -> Option<String> {
        if self.node_type == DbNodeType::Table {
            Some(self.name.clone())
        } else {
            self.metadata.get("table").cloned()
        }
    }
}

/// Database information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DatabaseInfo {
    pub name: String,
    pub charset: Option<String>,
    pub collation: Option<String>,
    pub size: Option<String>,
    pub table_count: Option<i64>,
    pub comment: Option<String>,
}

/// Column information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub is_primary_key: bool,
    pub default_value: Option<String>,
    pub comment: Option<String>,
    /// 列级字符集（如 MySQL 的 CHARACTER_SET_NAME）
    #[serde(default)]
    pub charset: Option<String>,
    /// 列级排序规则（如 MySQL 的 COLLATION_NAME）
    #[serde(default)]
    pub collation: Option<String>,
}

/// Index information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub is_unique: bool,
    #[serde(default)]
    pub is_primary: bool,
    pub index_type: Option<String>,
}

/// Parameters for loading all metadata needed to compare one table-like object.
#[derive(Debug, Clone)]
pub struct DirectTableMetadataRequest {
    pub connection_id: String,
    pub database: String,
    pub schema: Option<String>,
    pub table: String,
    pub include_table_metadata: bool,
}

/// Metadata loaded from one direct database session.
#[derive(Debug, Clone, Default)]
pub struct DirectTableMetadata {
    pub columns: Vec<ColumnInfo>,
    pub indexes: Vec<IndexInfo>,
    pub foreign_keys: Vec<ForeignKeyDefinition>,
}

/// Table information with description/metadata
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableObjectType {
    #[default]
    Table,
    View,
}

/// Table-like object information with description/metadata.
///
/// `list_tables` implementations historically returned only tables, but a few
/// drivers also return views. Keeping the kind here prevents schema compare
/// from generating destructive table DDL for a view.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TableInfo {
    pub name: String,
    #[serde(default)]
    pub object_type: TableObjectType,
    pub schema: Option<String>,
    pub comment: Option<String>,
    pub engine: Option<String>,
    pub create_time: Option<String>,
    pub charset: Option<String>,
    pub collation: Option<String>,
}

/// View information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ViewInfo {
    pub name: String,
    pub schema: Option<String>,
    pub definition: Option<String>,
    pub comment: Option<String>,
}

/// Function information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FunctionInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub return_type: Option<String>,
    pub parameters: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    pub definition: Option<String>,
    pub comment: Option<String>,
}

pub const ROUTINE_NAME_METADATA_KEY: &str = "routine_name";
pub const ROUTINE_IDENTITY_ARGUMENTS_METADATA_KEY: &str = "routine_identity_arguments";
pub const ROUTINE_OBJECT_ID_METADATA_KEY: &str = "routine_object_id";

/// A database routine identity that is sufficient to select overloaded routines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineIdentity {
    pub database: String,
    pub schema: Option<String>,
    pub name: String,
    pub identity_arguments: Option<String>,
    pub object_id: Option<String>,
}

impl RoutineIdentity {
    pub fn from_node(node: &DbNode) -> Option<Self> {
        let database = node.get_database_name()?;
        let schema = node
            .get_schema_name()
            .filter(|schema| !schema.trim().is_empty());
        let name = node
            .metadata
            .get(ROUTINE_NAME_METADATA_KEY)
            .cloned()
            .unwrap_or_else(|| node.name.clone());

        Some(Self {
            database,
            schema,
            name,
            identity_arguments: node
                .metadata
                .get(ROUTINE_IDENTITY_ARGUMENTS_METADATA_KEY)
                .cloned(),
            object_id: node.metadata.get(ROUTINE_OBJECT_ID_METADATA_KEY).cloned(),
        })
    }
}

/// Trigger information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TriggerInfo {
    pub name: String,
    pub table_name: String,
    pub event: String,
    pub timing: String,
    pub definition: Option<String>,
}

/// Sequence information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SequenceInfo {
    pub name: String,
    pub start_value: Option<i64>,
    pub increment: Option<i64>,
    pub min_value: Option<i64>,
    pub max_value: Option<i64>,
}

/// Check constraint information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckInfo {
    pub name: String,
    pub table_name: String,
    pub definition: Option<String>,
}

// === SQL Operation Request Objects ===

#[derive(Debug, Clone)]
pub struct CreateDatabaseRequest {
    pub database_name: String,
    pub charset: Option<String>,
    pub collation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DropDatabaseRequest {
    pub database_name: String,
    pub if_exists: bool,
}

#[derive(Debug, Clone)]
pub struct AlterDatabaseRequest {
    pub database_name: String,
    pub charset: Option<String>,
    pub collation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateTableRequest {
    pub database_name: String,
    pub table_name: String,
    pub columns: Vec<ColumnInfo>,
    pub if_not_exists: bool,
}

#[derive(Debug, Clone)]
pub struct DropTableRequest {
    pub database_name: String,
    pub table_name: String,
    pub if_exists: bool,
}

#[derive(Debug, Clone)]
pub struct RenameTableRequest {
    pub database_name: String,
    pub old_table_name: String,
    pub new_table_name: String,
}

#[derive(Debug, Clone)]
pub struct TruncateTableRequest {
    pub database_name: String,
    pub table_name: String,
}

#[derive(Debug, Clone)]
pub struct AddColumnRequest {
    pub database_name: String,
    pub table_name: String,
    pub column: ColumnInfo,
}

#[derive(Debug, Clone)]
pub struct DropColumnRequest {
    pub database_name: String,
    pub table_name: String,
    pub column_name: String,
}

#[derive(Debug, Clone)]
pub struct ModifyColumnRequest {
    pub database_name: String,
    pub table_name: String,
    pub column: ColumnInfo,
}

#[derive(Debug, Clone)]
pub struct CreateIndexRequest {
    pub database_name: String,
    pub table_name: String,
    pub index: IndexInfo,
}

#[derive(Debug, Clone)]
pub struct DropIndexRequest {
    pub database_name: String,
    pub table_name: String,
    pub index_name: String,
}

#[derive(Debug, Clone)]
pub struct CreateViewRequest {
    pub database_name: String,
    pub view_name: String,
    pub definition: String,
    pub or_replace: bool,
}

#[derive(Debug, Clone)]
pub struct DropViewRequest {
    pub database_name: String,
    pub view_name: String,
    pub if_exists: bool,
}

#[derive(Debug, Clone)]
pub struct CreateFunctionRequest {
    pub database_name: String,
    pub definition: String,
}

#[derive(Debug, Clone)]
pub struct DropFunctionRequest {
    pub database_name: String,
    pub function_name: String,
    pub if_exists: bool,
}

#[derive(Debug, Clone)]
pub struct CreateProcedureRequest {
    pub database_name: String,
    pub definition: String,
}

#[derive(Debug, Clone)]
pub struct DropProcedureRequest {
    pub database_name: String,
    pub procedure_name: String,
    pub if_exists: bool,
}

#[derive(Debug, Clone)]
pub struct CreateTriggerRequest {
    pub database_name: String,
    pub definition: String,
}

#[derive(Debug, Clone)]
pub struct DropTriggerRequest {
    pub database_name: String,
    pub trigger_name: String,
    pub if_exists: bool,
}

#[derive(Debug, Clone)]
pub struct CreateSequenceRequest {
    pub database_name: String,
    pub sequence: SequenceInfo,
}

#[derive(Debug, Clone)]
pub struct DropSequenceRequest {
    pub database_name: String,
    pub sequence_name: String,
    pub if_exists: bool,
}

#[derive(Debug, Clone)]
pub struct AlterSequenceRequest {
    pub database_name: String,
    pub sequence: SequenceInfo,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ObjectViewColumnAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ObjectViewColumn {
    pub key: String,
    pub label: String,
    pub width_px: f32,
    pub align: ObjectViewColumnAlign,
    pub resizable: bool,
}

impl ObjectViewColumn {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            width_px: 100.0,
            align: ObjectViewColumnAlign::Left,
            resizable: true,
        }
    }

    pub fn localized(key: impl Into<String>, label_i18n_key: &str) -> Self {
        Self::new(
            key,
            crate::translate_or_raw_for_locale(rust_i18n::locale().as_ref(), label_i18n_key),
        )
    }

    pub fn width(mut self, width_px: f32) -> Self {
        self.width_px = width_px;
        self
    }

    pub fn text_center(mut self) -> Self {
        self.align = ObjectViewColumnAlign::Center;
        self
    }

    pub fn text_right(mut self) -> Self {
        self.align = ObjectViewColumnAlign::Right;
        self
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ObjectView {
    pub db_node_type: DbNodeType,
    pub title: String,
    pub columns: Vec<ObjectViewColumn>,
    pub rows: Vec<Vec<String>>,
}

// === Table Data Query Types ===

/// Abstract data type for UI rendering
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FieldType {
    /// Integer numbers (INT, BIGINT, SMALLINT, etc.)
    Integer,
    /// Decimal numbers (DECIMAL, NUMERIC, FLOAT, DOUBLE, etc.)
    Decimal,
    /// Short text (VARCHAR, CHAR)
    Text,
    /// Long text (TEXT, LONGTEXT, CLOB)
    LongText,
    /// Boolean (BOOL, BOOLEAN, BIT)
    Boolean,
    /// Date only (DATE)
    Date,
    /// Time only (TIME)
    Time,
    /// Date and time (DATETIME, TIMESTAMP)
    DateTime,
    /// Binary data (BLOB, BINARY, BYTEA)
    Binary,
    /// JSON data
    Json,
    /// Unknown or unsupported type
    Unknown,
}

impl FieldType {
    /// Infer field type from database type string
    pub fn from_db_type(db_type: &str) -> Self {
        let mut normalized = db_type.trim().to_uppercase();

        // ClickHouse wraps the actual type in Nullable/LowCardinality.
        loop {
            let unwrapped = normalized
                .strip_prefix("NULLABLE(")
                .or_else(|| normalized.strip_prefix("LOWCARDINALITY("))
                .and_then(|value| value.strip_suffix(')'));
            let Some(inner) = unwrapped else {
                break;
            };
            normalized = inner.trim().to_string();
        }

        if let Some(type_name) = normalized.strip_prefix("MYSQL_TYPE_") {
            normalized = type_name.to_string();
        }

        // MySQL may append these attributes to a type name.
        normalized = normalized
            .trim_end_matches(" ZEROFILL")
            .trim_end_matches(" UNSIGNED")
            .trim()
            .to_string();

        let base_type = normalized.split('(').next().unwrap_or(&normalized).trim();

        // Preserve the full prefix for qualified types such as
        // Oracle TIMESTAMP(6) WITH TIME ZONE and ClickHouse DateTime64(3).
        if normalized.starts_with("TIMESTAMP") || normalized.starts_with("DATETIME") {
            return Self::DateTime;
        }

        match base_type {
            // SQL dialects, protocol enums, and Arrow integer names.
            "INT" | "INTEGER" | "BIGINT" | "SMALLINT" | "TINYINT" | "MEDIUMINT" | "SERIAL"
            | "BIGSERIAL" | "SMALLSERIAL" | "INT1" | "INT2" | "INT4" | "INT8" | "INT16"
            | "INT32" | "INT64" | "INT128" | "INT256" | "UINT8" | "UINT16" | "UINT32"
            | "UINT64" | "UINT128" | "UINT256" | "HUGEINT" | "UTINYINT" | "USMALLINT" | "UINT"
            | "UBIGINT" | "OID" | "TINY" | "SHORT" | "LONG" | "LONGLONG" | "INT24" | "YEAR" => {
                Self::Integer
            }
            // SQL dialects, protocol enums, Arrow, and Oracle decimals.
            "DECIMAL" | "NUMERIC" | "FLOAT" | "DOUBLE" | "REAL" | "DOUBLE PRECISION" | "MONEY"
            | "FLOAT4" | "FLOAT8" | "FLOAT16" | "FLOAT32" | "FLOAT64" | "FLOATN" | "DECIMALN"
            | "NUMERICN" | "MONEY4" | "NEWDECIMAL" | "NUMBER" | "BINARY_FLOAT"
            | "BINARY_DOUBLE" | "DECIMAL32" | "DECIMAL64" | "DECIMAL128" | "DECIMAL256" => {
                Self::Decimal
            }
            // BIT remains Boolean for MSSQL and one-bit MySQL columns. A
            // length-aware metadata model is needed to classify multi-bit
            // protocol values more precisely.
            "BOOL" | "BOOLEAN" | "BIT" | "BITN" => Self::Boolean,
            "DATE" | "DATE32" | "DATE64" | "DATEN" | "NEWDATE" => Self::Date,
            "TIME" | "TIME32" | "TIME64" | "TIMEN" => Self::Time,
            "CHAR" | "VARCHAR" | "NCHAR" | "NVARCHAR" | "VARCHAR2" | "NVARCHAR2" | "BPCHAR"
            | "NAME" | "STRING" | "FIXEDSTRING" | "CHARACTER VARYING" | "CHARACTER"
            | "BIGVARCHAR" | "BIGCHAR" | "GUID" | "UUID" | "XML" | "ENUM" | "SET"
            | "VAR_STRING" => Self::Text,
            "TEXT" | "LONGTEXT" | "MEDIUMTEXT" | "TINYTEXT" | "CLOB" | "NCLOB" | "NTEXT" => {
                Self::LongText
            }
            "BLOB" | "LONGBLOB" | "LONG_BLOB" | "MEDIUMBLOB" | "MEDIUM_BLOB" | "TINYBLOB"
            | "TINY_BLOB" | "BINARY" | "VARBINARY" | "BYTEA" | "IMAGE" | "RAW" | "LONG RAW"
            | "BFILE" | "BIGVARBIN" | "BIGBINARY" | "LARGEBINARY" | "GEOMETRY" | "VECTOR" => {
                Self::Binary
            }
            "JSON" | "JSONB" | "ARRAY" | "MAP" | "TUPLE" | "LIST" | "STRUCT" => Self::Json,
            _ if base_type.starts_with("ARRAY")
                || base_type.starts_with("MAP")
                || base_type.starts_with("TUPLE")
                || base_type.starts_with("LIST")
                || base_type.starts_with("STRUCT") =>
            {
                Self::Json
            }
            _ if base_type.starts_with("UTF8") || base_type.starts_with("LARGEUTF8") => Self::Text,
            _ if base_type.starts_with("BINARY") || base_type.starts_with("LARGEBINARY") => {
                Self::Binary
            }
            _ => Self::Unknown,
        }
    }
}

/// Column metadata for table data display
#[derive(Debug, Clone)]
pub struct TableColumnMeta {
    /// Column name
    pub name: String,
    /// Original database type (e.g., "VARCHAR(255)")
    pub db_type: String,
    /// Abstract field type for UI rendering
    pub field_type: FieldType,
    /// Whether the column is nullable
    pub nullable: bool,
    /// Whether the column is a primary key
    pub is_primary_key: bool,
    /// Column index in the result set
    pub index: usize,
}

/// A cell value used while persisting table edits.
///
/// This type deliberately distinguishes SQL `NULL` from text values such as
/// the empty string or the literal string `"NULL"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableCellValue {
    Null,
    Text(String),
    Binary(Vec<u8>),
}

impl TableCellValue {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Null | Self::Binary(_) => None,
            Self::Text(value) => Some(value),
        }
    }

    pub fn as_binary(&self) -> Option<&[u8]> {
        match self {
            Self::Binary(bytes) => Some(bytes),
            Self::Null | Self::Text(_) => None,
        }
    }
}

impl From<Option<String>> for TableCellValue {
    fn from(value: Option<String>) -> Self {
        match value {
            Some(value) => Self::Text(value),
            None => Self::Null,
        }
    }
}

impl From<String> for TableCellValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for TableCellValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

/// Represents a single cell change when persisting table edits
#[derive(Debug, Clone)]
pub struct TableCellChange {
    pub column_index: usize,
    pub column_name: String,
    pub old_value: TableCellValue,
    pub new_value: TableCellValue,
}

/// Represents a table row change for persistence operations
#[derive(Debug, Clone)]
pub enum TableRowChange {
    Added {
        data: Vec<TableCellValue>,
    },
    Updated {
        original_data: Vec<TableCellValue>,
        changes: Vec<TableCellChange>,
        rowid: Option<String>,
    },
    Deleted {
        original_data: Vec<TableCellValue>,
        rowid: Option<String>,
    },
}

/// Request payload for saving table edits back to the database
#[derive(Debug, Clone)]
pub struct TableSaveRequest {
    pub database: String,
    pub schema: Option<String>,
    pub table: String,
    pub columns: Vec<ColumnInfo>,
    pub index_infos: Vec<IndexInfo>,
    pub changes: Vec<TableRowChange>,
}

/// Request for generating copy SQL (INSERT, UPDATE, DELETE statements)
#[derive(Debug, Clone)]
pub struct CopySqlRequest {
    /// Schema name (optional, for databases that support schemas)
    pub schema: Option<String>,
    /// Table name
    pub table: String,
    /// Column information
    pub columns: Vec<ColumnInfo>,
    /// Row data to generate SQL for
    pub rows: Vec<Vec<TableCellValue>>,
    /// Original row data (for UPDATE statements, used to generate WHERE clause)
    pub original_rows: Option<Vec<Vec<TableCellValue>>>,
    /// Column names
    pub column_names: Vec<String>,
}

impl CopySqlRequest {
    pub fn new(table: impl Into<String>, columns: Vec<ColumnInfo>) -> Self {
        let column_names = columns.iter().map(|c| c.name.clone()).collect();
        Self {
            schema: None,
            table: table.into(),
            columns,
            rows: Vec::new(),
            original_rows: None,
            column_names,
        }
    }

    pub fn with_schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    pub fn with_rows(mut self, rows: Vec<Vec<Option<String>>>) -> Self {
        self.rows = rows
            .into_iter()
            .map(|row| row.into_iter().map(TableCellValue::from).collect())
            .collect();
        self
    }

    pub fn with_original_rows(mut self, original_rows: Vec<Vec<Option<String>>>) -> Self {
        self.original_rows = Some(
            original_rows
                .into_iter()
                .map(|row| row.into_iter().map(TableCellValue::from).collect())
                .collect(),
        );
        self
    }

    pub fn with_typed_rows(mut self, rows: Vec<Vec<TableCellValue>>) -> Self {
        self.rows = rows;
        self
    }

    pub fn with_typed_original_rows(mut self, original_rows: Vec<Vec<TableCellValue>>) -> Self {
        self.original_rows = Some(original_rows);
        self
    }

    pub fn with_column_names(mut self, column_names: Vec<String>) -> Self {
        self.column_names = column_names;
        self
    }
}

/// 复制为 SQL 的类型
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyAsSqlType {
    /// INSERT 语句
    Insert,
    /// INSERT 语句（带注释）
    InsertWithComments,
    /// UPDATE 语句
    Update,
    /// DELETE 语句
    Delete,
}

/// Response from applying table edits
#[derive(Debug, Clone)]
pub struct TableSaveResponse {
    pub success_count: usize,
    pub errors: Vec<String>,
}

/// Request for querying table data with pagination and optional SQL clauses
#[derive(Debug, Clone, Default)]
pub struct TableDataRequest {
    /// Database name
    pub database: String,
    /// Schema name (for databases that support schemas like PostgreSQL, MSSQL)
    pub schema: Option<String>,
    /// Table name
    pub table: String,
    /// Page number (1-based)
    pub page: usize,
    /// Page size
    pub page_size: usize,
    /// Explicit row offset. When absent, it is derived from `page` and `page_size`.
    ///
    /// Callers that vary `page_size` between requests must set this field to
    /// avoid overlapping or skipping rows.
    pub offset: Option<usize>,
    /// Previously observed total row count for the same filtered query.
    ///
    /// Compare pagination can reuse this after the first page so later pages
    /// do not repeat an expensive `COUNT(*)`.
    pub known_total_count: Option<usize>,
    /// Raw WHERE clause (e.g., "id > 10 AND name LIKE '%test%'")
    pub where_clause: Option<String>,
    /// Raw ORDER BY clause (e.g., "id DESC, name ASC")
    pub order_by_clause: Option<String>,
}

impl TableDataRequest {
    pub fn new(database: impl Into<String>, table: impl Into<String>) -> Self {
        Self {
            database: database.into(),
            schema: None,
            table: table.into(),
            page: 1,
            page_size: 100,
            offset: None,
            known_total_count: None,
            where_clause: None,
            order_by_clause: None,
        }
    }

    pub fn with_schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    pub fn with_page(mut self, page: usize, page_size: usize) -> Self {
        self.page = page;
        self.page_size = page_size;
        self
    }

    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }

    pub fn with_known_total_count(mut self, total_count: usize) -> Self {
        self.known_total_count = Some(total_count);
        self
    }

    pub fn effective_offset(&self) -> usize {
        self.offset
            .unwrap_or_else(|| self.page.saturating_sub(1).saturating_mul(self.page_size))
    }

    pub fn with_where_clause(mut self, clause: impl Into<String>) -> Self {
        let c = clause.into();
        self.where_clause = if c.is_empty() { None } else { Some(c) };
        self
    }

    pub fn with_order_by_clause(mut self, clause: impl Into<String>) -> Self {
        let c = clause.into();
        self.order_by_clause = if c.is_empty() { None } else { Some(c) };
        self
    }
}

/// Response for table data query
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TableDataResponse {
    /// Row data (each cell is Option<String>, None means NULL)
    pub query_result: QueryResult,
    /// Total row count (for pagination)
    pub total_count: usize,
    /// Current page
    pub page: usize,
    /// Page size
    pub page_size: usize,
    /// Duration of the query
    pub duration: u128,
}

/// Character set information
#[derive(Debug, Clone)]
pub struct CharsetInfo {
    pub name: String,
    pub description: String,
    pub default_collation: String,
}

/// Collation information
#[derive(Debug, Clone)]
pub struct CollationInfo {
    pub name: String,
    pub charset: String,
    pub is_default: bool,
}

// === Table Designer Types ===

/// Detailed column definition for table designer
#[derive(Debug, Clone, Default)]
pub struct ColumnDefinition {
    pub name: String,
    pub data_type: String,
    pub length: Option<u32>,
    pub precision: Option<u32>,
    pub scale: Option<u32>,
    pub is_nullable: bool,
    pub is_primary_key: bool,
    pub is_auto_increment: bool,
    pub is_unsigned: bool,
    pub default_value: Option<String>,
    pub comment: String,
    pub charset: Option<String>,
    pub collation: Option<String>,
}

impl ColumnDefinition {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_nullable: true,
            ..Default::default()
        }
    }

    pub fn data_type(mut self, data_type: impl Into<String>) -> Self {
        self.data_type = data_type.into();
        self
    }

    pub fn length(mut self, length: u32) -> Self {
        self.length = Some(length);
        self
    }

    pub fn nullable(mut self, nullable: bool) -> Self {
        self.is_nullable = nullable;
        self
    }

    pub fn primary_key(mut self, pk: bool) -> Self {
        self.is_primary_key = pk;
        self
    }

    pub fn auto_increment(mut self, ai: bool) -> Self {
        self.is_auto_increment = ai;
        self
    }

    pub fn default_value(mut self, value: impl Into<String>) -> Self {
        self.default_value = Some(value.into());
        self
    }

    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = comment.into();
        self
    }
}

/// Index definition for table designer
#[derive(Debug, Clone, Default)]
pub struct IndexDefinition {
    pub name: String,
    pub columns: Vec<String>,
    pub is_unique: bool,
    pub is_primary: bool,
    pub index_type: Option<String>,
    pub comment: String,
}

impl IndexDefinition {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    pub fn columns(mut self, columns: Vec<String>) -> Self {
        self.columns = columns;
        self
    }

    pub fn unique(mut self, unique: bool) -> Self {
        self.is_unique = unique;
        self
    }

    pub fn primary(mut self, primary: bool) -> Self {
        self.is_primary = primary;
        self
    }
}

/// Foreign key definition
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ForeignKeyDefinition {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_table: String,
    /// Schema containing the referenced table. `None` means the target schema/default namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_schema: Option<String>,
    pub ref_columns: Vec<String>,
    pub on_delete: String,
    pub on_update: String,
}

/// Table options (engine, charset, etc.)
#[derive(Debug, Clone, Default)]
pub struct TableOptions {
    pub engine: Option<String>,
    pub charset: Option<String>,
    pub collation: Option<String>,
    pub comment: String,
    pub auto_increment: Option<u64>,
}

/// Complete table design
#[derive(Debug, Clone, Default)]
pub struct TableDesign {
    pub database_name: String,
    pub table_name: String,
    pub columns: Vec<ColumnDefinition>,
    pub indexes: Vec<IndexDefinition>,
    pub foreign_keys: Vec<ForeignKeyDefinition>,
    pub options: TableOptions,
}

impl TableDesign {
    pub fn new(database_name: impl Into<String>, table_name: impl Into<String>) -> Self {
        Self {
            database_name: database_name.into(),
            table_name: table_name.into(),
            ..Default::default()
        }
    }

    pub fn add_column(&mut self, column: ColumnDefinition) {
        self.columns.push(column);
    }

    pub fn add_index(&mut self, index: IndexDefinition) {
        self.indexes.push(index);
    }

    pub fn primary_key_columns(&self) -> Vec<&str> {
        self.columns
            .iter()
            .filter(|c| c.is_primary_key)
            .map(|c| c.name.as_str())
            .collect()
    }
}

/// Parsed column type information
#[derive(Debug, Clone, Default)]
pub struct ParsedColumnType {
    pub base_type: String,
    pub length: Option<u32>,
    pub scale: Option<u32>,
    pub enum_values: Option<String>,
    pub is_unsigned: bool,
    pub is_auto_increment: bool,
}

impl ParsedColumnType {
    pub fn new(base_type: impl Into<String>) -> Self {
        Self {
            base_type: base_type.into(),
            ..Default::default()
        }
    }

    pub fn with_length(mut self, length: u32) -> Self {
        self.length = Some(length);
        self
    }

    pub fn with_scale(mut self, scale: u32) -> Self {
        self.scale = Some(scale);
        self
    }

    pub fn with_enum_values(mut self, values: impl Into<String>) -> Self {
        self.enum_values = Some(values.into());
        self
    }

    pub fn with_unsigned(mut self, unsigned: bool) -> Self {
        self.is_unsigned = unsigned;
        self
    }

    pub fn with_auto_increment(mut self, auto_increment: bool) -> Self {
        self.is_auto_increment = auto_increment;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_view_columns_are_serializable() {
        let view = ObjectView {
            db_node_type: DbNodeType::Table,
            title: "Tables".to_string(),
            columns: vec![
                ObjectViewColumn::new("name", "Name").width(180.0),
                ObjectViewColumn::new("rows", "Rows")
                    .width(100.0)
                    .text_right(),
            ],
            rows: vec![vec!["users".to_string(), "12".to_string()]],
        };

        let json = serde_json::to_string(&view).expect("ObjectView should serialize");
        let restored: ObjectView =
            serde_json::from_str(&json).expect("ObjectView should deserialize");

        assert_eq!("Tables", restored.title);
        assert_eq!("rows", restored.columns[1].key);
        assert_eq!("Rows", restored.columns[1].label);
        assert_eq!(ObjectViewColumnAlign::Right, restored.columns[1].align);
        assert_eq!(
            vec![vec!["users".to_string(), "12".to_string()]],
            restored.rows
        );
    }

    #[test]
    fn table_data_response_deserializes_from_external_driver_value() {
        let value = serde_json::json!({
            "query_result": {
                "sql": "SELECT _id FROM logs",
                "columns": ["_id"],
                "column_meta": [],
                "rows": [["abc"]],
                "elapsed_ms": 3
            },
            "total_count": 1,
            "page": 1,
            "page_size": 100,
            "duration": 3
        });

        let response: TableDataResponse =
            serde_json::from_value(value).expect("table data response decodes");

        assert_eq!(response.total_count, 1);
        assert_eq!(response.query_result.columns, vec!["_id"]);
    }

    #[test]
    fn field_type_normalizes_driver_specific_type_tokens() {
        assert_eq!(
            FieldType::from_db_type("MYSQL_TYPE_LONG"),
            FieldType::Integer
        );
        assert_eq!(
            FieldType::from_db_type("MYSQL_TYPE_NEWDECIMAL"),
            FieldType::Decimal
        );
        assert_eq!(
            FieldType::from_db_type("MYSQL_TYPE_LONG_BLOB"),
            FieldType::Binary
        );
        assert_eq!(
            FieldType::from_db_type("MYSQL_TYPE_MEDIUM_BLOB"),
            FieldType::Binary
        );
        assert_eq!(
            FieldType::from_db_type("MYSQL_TYPE_TINY_BLOB"),
            FieldType::Binary
        );
        for data_type in [
            "CHAR(10)",
            "BPCHAR",
            "NCHAR(10)",
            "BIGCHAR",
            "MYSQL_TYPE_STRING",
            "FixedString(10)",
            "Nullable(FixedString(10))",
        ] {
            assert_eq!(
                FieldType::from_db_type(data_type),
                FieldType::Text,
                "{data_type}"
            );
        }
        assert_eq!(FieldType::from_db_type("int4"), FieldType::Integer);
        assert_eq!(FieldType::from_db_type("float8"), FieldType::Decimal);
        assert_eq!(FieldType::from_db_type("Int4"), FieldType::Integer);
        assert_eq!(
            FieldType::from_db_type("DatetimeOffsetn"),
            FieldType::DateTime
        );
        assert_eq!(FieldType::from_db_type("NUMBER(10, 2)"), FieldType::Decimal);
        assert_eq!(
            FieldType::from_db_type("Nullable(UInt64)"),
            FieldType::Integer
        );
        assert_eq!(
            FieldType::from_db_type("LowCardinality(String)"),
            FieldType::Text
        );
        assert_eq!(
            FieldType::from_db_type("DateTime64(3)"),
            FieldType::DateTime
        );
        assert_eq!(
            FieldType::from_db_type("vendor_custom_type"),
            FieldType::Unknown
        );
    }

    #[test]
    fn field_type_maps_tdengine_describe_types() {
        // TDengine DESCRIBE 输出的类型走 MySQL 方言同臂:
        // TIMESTAMP → DateTime,NCHAR(n) → Text,BINARY(n) → Binary。
        assert_eq!(FieldType::from_db_type("TIMESTAMP"), FieldType::DateTime);
        assert_eq!(FieldType::from_db_type("NCHAR(8)"), FieldType::Text);
        assert_eq!(FieldType::from_db_type("BINARY(16)"), FieldType::Binary);
        assert_eq!(FieldType::from_db_type("VARCHAR(64)"), FieldType::Text);
        assert_eq!(FieldType::from_db_type("INT UNSIGNED"), FieldType::Integer);
        assert_eq!(
            FieldType::from_db_type("BIGINT UNSIGNED"),
            FieldType::Integer
        );
        assert_eq!(FieldType::from_db_type("BOOL"), FieldType::Boolean);
        assert_eq!(FieldType::from_db_type("JSON"), FieldType::Json);
    }

    #[test]
    fn table_data_request_explicit_offset_overrides_page_derived_offset() {
        let request = TableDataRequest::new("app", "users")
            .with_page(2, 5_000)
            .with_offset(10_000);

        assert_eq!(request.effective_offset(), 10_000);
    }

    #[test]
    fn table_data_request_falls_back_to_page_derived_offset() {
        let request = TableDataRequest::new("app", "users").with_page(3, 250);

        assert_eq!(request.effective_offset(), 500);
    }

    #[test]
    fn table_data_request_keeps_known_total_count() {
        let request = TableDataRequest::new("app", "users").with_known_total_count(42);

        assert_eq!(request.known_total_count, Some(42));
    }
}
