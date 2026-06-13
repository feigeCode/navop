//! Row IR——统一的行/值表示。
//!
//! 每个 cell 都是 `{ "type": ..., "value": ... }` 的 tagged union。把所有数据库
//! 类型抹平到一个跨语言可读的中间表示,host 不需要为每个数据库专门解析。
//!
//! 设计要点(详见 [`docs/design/extensions/api-database.md`] §14):
//!
//! - **decimal 用字符串**:跨语言无损精度
//! - **bytes 用 base64**:JSON 友好;MessagePack 也支持 bin,扩展可选
//! - **datetime 用 ISO 8601 字符串**:跨时区清晰
//! - **custom**:通过 `data_types.renderer` 贡献点扩展,raw 是原始字节 base64

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 列类型 kind,大致对应 SQL 标准类型族。
///
/// `extra` raw 类型字符串(由扩展提供)用 [`ColumnSpec::raw_type`] 保留,这里
/// 只做粗粒度分组,便于 host 选择默认渲染器。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnTypeKind {
    Null,
    Bool,
    I64,
    U64,
    F64,
    Decimal,
    Text,
    Bytes,
    Json,
    Uuid,
    Date,
    Time,
    Datetime,
    Duration,
    Array,
    Map,
    Geo,
    Custom,
    /// 未知/无法识别——尽量避免使用,但允许扩展返回。
    Unknown,
}

impl ColumnTypeKind {
    /// 是否数值类型(可用于聚合 / 排序的 numeric)。
    pub fn is_numeric(self) -> bool {
        matches!(self, Self::I64 | Self::U64 | Self::F64 | Self::Decimal)
    }

    /// 是否时间类型。
    pub fn is_temporal(self) -> bool {
        matches!(
            self,
            Self::Date | Self::Time | Self::Datetime | Self::Duration
        )
    }

    /// 是否容器类型(可能嵌套)。
    pub fn is_container(self) -> bool {
        matches!(self, Self::Array | Self::Map | Self::Json)
    }
}

/// 列说明,query/start 的响应里返回,描述结果集 schema。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnSpec {
    /// 列名(显示用,可能是别名)。
    pub name: String,
    /// 数据库声明的原始类型字符串,e.g. `VARCHAR(255)` / `numeric(10,2)` / `geometry(POINT,4326)`。
    #[serde(rename = "type")]
    pub type_str: String,
    /// 粗粒度 type kind。
    #[serde(rename = "type_kind")]
    pub type_kind: ColumnTypeKind,
    /// 是否允许 null。`None` 表示驱动未提供这个信息。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nullable: Option<bool>,
    /// 最大长度(varchar / text 等)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    /// 数值类型的 precision(decimal 总位数 / numeric)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<u32>,
    /// 数值类型的 scale(小数位数)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<i32>,
    /// 任意 driver 私有字段(charset、collation、enum values、custom typename 等)。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

impl ColumnSpec {
    pub fn new(
        name: impl Into<String>,
        type_str: impl Into<String>,
        type_kind: ColumnTypeKind,
    ) -> Self {
        Self {
            name: name.into(),
            type_str: type_str.into(),
            type_kind,
            nullable: None,
            max_length: None,
            precision: None,
            scale: None,
            extra: Value::Null,
        }
    }

    pub fn nullable(mut self, nullable: bool) -> Self {
        self.nullable = Some(nullable);
        self
    }

    pub fn max_length(mut self, n: u32) -> Self {
        self.max_length = Some(n);
        self
    }

    pub fn precision(mut self, n: u32) -> Self {
        self.precision = Some(n);
        self
    }

    pub fn scale(mut self, n: i32) -> Self {
        self.scale = Some(n);
        self
    }

    pub fn with_extra(mut self, extra: Value) -> Self {
        self.extra = extra;
        self
    }
}

