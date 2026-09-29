use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use one_core::storage::DbConnectionConfig;
use tiberius::{AuthMethod, Client, ColumnType, Config, Row, Uuid};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};
use tracing::{debug, error, info};

use crate::connection::{DbConnection, DbError, StreamingProgress};
use crate::executor::{
    BinaryCell, ExecOptions, ExecResult, QueryColumnMeta, QueryResult, SqlErrorInfo, SqlResult,
    SqlSource,
};
use crate::ssh_tunnel::resolve_connection_target;
use crate::types::FieldType;
use crate::{DatabasePlugin, format_message, truncate_str};
use connection_tunnel::TunnelGuard;
use db_value::{
    CellState, ColumnDescriptor, DbValue, FloatWidth, Nullability, RawPayload, RawRepresentation,
    ResultBatch, ResultRow,
};

/// MSSQL 标量类型对应的 typed 解码目标。
///
/// TDS 对可空列使用变长包装类型：可空 `bit` 走 `BITN`、可空 `int` 走 `INTN`、
/// 可空 `float` 走 `FLOATN` 等。系统目录视图里的 `is_nullable`、`is_unique`
/// 这类列几乎都是可空 `bit`，因此变长包装类型必须与定长类型一起建模，否则
/// 列/索引元数据读取会因为单个单元格无法解码而整条失败。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MssqlScalarDecoder {
    Bit,
    Null,
    Int1,
    Int2,
    Int4,
    Int8,
    Money,
    Float4,
    Float8,
    Guid,
    Decimal,
    DateTime,
    Date,
    Time,
    DateTimeOffset,
}

impl MssqlScalarDecoder {
    /// 未建模类型（如 `Udt`、`SSVariant`）返回 `None`，由调用方标为 `Undecoded`。
    fn for_column_type(column_type: ColumnType) -> Option<Self> {
        use ColumnType::*;
        Some(match column_type {
            Bit | Bitn => Self::Bit,
            Null => Self::Null,
            Int1 => Self::Int1,
            Int2 => Self::Int2,
            Int4 => Self::Int4,
            // `Intn` 只会在不常见列宽下出现，按最宽整数兜底。
            Int8 | Intn => Self::Int8,
            Money | Money4 => Self::Money,
            Float4 => Self::Float4,
            Float8 | Floatn => Self::Float8,
            Guid => Self::Guid,
            Decimaln | Numericn => Self::Decimal,
            Datetime | Datetimen | Datetime2 => Self::DateTime,
            Daten => Self::Date,
            Timen => Self::Time,
            DatetimeOffsetn => Self::DateTimeOffset,
            _ => return None,
        })
    }
}

pub struct MssqlDbConnection {
    config: DbConnectionConfig,
    client: Arc<Mutex<Option<Client<Compat<TcpStream>>>>>,
    tunnel: Option<TunnelGuard>,
}

impl MssqlDbConnection {
    pub fn new(config: DbConnectionConfig) -> Self {
        Self {
            config,
            client: Arc::new(Mutex::new(None)),
            tunnel: None,
        }
    }

    fn extract_value(row: &Row, index: usize) -> Option<String> {
        row.try_get::<&str, _>(index)
            .ok()
            .flatten()
            .map(|s| s.to_string())
            .or_else(|| {
                row.try_get::<i32, _>(index)
                    .ok()
                    .flatten()
                    .map(|v| v.to_string())
            })
            .or_else(|| {
                row.try_get::<i64, _>(index)
                    .ok()
                    .flatten()
                    .map(|v| v.to_string())
            })
            .or_else(|| {
                row.try_get::<f64, _>(index)
                    .ok()
                    .flatten()
                    .map(|v| v.to_string())
            })
            .or_else(|| {
                row.try_get::<bool, _>(index)
                    .ok()
                    .flatten()
                    .map(|v| v.to_string())
            })
            .or_else(|| {
                use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

                row.try_get::<NaiveDateTime, _>(index)
                    .ok()
                    .flatten()
                    .map(|v| v.format("%Y-%m-%d %H:%M:%S").to_string())
                    .or_else(|| {
                        row.try_get::<NaiveDate, _>(index)
                            .ok()
                            .flatten()
                            .map(|v| v.format("%Y-%m-%d").to_string())
                    })
                    .or_else(|| {
                        row.try_get::<NaiveTime, _>(index)
                            .ok()
                            .flatten()
                            .map(|v| v.format("%H:%M:%S").to_string())
                    })
            })
    }

    fn is_character_column_type(column_type: ColumnType) -> bool {
        matches!(
            column_type,
            ColumnType::BigVarChar
                | ColumnType::BigChar
                | ColumnType::NVarchar
                | ColumnType::NChar
                | ColumnType::Text
                | ColumnType::NText
                | ColumnType::Xml
        )
    }

    fn is_binary_column_type(column_type: ColumnType) -> bool {
        matches!(
            column_type,
            ColumnType::BigVarBin | ColumnType::BigBinary | ColumnType::Image
        )
    }

    /// 处理 `try_get` 的解码结果:成功值、SQL NULL 与解码失败严格分开。
    fn try_cell<T>(
        result: tiberius::Result<Option<T>>,
        native_type: ColumnType,
        to_value: impl FnOnce(T) -> (DbValue, String),
    ) -> (CellState, Option<String>) {
        match result {
            Ok(Some(value)) => {
                let (typed, display) = to_value(value);
                (CellState::Decoded(typed), Some(display))
            }
            Ok(None) => (CellState::Decoded(DbValue::Null), None),
            Err(error) => Self::decode_failure(native_type, None, error.to_string()),
        }
    }

    fn decode_failure(
        native_type: ColumnType,
        raw: Option<Vec<u8>>,
        diagnostic: String,
    ) -> (CellState, Option<String>) {
        let type_name = format!("{native_type:?}");
        let display = match &raw {
            Some(bytes) => format!("0x{}", hex::encode(bytes)),
            None => format!("<{type_name} decode error>"),
        };
        (
            CellState::DecodeError {
                native_type: type_name,
                raw: raw.map(|bytes| RawPayload {
                    bytes,
                    representation: RawRepresentation::DatabaseValueBytes,
                }),
                diagnostic,
            },
            Some(display),
        )
    }