/// 单元格值。
///
/// 序列化为 `{ "type": "...", "value": ... }` 的 tagged union。
/// `Null` 没有 `value` 字段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CellValue {
    Null,
    Bool {
        value: bool,
    },
    I64 {
        value: i64,
    },
    U64 {
        value: u64,
    },
    F64 {
        value: f64,
    },
    /// `decimal` 以字符串保留精度。
    Decimal {
        value: String,
    },
    Text {
        value: String,
    },
    /// 原始字节,序列化为 base64 字符串。
    Bytes {
        value: String,
    },
    /// 任意 JSON 值(jsonb / document 等)。
    Json {
        value: Value,
    },
    Uuid {
        value: String,
    },
    /// ISO 8601 date: `YYYY-MM-DD`。
    Date {
        value: String,
    },
    /// ISO 8601 time: `HH:MM:SS[.fff]`。
    Time {
        value: String,
    },
    /// ISO 8601 datetime: `YYYY-MM-DDTHH:MM:SS[.fff]Z`(带时区)。
    Datetime {
        value: String,
    },
    /// ISO 8601 duration: `P1DT2H3M4S`。
    Duration {
        value: String,
    },
    /// 同构数组,element_type 提示元素 kind。
    Array {
        element_type: ColumnTypeKind,
        value: Vec<CellValue>,
    },
    /// 字符串 key 的 map(对应 hstore / map<text,text> 等)。
    Map {
        value: serde_json::Map<String, Value>,
    },
    /// 地理类型,subtype 是 `point` / `linestring` / `polygon` 等,
    /// value 是 WKT 字符串(也可换成 GeoJSON,由扩展决定)。
    Geo {
        subtype: String,
        value: String,
    },
    /// 自定义类型——驱动私有,raw 是 base64 后的原始字节。
    Custom {
        subtype: String,
        raw: String,
    },
}

impl CellValue {
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// 该值的 [`ColumnTypeKind`]。
    pub fn type_kind(&self) -> ColumnTypeKind {
        match self {
            Self::Null => ColumnTypeKind::Null,
            Self::Bool { .. } => ColumnTypeKind::Bool,
            Self::I64 { .. } => ColumnTypeKind::I64,
            Self::U64 { .. } => ColumnTypeKind::U64,
            Self::F64 { .. } => ColumnTypeKind::F64,
            Self::Decimal { .. } => ColumnTypeKind::Decimal,
            Self::Text { .. } => ColumnTypeKind::Text,
            Self::Bytes { .. } => ColumnTypeKind::Bytes,
            Self::Json { .. } => ColumnTypeKind::Json,
            Self::Uuid { .. } => ColumnTypeKind::Uuid,
            Self::Date { .. } => ColumnTypeKind::Date,
            Self::Time { .. } => ColumnTypeKind::Time,
            Self::Datetime { .. } => ColumnTypeKind::Datetime,
            Self::Duration { .. } => ColumnTypeKind::Duration,
            Self::Array { .. } => ColumnTypeKind::Array,
            Self::Map { .. } => ColumnTypeKind::Map,
            Self::Geo { .. } => ColumnTypeKind::Geo,
            Self::Custom { .. } => ColumnTypeKind::Custom,
        }
    }
}

/// 一行——cell 的有序列表。
///
/// 列名 / 类型由 [`ColumnSpec`] 提供,这里只是按 ordinal 排列的值,
/// MessagePack 序列化时表现为 array,JSON 也是 array,带宽友好。
pub type Row = Vec<CellValue>;