    /// 按 Tiberius 列类型分派解码:binary/text 由列类型决定,不再靠字节内容猜测。
    fn extract_cell(
        row: &Row,
        index: usize,
        column_type: ColumnType,
    ) -> (CellState, Option<String>) {
        if Self::is_binary_column_type(column_type) {
            return match row.try_get::<&[u8], _>(index) {
                Ok(Some(bytes)) => {
                    let bytes = bytes.to_vec();
                    (
                        CellState::Decoded(DbValue::Binary(bytes.clone())),
                        Some(format!("0x{}", hex::encode(bytes))),
                    )
                }
                Ok(None) => (CellState::Decoded(DbValue::Null), None),
                Err(error) => Self::decode_failure(column_type, None, error.to_string()),
            };
        }

        if matches!(column_type, ColumnType::Xml) {
            return match row.try_get::<&tiberius::xml::XmlData, _>(index) {
                Ok(Some(xml)) => {
                    let text = xml.as_ref().to_string();
                    (CellState::Decoded(DbValue::Text(text.clone())), Some(text))
                }
                Ok(None) => (CellState::Decoded(DbValue::Null), None),
                Err(error) => Self::decode_failure(column_type, None, error.to_string()),
            };
        }

        if Self::is_character_column_type(column_type) {
            return match row.try_get::<&str, _>(index) {
                Ok(Some(text)) => {
                    let text = text.to_string();
                    (CellState::Decoded(DbValue::Text(text.clone())), Some(text))
                }
                Ok(None) => (CellState::Decoded(DbValue::Null), None),
                Err(error) => match row.try_get::<&[u8], _>(index) {
                    Ok(Some(bytes)) => {
                        Self::decode_failure(column_type, Some(bytes.to_vec()), error.to_string())
                    }
                    _ => Self::decode_failure(column_type, None, error.to_string()),
                },
            };
        }

        let Some(decoder) = MssqlScalarDecoder::for_column_type(column_type) else {
            // 未建模类型:保留 legacy 显示,typed 明确 Undecoded,不伪装成值。
            let display = Self::extract_value(row, index);
            return (
                CellState::Undecoded {
                    native_type: format!("{column_type:?}"),
                    raw: None,
                    reason: "no typed decoder for this MSSQL column type".to_string(),
                },
                display,
            );
        };

        match decoder {
            MssqlScalarDecoder::Bit => {
                Self::try_cell(row.try_get::<bool, _>(index), column_type, |v| {
                    (DbValue::Bool(v), v.to_string())
                })
            }
            MssqlScalarDecoder::Null => (CellState::Decoded(DbValue::Null), None),
            MssqlScalarDecoder::Int1 => {
                Self::try_cell(row.try_get::<u8, _>(index), column_type, |v| {
                    let s = v.to_string();
                    (DbValue::Integer(s.clone()), s)
                })
            }
            MssqlScalarDecoder::Int2 => {
                Self::try_cell(row.try_get::<i16, _>(index), column_type, |v| {
                    let s = v.to_string();
                    (DbValue::Integer(s.clone()), s)
                })
            }
            MssqlScalarDecoder::Int4 => {
                Self::try_cell(row.try_get::<i32, _>(index), column_type, |v| {
                    let s = v.to_string();
                    (DbValue::Integer(s.clone()), s)
                })
            }
            MssqlScalarDecoder::Int8 => {
                Self::try_cell(row.try_get::<i64, _>(index), column_type, |v| {
                    let s = v.to_string();
                    (DbValue::Integer(s.clone()), s)
                })
            }
            // Driver limitation: tiberius decodes TDS money as `ColumnData::F64`
            // after already losing the exact scaled integer, so large money values
            // cannot be recovered with full precision here. Kept as Decimal text
            // for display; not claimed as exact.
            MssqlScalarDecoder::Money => {
                Self::try_cell(row.try_get::<f64, _>(index), column_type, |v| {
                    let s = v.to_string();
                    (DbValue::Decimal(s.clone()), s)
                })
            }
            MssqlScalarDecoder::Float4 => {
                Self::try_cell(row.try_get::<f32, _>(index), column_type, |v| {
                    let s = v.to_string();
                    (
                        DbValue::Float {
                            value: s.clone(),
                            width: FloatWidth::F32,
                        },
                        s,
                    )
                })
            }
            MssqlScalarDecoder::Float8 => {
                Self::try_cell(row.try_get::<f64, _>(index), column_type, |v| {
                    let s = v.to_string();
                    (
                        DbValue::Float {
                            value: s.clone(),
                            width: FloatWidth::F64,
                        },
                        s,
                    )
                })
            }
            MssqlScalarDecoder::Guid => {
                Self::try_cell(row.try_get::<Uuid, _>(index), column_type, |v| {
                    let s = v.to_string();
                    (DbValue::Uuid(s.clone()), s)
                })
            }
            MssqlScalarDecoder::Decimal => Self::try_cell(
                row.try_get::<tiberius::numeric::Numeric, _>(index),
                column_type,
                |v| {
                    let s = v.to_string();
                    (DbValue::Decimal(s.clone()), s)
                },
            ),
            MssqlScalarDecoder::DateTime => {
                Self::try_cell(row.try_get::<NaiveDateTime, _>(index), column_type, |v| {
                    let s = v.format("%Y-%m-%d %H:%M:%S").to_string();
                    (DbValue::DateTime(s.clone()), s)
                })
            }
            MssqlScalarDecoder::Date => {
                Self::try_cell(row.try_get::<NaiveDate, _>(index), column_type, |v| {
                    let s = v.format("%Y-%m-%d").to_string();
                    (DbValue::Date(s.clone()), s)
                })
            }
            MssqlScalarDecoder::Time => {
                Self::try_cell(row.try_get::<NaiveTime, _>(index), column_type, |v| {
                    let s = v.format("%H:%M:%S%.f").to_string();
                    (DbValue::Time(s.clone()), s)
                })
            }
            MssqlScalarDecoder::DateTimeOffset => Self::try_cell(
                row.try_get::<DateTime<FixedOffset>, _>(index),
                column_type,
                |v| {
                    let s = v.format("%Y-%m-%d %H:%M:%S%.f%:z").to_string();
                    (DbValue::DateTime(s.clone()), s)
                },
            ),
        }
    }

    fn build_descriptors(columns: &[String], column_types: &[String]) -> Vec<ColumnDescriptor> {
        columns
            .iter()
            .zip(column_types.iter())
            .enumerate()
            .map(|(index, (name, db_type))| ColumnDescriptor {
                id: format!("column:{index}"),
                label: name.clone(),
                native_type: db_type.clone(),
                logical_type: format!("{:?}", FieldType::from_db_type(db_type)),
                nullable: Nullability::Unknown,
                charset: None,
                collation: None,
                precision: None,
                scale: None,
            })
            .collect()
    }

    fn collect_binary_cells(rows: &[ResultRow]) -> Vec<BinaryCell> {
        let mut binary_cells = Vec::new();
        for (row_index, row) in rows.iter().enumerate() {
            for (column_index, cell) in row.cells.iter().enumerate() {
                if let CellState::Decoded(DbValue::Binary(bytes)) = cell {
                    binary_cells.push(BinaryCell {
                        row_index,
                        column_index,
                        bytes: bytes.clone(),
                    });
                }
            }
        }
        binary_cells
    }

    fn build_query_result(
        columns: Vec<String>,
        column_types: Vec<String>,
        rows: Vec<Row>,
        sql: String,
        elapsed_ms: u128,
    ) -> SqlResult {
        debug!(
            "[MSSQL] Query returned {} rows, {} columns: {:?}",
            rows.len(),
            columns.len(),
            columns
        );

        let column_meta: Vec<QueryColumnMeta> = columns
            .iter()
            .zip(column_types.iter())
            .map(|(name, db_type)| QueryColumnMeta::new(name.clone(), db_type.clone()))
            .collect();
        let descriptors = Self::build_descriptors(&columns, &column_types);

        let (cells_rows, display_rows): (Vec<Vec<CellState>>, Vec<Vec<Option<String>>>) = rows
            .iter()
            .map(|row| {
                let mut cells = Vec::with_capacity(columns.len());
                let mut display = Vec::with_capacity(columns.len());
                for i in 0..columns.len() {
                    let column_type = row.columns()[i].column_type();
                    let (state, cell_display) = Self::extract_cell(row, i, column_type);
                    cells.push(state);
                    display.push(cell_display);
                }
                (cells, display)
            })
            .collect();

        let batch_rows = cells_rows
            .into_iter()
            .enumerate()
            .map(|(row_index, cells)| ResultRow {
                id: row_index as u64,
                cells,
            })
            .collect::<Vec<_>>();
        let binary_cells = Self::collect_binary_cells(&batch_rows);
        let typed_batch = match ResultBatch::try_new(0, descriptors, batch_rows, true) {
            Ok(batch) => Arc::new(batch),
            Err(error) => {
                return SqlResult::Error(SqlErrorInfo {
                    sql,
                    message: format!("failed to build typed result batch: {error}"),
                });
            }
        };

        SqlResult::Query(QueryResult {
            sql,
            columns,
            column_meta,
            rows: display_rows,
            binary_cells,
            elapsed_ms,
            typed_batch: Some(typed_batch),
        })
    }

    fn build_exec_result(sql: String, rows_affected: u64, elapsed_ms: u128) -> SqlResult {
        let message = format_message(&sql, rows_affected);
        SqlResult::Exec(ExecResult {
            sql,
            rows_affected,
            elapsed_ms,
            message: Some(message),
        })
    }