/// 参数化查询的输入参数。
///
/// 与 [`CellValue`] 结构一致,但语义上是「输入」而非「输出」。
pub type ParamValue = CellValue;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_cell_serializes_without_value() {
        let c = CellValue::Null;
        let s = serde_json::to_string(&c).unwrap();
        assert_eq!(s, r#"{"type":"null"}"#);
    }

    #[test]
    fn bool_cell_round_trip() {
        let c = CellValue::Bool { value: true };
        let s = serde_json::to_string(&c).unwrap();
        assert_eq!(s, r#"{"type":"bool","value":true}"#);
        let parsed: CellValue = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn i64_and_u64_cells() {
        let i = CellValue::I64 { value: -42 };
        let u = CellValue::U64 {
            value: 18_000_000_000_000_000_000,
        };
        let si = serde_json::to_string(&i).unwrap();
        let su = serde_json::to_string(&u).unwrap();
        assert_eq!(si, r#"{"type":"i64","value":-42}"#);
        assert_eq!(su, r#"{"type":"u64","value":18000000000000000000}"#);
    }

    #[test]
    fn decimal_uses_string_to_preserve_precision() {
        let d = CellValue::Decimal {
            value: "12345.67890123456789".to_string(),
        };
        let s = serde_json::to_string(&d).unwrap();
        assert!(s.contains(r#""value":"12345.67890123456789""#));
    }

    #[test]
    fn text_cell_round_trip() {
        let t = CellValue::Text {
            value: "hello 世界".to_string(),
        };
        let s = serde_json::to_string(&t).unwrap();
        let parsed: CellValue = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, t);
    }

    #[test]
    fn bytes_cell_uses_base64_string() {
        let b = CellValue::Bytes {
            value: "AQID".to_string(), // base64 of [1,2,3]
        };
        let s = serde_json::to_string(&b).unwrap();
        assert!(s.contains(r#""value":"AQID""#));
    }

    #[test]
    fn json_cell_preserves_nested() {
        let j = CellValue::Json {
            value: serde_json::json!({"a": [1, 2], "b": null}),
        };
        let s = serde_json::to_string(&j).unwrap();
        let parsed: CellValue = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, j);
    }

    #[test]
    fn datetime_kinds_round_trip() {
        let d = CellValue::Date {
            value: "2026-05-27".to_string(),
        };
        let t = CellValue::Time {
            value: "12:34:56.789".to_string(),
        };
        let dt = CellValue::Datetime {
            value: "2026-05-27T12:34:56.789Z".to_string(),
        };
        let dur = CellValue::Duration {
            value: "P1DT2H".to_string(),
        };
        for c in [d, t, dt, dur] {
            let s = serde_json::to_string(&c).unwrap();
            let parsed: CellValue = serde_json::from_str(&s).unwrap();
            assert_eq!(parsed, c);
        }
    }

    #[test]
    fn uuid_cell_round_trip() {
        let u = CellValue::Uuid {
            value: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        };
        let s = serde_json::to_string(&u).unwrap();
        let parsed: CellValue = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, u);
    }

    #[test]
    fn array_cell_tracks_element_type() {
        let a = CellValue::Array {
            element_type: ColumnTypeKind::Text,
            value: vec![
                CellValue::Text {
                    value: "a".to_string(),
                },
                CellValue::Text {
                    value: "b".to_string(),
                },
            ],
        };
        let s = serde_json::to_string(&a).unwrap();
        assert!(s.contains(r#""element_type":"text""#));
        let parsed: CellValue = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, a);
    }

    #[test]
    fn map_cell_round_trip() {
        let mut m = serde_json::Map::new();
        m.insert("k".to_string(), serde_json::json!("v"));
        let c = CellValue::Map { value: m };
        let s = serde_json::to_string(&c).unwrap();
        let parsed: CellValue = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn geo_cell_round_trip() {
        let g = CellValue::Geo {
            subtype: "point".to_string(),
            value: "POINT(1 2)".to_string(),
        };
        let s = serde_json::to_string(&g).unwrap();
        let parsed: CellValue = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, g);
    }

    #[test]
    fn custom_cell_round_trip() {
        let c = CellValue::Custom {
            subtype: "cassandra.varint".to_string(),
            raw: "AQID".to_string(),
        };
        let s = serde_json::to_string(&c).unwrap();
        let parsed: CellValue = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn type_kind_classifiers() {
        assert!(ColumnTypeKind::I64.is_numeric());
        assert!(ColumnTypeKind::Decimal.is_numeric());
        assert!(!ColumnTypeKind::Text.is_numeric());

        assert!(ColumnTypeKind::Datetime.is_temporal());
        assert!(!ColumnTypeKind::Json.is_temporal());

        assert!(ColumnTypeKind::Array.is_container());
        assert!(ColumnTypeKind::Json.is_container());
        assert!(!ColumnTypeKind::Text.is_container());
    }

    #[test]
    fn cell_value_type_kind_matches_variant() {
        assert_eq!(CellValue::Null.type_kind(), ColumnTypeKind::Null);
        assert_eq!(
            CellValue::Bool { value: true }.type_kind(),
            ColumnTypeKind::Bool
        );
        assert_eq!(
            CellValue::Decimal { value: "1".into() }.type_kind(),
            ColumnTypeKind::Decimal
        );
        assert_eq!(
            CellValue::Array {
                element_type: ColumnTypeKind::I64,
                value: vec![],
            }
            .type_kind(),
            ColumnTypeKind::Array
        );
    }

    #[test]
    fn cell_value_is_null_helper() {
        assert!(CellValue::Null.is_null());
        assert!(!CellValue::Bool { value: false }.is_null());
    }

    #[test]
    fn column_spec_builder_chains() {
        let c = ColumnSpec::new("id", "BIGINT UNSIGNED", ColumnTypeKind::U64)
            .nullable(false)
            .max_length(20)
            .precision(20)
            .scale(0)
            .with_extra(serde_json::json!({"auto_increment": true}));
        assert_eq!(c.name, "id");
        assert_eq!(c.type_str, "BIGINT UNSIGNED");
        assert_eq!(c.type_kind, ColumnTypeKind::U64);
        assert_eq!(c.nullable, Some(false));
        assert_eq!(c.max_length, Some(20));
        assert_eq!(c.precision, Some(20));
        assert_eq!(c.scale, Some(0));
        assert_eq!(c.extra, serde_json::json!({"auto_increment": true}));
    }

    #[test]
    fn column_spec_serialize_skips_none() {
        let c = ColumnSpec::new("x", "int", ColumnTypeKind::I64);
        let s = serde_json::to_string(&c).unwrap();
        assert!(!s.contains("nullable"));
        assert!(!s.contains("max_length"));
        assert!(!s.contains("extra"));
        assert!(s.contains(r#""name":"x""#));
        assert!(s.contains(r#""type":"int""#));
        assert!(s.contains(r#""type_kind":"i64""#));
    }

    #[test]
    fn column_spec_round_trip() {
        let c = ColumnSpec::new("name", "varchar(255)", ColumnTypeKind::Text)
            .nullable(true)
            .max_length(255);
        let s = serde_json::to_string(&c).unwrap();
        let parsed: ColumnSpec = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.name, "name");
        assert_eq!(parsed.type_str, "varchar(255)");
        assert_eq!(parsed.type_kind, ColumnTypeKind::Text);
        assert_eq!(parsed.nullable, Some(true));
        assert_eq!(parsed.max_length, Some(255));
    }

    #[test]
    fn row_serializes_as_array() {
        let row: Row = vec![
            CellValue::I64 { value: 1 },
            CellValue::Text {
                value: "abc".to_string(),
            },
            CellValue::Null,
        ];
        let s = serde_json::to_string(&row).unwrap();
        assert!(s.starts_with('['));
        assert!(s.ends_with(']'));
        let parsed: Row = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[2], CellValue::Null);
    }

    #[test]
    fn column_type_kind_serialize_snake_case() {
        assert_eq!(
            serde_json::to_string(&ColumnTypeKind::Datetime).unwrap(),
            r#""datetime""#
        );
        assert_eq!(
            serde_json::to_string(&ColumnTypeKind::Unknown).unwrap(),
            r#""unknown""#
        );
    }
}