    async fn execute_single(
        client: &mut Client<Compat<TcpStream>>,
        sql: &str,
    ) -> Result<SqlResult, DbError> {
        let start = Instant::now();
        let sql_string = sql.to_string();
        let sql_preview = if sql.len() > 200 {
            format!("{}...", truncate_str(sql, 200))
        } else {
            sql.to_string()
        };
        debug!("[MSSQL] Executing SQL: {}", sql_preview);

        match client.query(sql, &[]).await {
            Ok(mut stream) => {
                debug!("[MSSQL] Query submitted successfully, fetching columns...");
                let (columns, column_types): (Vec<String>, Vec<String>) =
                    match stream.columns().await {
                        Ok(Some(cols)) => cols
                            .iter()
                            .map(|c| (c.name().to_string(), format!("{:?}", c.column_type())))
                            .unzip(),
                        Ok(None) => (Vec::new(), Vec::new()),
                        Err(e) => {
                            error!("[MSSQL] Failed to fetch result columns: {}", e);
                            return Ok(SqlResult::Error(SqlErrorInfo {
                                sql: sql_string,
                                message: e.to_string(),
                            }));
                        }
                    };

                if columns.is_empty() {
                    let rows_affected = match stream.into_results().await {
                        Ok(results) => results.iter().map(|r| r.len() as u64).sum(),
                        Err(e) => {
                            error!("[MSSQL] Failed to fetch execution result: {}", e);
                            return Ok(SqlResult::Error(SqlErrorInfo {
                                sql: sql_string,
                                message: e.to_string(),
                            }));
                        }
                    };
                    let elapsed_ms = start.elapsed().as_millis();
                    debug!(
                        "[MSSQL] Execute completed: {} rows affected, {}ms",
                        rows_affected, elapsed_ms
                    );
                    Ok(Self::build_exec_result(
                        sql_string,
                        rows_affected,
                        elapsed_ms,
                    ))
                } else {
                    debug!("[MSSQL] Fetching result rows, columns: {:?}", columns);
                    match stream.into_first_result().await {
                        Ok(rows) => {
                            let elapsed_ms = start.elapsed().as_millis();
                            debug!(
                                "[MSSQL] Query completed: {} rows returned, {}ms",
                                rows.len(),
                                elapsed_ms
                            );
                            Ok(Self::build_query_result(
                                columns,
                                column_types,
                                rows,
                                sql_string,
                                elapsed_ms,
                            ))
                        }
                        Err(e) => {
                            error!("[MSSQL] Failed to fetch result rows: {}", e);
                            Ok(SqlResult::Error(SqlErrorInfo {
                                sql: sql_string,
                                message: e.to_string(),
                            }))
                        }
                    }
                }
            }
            Err(e) => {
                error!("[MSSQL] Query execution failed: {}", e);
                Ok(SqlResult::Error(SqlErrorInfo {
                    sql: sql_string,
                    message: e.to_string(),
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nullable_tds_wrapped_types_have_typed_decoders() {
        // 回归：可空列会让 TDS 换成变长包装类型，例如系统目录里的
        // `is_nullable` / `is_unique` 都是可空 `bit`（BITN）。缺这些分支时
        // 单元格会落成 Undecoded，列/索引元数据读取整条失败。
        let expected = [
            (ColumnType::Bitn, MssqlScalarDecoder::Bit),
            (ColumnType::Intn, MssqlScalarDecoder::Int8),
            (ColumnType::Floatn, MssqlScalarDecoder::Float8),
            (
                ColumnType::DatetimeOffsetn,
                MssqlScalarDecoder::DateTimeOffset,
            ),
            (ColumnType::Null, MssqlScalarDecoder::Null),
        ];

        for (column_type, decoder) in expected {
            assert_eq!(
                MssqlScalarDecoder::for_column_type(column_type),
                Some(decoder),
                "{column_type:?}"
            );
        }
    }

    #[test]
    fn fixed_length_types_map_to_their_decoders() {
        let expected = [
            (ColumnType::Bit, MssqlScalarDecoder::Bit),
            (ColumnType::Int1, MssqlScalarDecoder::Int1),
            (ColumnType::Int2, MssqlScalarDecoder::Int2),
            (ColumnType::Int4, MssqlScalarDecoder::Int4),
            (ColumnType::Int8, MssqlScalarDecoder::Int8),
            (ColumnType::Money, MssqlScalarDecoder::Money),
            (ColumnType::Money4, MssqlScalarDecoder::Money),
            (ColumnType::Float4, MssqlScalarDecoder::Float4),
            (ColumnType::Float8, MssqlScalarDecoder::Float8),
            (ColumnType::Guid, MssqlScalarDecoder::Guid),
            (ColumnType::Decimaln, MssqlScalarDecoder::Decimal),
            (ColumnType::Numericn, MssqlScalarDecoder::Decimal),
            (ColumnType::Datetime, MssqlScalarDecoder::DateTime),
            (ColumnType::Datetimen, MssqlScalarDecoder::DateTime),
            (ColumnType::Datetime2, MssqlScalarDecoder::DateTime),
            (ColumnType::Daten, MssqlScalarDecoder::Date),
            (ColumnType::Timen, MssqlScalarDecoder::Time),
        ];

        for (column_type, decoder) in expected {
            assert_eq!(
                MssqlScalarDecoder::for_column_type(column_type),
                Some(decoder),
                "{column_type:?}"
            );
        }
    }

    #[test]
    fn unmodelled_types_stay_undecoded() {
        // `Udt` / `SSVariant` 在驱动层就是未实现，保持 Undecoded 而不是伪装成值。
        for column_type in [ColumnType::Udt, ColumnType::SSVariant] {
            assert_eq!(MssqlScalarDecoder::for_column_type(column_type), None);
        }
    }

    #[test]
    fn fixed_and_variable_character_types_are_text() {
        for column_type in [
            ColumnType::BigChar,
            ColumnType::BigVarChar,
            ColumnType::NChar,
            ColumnType::NVarchar,
            ColumnType::Text,
            ColumnType::NText,
            ColumnType::Xml,
        ] {
            assert!(
                MssqlDbConnection::is_character_column_type(column_type),
                "{column_type:?}"
            );
        }

        for column_type in [
            ColumnType::BigBinary,
            ColumnType::BigVarBin,
            ColumnType::Image,
        ] {
            assert!(
                !MssqlDbConnection::is_character_column_type(column_type),
                "{column_type:?}"
            );
        }
    }

    #[test]
    fn binary_column_types_are_classified_as_binary() {
        for column_type in [
            ColumnType::BigBinary,
            ColumnType::BigVarBin,
            ColumnType::Image,
        ] {
            assert!(
                MssqlDbConnection::is_binary_column_type(column_type),
                "{column_type:?}"
            );
        }
        for column_type in [
            ColumnType::BigChar,
            ColumnType::BigVarChar,
            ColumnType::NVarchar,
        ] {
            assert!(
                !MssqlDbConnection::is_binary_column_type(column_type),
                "{column_type:?}"
            );
        }
    }

    #[test]
    fn decode_failure_is_distinct_from_null_and_carries_raw_bytes() {
        let (state, display) = MssqlDbConnection::decode_failure(
            ColumnType::BigVarChar,
            Some(b"\xFF\xFE".to_vec()),
            "boom".into(),
        );
        assert_eq!(
            state,
            CellState::DecodeError {
                native_type: "BigVarChar".to_string(),
                raw: Some(RawPayload {
                    bytes: b"\xFF\xFE".to_vec(),
                    representation: RawRepresentation::DatabaseValueBytes,
                }),
                diagnostic: "boom".to_string(),
            }
        );
        assert_eq!(display, Some("0xfffe".to_string()));
    }
}

#[async_trait]
impl DbConnection for MssqlDbConnection {
    fn config(&self) -> &DbConnectionConfig {
        &self.config
    }

    fn set_config_database(&mut self, database: Option<String>) {
        self.config.database = database;
    }

    async fn connect(&mut self) -> Result<(), DbError> {
        let config = &self.config;
        info!("[MSSQL] Connecting to {}:{}", config.host, config.port);
        let target = resolve_connection_target(config).await?;
        self.tunnel = target.tunnel;

        let mut tiberius_config = Config::new();
        tiberius_config.host(&target.host);
        tiberius_config.port(target.port);
        tiberius_config.authentication(AuthMethod::sql_server(&config.username, &config.password));

        if config
            .get_param("trust_cert")
            .map(|v| v != "false")
            .unwrap_or(true)
        {
            tiberius_config.trust_cert();
        }

        let encrypt = config
            .get_param("encrypt")
            .map(|s| s.as_str())
            .unwrap_or("off");
        match encrypt {
            "on" => tiberius_config.encryption(tiberius::EncryptionLevel::On),
            "required" => tiberius_config.encryption(tiberius::EncryptionLevel::Required),
            _ => tiberius_config.encryption(tiberius::EncryptionLevel::NotSupported),
        };

        if let Some(app_name) = config.get_param("application_name") {
            tiberius_config.application_name(app_name);
        }

        if let Some(ref db) = config.database {
            tiberius_config.database(db);
            debug!("[MSSQL] Using database: {}", db);
        }

        let connect_timeout = config.get_param_as::<u64>("connect_timeout").unwrap_or(30);
        debug!("[MSSQL] Connect timeout: {}s", connect_timeout);

        debug!("[MSSQL] Establishing TCP connection...");
        let tcp = tokio::time::timeout(
            std::time::Duration::from_secs(connect_timeout),
            TcpStream::connect(tiberius_config.get_addr()),
        )
        .await
        .map_err(|_| DbError::connection("connection timeout"))?
        .map_err(|e| {
            error!("[MSSQL] TCP connection failed: {}", e);
            DbError::connection_with_source("failed to connect to TCP", e)
        })?;
        debug!("[MSSQL] TCP connection established");

        debug!("[MSSQL] Authenticating with SQL Server...");
        let client = Client::connect(tiberius_config, tcp.compat_write())
            .await
            .map_err(|e| {
                error!("[MSSQL] Authentication failed: {}", e);
                DbError::connection_with_source("failed to connect to MSSQL", e)
            })?;
        info!("[MSSQL] Connected successfully");

        {
            let mut guard = self.client.lock().await;
            *guard = Some(client);
        }

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), DbError> {
        debug!("[MSSQL] Disconnecting...");
        let mut guard = self.client.lock().await;
        *guard = None;
        self.tunnel = None;
        info!("[MSSQL] Disconnected");
        Ok(())
    }

    async fn execute(
        &self,
        plugin: &dyn DatabasePlugin,
        script: &str,
        options: ExecOptions,
    ) -> Result<Vec<SqlResult>, DbError> {
        debug!(
            "[MSSQL] execute() called, transactional={}, stop_on_error={}",
            options.transactional, options.stop_on_error
        );
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().ok_or(DbError::NotConnected)?;

        let parser = plugin
            .create_parser(SqlSource::Script(script.to_string()))
            .map_err(|e| DbError::query(format!("Failed to create parser: {}", e)))?;
        let statements: Vec<String> = parser
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|e| DbError::query_with_source("failed to parse SQL script", e))?
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        debug!("[MSSQL] Split into {} statement(s)", statements.len());
        let mut results = Vec::new();

        if options.transactional {
            let non_empty_statements: Vec<&str> = statements
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();

            if non_empty_statements.is_empty() {
                debug!("[MSSQL] No statements to execute");
                return Ok(results);
            }

            let batch_sql = format!(
                "SET XACT_ABORT ON;\nBEGIN TRANSACTION;\n{}\nCOMMIT;",
                non_empty_statements.join(";\n")
            );

            debug!(
                "[MSSQL] Executing transactional batch with {} statement(s)",
                non_empty_statements.len()
            );
            let start = Instant::now();
            match client.execute(&batch_sql, &[]).await {
                Ok(exec_result) => {
                    let elapsed_ms = start.elapsed().as_millis();
                    let rows_affected = exec_result.total();
                    debug!(
                        "[MSSQL] Transactional batch completed: {} rows affected, {}ms",
                        rows_affected, elapsed_ms
                    );
                    results.push(SqlResult::Exec(ExecResult {
                        sql: batch_sql,
                        rows_affected,
                        elapsed_ms,
                        message: Some(format!(
                            "Executed {} statement(s), {} row(s) affected",
                            non_empty_statements.len(),
                            rows_affected
                        )),
                    }));
                }
                Err(e) => {
                    error!("[MSSQL] Transactional batch failed: {}", e);
                    results.push(SqlResult::Error(SqlErrorInfo {
                        sql: batch_sql,
                        message: e.to_string(),
                    }));
                }
            }
        } else {
            for (idx, sql) in statements.iter().enumerate() {
                let sql = sql.trim();
                if sql.is_empty() {
                    continue;
                }

                debug!(
                    "[MSSQL] Executing statement {}/{}",
                    idx + 1,
                    statements.len()
                );
                let sql_to_execute = plugin.apply_query_max_rows(sql, options.max_rows);
                let result = Self::execute_single(client, sql_to_execute.as_ref()).await?;

                let is_error = result.is_error();
                if is_error {
                    debug!(
                        "[MSSQL] Statement {}/{} returned error",
                        idx + 1,
                        statements.len()
                    );
                }
                results.push(result.with_original_sql(sql));

                if is_error && options.stop_on_error {
                    debug!("[MSSQL] Stopping execution due to error (stop_on_error=true)");
                    break;
                }
            }
        }

        debug!(
            "[MSSQL] execute() completed with {} result(s)",
            results.len()
        );
        Ok(results)
    }

    async fn query(&self, query: &str) -> Result<SqlResult, DbError> {
        debug!("[MSSQL] Acquiring client lock...");
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().ok_or(DbError::NotConnected)?;
        debug!("[MSSQL] Lock acquired");

        debug!(
            "[MSSQL] Executing query: {}",
            &query[..query.len().min(100)]
        );
        Self::execute_single(client, query).await
    }

    async fn current_database(&self) -> Result<Option<String>, DbError> {
        debug!("[MSSQL] Querying current database");
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().ok_or(DbError::NotConnected)?;

        let stream = client.query("SELECT DB_NAME()", &[]).await.map_err(|e| {
            error!("[MSSQL] Failed to query current database: {}", e);
            DbError::query_with_source("failed to query current database", e)
        })?;
        let rows = stream.into_first_result().await.map_err(|e| {
            error!("[MSSQL] Failed to fetch current database result: {}", e);
            DbError::query_with_source("failed to fetch current database result", e)
        })?;
        let result = match rows.first() {
            Some(row) => row
                .try_get::<&str, _>(0)
                .map_err(|e| {
                    error!("[MSSQL] Failed to decode current database: {}", e);
                    DbError::query_with_source("failed to decode current database", e)
                })?
                .map(str::to_string),
            None => {
                debug!("[MSSQL] No rows returned for current database query");
                None
            }
        };
        debug!("[MSSQL] Current database: {:?}", result);
        Ok(result)
    }

    async fn switch_database(&self, database: &str) -> Result<(), DbError> {
        debug!("[MSSQL] Switching to database: {}", database);
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().ok_or(DbError::NotConnected)?;

        let sql = format!("USE [{}]", database.replace("]", "]]"));
        debug!("[MSSQL] Executing: {}", sql);
        client.execute(&sql, &[]).await.map_err(|e| {
            error!("[MSSQL] Failed to switch database: {}", e);
            DbError::query_with_source("failed to switch database", e)
        })?;

        info!("[MSSQL] Switched to database: {}", database);
        Ok(())
    }

    async fn execute_streaming(
        &self,
        plugin: &dyn DatabasePlugin,
        source: SqlSource,
        options: ExecOptions,
        sender: mpsc::Sender<StreamingProgress>,
    ) -> Result<(), DbError> {
        debug!(
            "[MSSQL] execute_streaming() called, transactional={}, streaming={}",
            options.transactional, options.streaming
        );
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().ok_or(DbError::NotConnected)?;

        let total_size = source.file_size().unwrap_or(0);
        let is_file_source = source.is_file();

        let mut parser = plugin
            .create_parser(source)
            .map_err(|e| DbError::query(format!("Failed to create parser: {}", e)))?;

        if options.streaming || is_file_source {
            let mut current = 0usize;

            while let Some(stmt_result) = parser.next() {
                let bytes_read = parser.bytes_read();
                let sql = match stmt_result {
                    Ok(s) if !s.trim().is_empty() => s,
                    Ok(_) => continue,
                    Err(e) => {
                        let progress = StreamingProgress::with_file_progress(
                            current,
                            SqlResult::Error(SqlErrorInfo {
                                sql: String::new(),
                                message: format!("Parse error: {}", e),
                            }),
                            bytes_read,
                            total_size,
                        );
                        let _ = sender.send(progress).await;
                        if options.stop_on_error {
                            break;
                        }
                        continue;
                    }
                };

                current += 1;
                debug!("[MSSQL] Streaming statement {}", current);

                let sql_to_execute = plugin.apply_query_max_rows(&sql, options.max_rows);
                let result = match Self::execute_single(client, sql_to_execute.as_ref()).await {
                    Ok(r) => r,
                    Err(e) => {
                        error!("[MSSQL] Streaming statement {} failed: {}", current, e);
                        SqlResult::Error(SqlErrorInfo {
                            sql: sql.clone(),
                            message: e.to_string(),
                        })
                    }
                };

                let result = result.with_original_sql(sql.as_str());
                let is_error = result.is_error();
                let progress =
                    StreamingProgress::with_file_progress(current, result, bytes_read, total_size);
                if sender.send(progress).await.is_err() {
                    break;
                }

                if is_error && options.stop_on_error {
                    break;
                }
            }
        } else {
            let statements: Vec<String> = parser
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(|e| DbError::query_with_source("failed to parse SQL script", e))?
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            let total = statements.len();
            debug!("[MSSQL] Streaming {} statement(s)", total);

            if options.transactional {
                if statements.is_empty() {
                    debug!("[MSSQL] No statements to execute");
                    return Ok(());
                }

                let batch_sql = format!(
                    "SET XACT_ABORT ON;\nBEGIN TRANSACTION;\n{}\nCOMMIT;",
                    statements.join(";\n")
                );

                debug!("[MSSQL] Executing transactional batch in streaming mode");
                let start = Instant::now();
                let result = match client.execute(&batch_sql, &[]).await {
                    Ok(exec_result) => {
                        let elapsed_ms = start.elapsed().as_millis();
                        let rows_affected = exec_result.total();
                        SqlResult::Exec(ExecResult {
                            sql: batch_sql,
                            rows_affected,
                            elapsed_ms,
                            message: Some(format!(
                                "Executed {} statement(s), {} row(s) affected",
                                total, rows_affected
                            )),
                        })
                    }
                    Err(e) => {
                        error!("[MSSQL] Transactional batch failed: {}", e);
                        SqlResult::Error(SqlErrorInfo {
                            sql: batch_sql,
                            message: e.to_string(),
                        })
                    }
                };

                let progress = StreamingProgress::new(total, total, result);
                let _ = sender.send(progress).await;
            } else {
                for (index, sql) in statements.into_iter().enumerate() {
                    let current = index + 1;
                    debug!("[MSSQL] Streaming statement {}/{}", current, total);

                    let sql_to_execute = plugin.apply_query_max_rows(&sql, options.max_rows);
                    let result = match Self::execute_single(client, sql_to_execute.as_ref()).await {
                        Ok(r) => r,
                        Err(e) => {
                            error!(
                                "[MSSQL] Streaming statement {}/{} failed: {}",
                                current, total, e
                            );
                            SqlResult::Error(SqlErrorInfo {
                                sql: sql.clone(),
                                message: e.to_string(),
                            })
                        }
                    };

                    let result = result.with_original_sql(sql.as_str());
                    let is_error = result.is_error();
                    let progress = StreamingProgress::new(current, total, result);
                    if sender.send(progress).await.is_err() {
                        break;
                    }

                    if is_error && options.stop_on_error {
                        break;
                    }
                }
            }
        }

        debug!("[MSSQL] execute_streaming() completed");
        Ok(())
    }
}
