//! 数值 cell `value` 的宽松读取。
//!
//! wire 契约里 `i64` / `u64` / `f64` cell 的 `value` 是 JSON number,序列化仍只发
//! number。但外部驱动是独立进程里的第三方 JSON 生产者:MySQL 文本协议驱动把
//! `BIGINT UNSIGNED` 列按声明类型还原后以十进制文本发送(见 navop-extensions
//! `dbipc.toCellForKind`),此时宿主若严格按 number 解析就会报
//! `invalid type: string "4", expected u64`,整张表都读不出来。
//!
//! 十进制文本与 JSON number 在宿主侧完全等价:`u64::MAX` 这样的极值也不丢精度。
//! 因此这里在**解析**时接受等值文本;无法解释的文本(例如 `"abc"`)依然报错,
//! 不会被静默降级成别的类型而掩盖驱动问题。
//!
//! 注意:工作区为 `serde_json` 开了 `arbitrary_precision`,该 feature 下浮点数
//! (以及超出 `u64` 范围的整数)不走 `visit_f64` / `visit_u64`,而是被包成私有
//! number map 交给 `visit_map`。所以三个读取器都必须处理 `visit_map`,否则
//! `1.5` 这样的普通浮点会被误判成"传了个对象"。

use std::fmt;

use serde::de::{Deserializer, Error as DeError, Expected, MapAccess, Unexpected, Visitor};

/// serde_json 在 `arbitrary_precision` 下包裹"无法用 u64/i64/f64 表示的数字"用的
/// 私有 key(见 serde_json `number::TOKEN`,非 pub,只能按字面量比对)。
const ARBITRARY_PRECISION_TOKEN: &str = "$serde_json::private::Number";

/// 数值 cell 的期望描述,用于错误信息。
struct CellNumberExpected(&'static str);

impl Expected for CellNumberExpected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

const U64_EXPECTED: CellNumberExpected =
    CellNumberExpected("an u64 number or a decimal integer string");
const I64_EXPECTED: CellNumberExpected =
    CellNumberExpected("an i64 number or a decimal integer string");
const F64_EXPECTED: CellNumberExpected = CellNumberExpected("an f64 number or a numeric string");

/// 从 `arbitrary_precision` 的私有 number map 里取回原始数字文本。
///
/// 非该形态的 map 一律视为类型不符,由调用方报错,不做任何猜测性解析。
fn precision_number_text<'de, A: MapAccess<'de>>(access: &mut A) -> Result<String, A::Error> {
    let not_a_number = || A::Error::custom("a JSON object is not a cell number");
    match access.next_key::<String>()?.as_deref() {
        Some(ARBITRARY_PRECISION_TOKEN) => {}
        _ => return Err(not_a_number()),
    }
    let text = access.next_value::<String>()?;
    if access.next_key::<String>()?.is_some() {
        return Err(not_a_number());
    }
    Ok(text)
}

/// `CellValue::I64` 的 `value`:number 或十进制整数文本。
pub(crate) fn i64_value<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(I64Visitor)
}

/// `CellValue::U64` 的 `value`:number 或十进制整数文本。
pub(crate) fn u64_value<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(U64Visitor)
}

/// `CellValue::F64` 的 `value`:number 或浮点/十进制文本。
pub(crate) fn f64_value<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(F64Visitor)
}

struct I64Visitor;

impl<'de> Visitor<'de> for I64Visitor {
    type Value = i64;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(I64_EXPECTED.0)
    }

    fn visit_i64<E: DeError>(self, value: i64) -> Result<i64, E> {
        Ok(value)
    }

    fn visit_u64<E: DeError>(self, value: u64) -> Result<i64, E> {
        i64::try_from(value)
            .map_err(|_| E::invalid_value(Unexpected::Unsigned(value), &I64_EXPECTED))
    }

    fn visit_str<E: DeError>(self, value: &str) -> Result<i64, E> {
        value
            .trim()
            .parse::<i64>()
            .map_err(|_| E::invalid_value(Unexpected::Str(value), &I64_EXPECTED))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<i64, A::Error> {
        parse_from_precision_map(&mut access, &I64_EXPECTED, |text| {
            text.trim().parse::<i64>().ok()
        })
    }
}

struct U64Visitor;

impl<'de> Visitor<'de> for U64Visitor {
    type Value = u64;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(U64_EXPECTED.0)
    }

    fn visit_u64<E: DeError>(self, value: u64) -> Result<u64, E> {
        Ok(value)
    }

    fn visit_i64<E: DeError>(self, value: i64) -> Result<u64, E> {
        u64::try_from(value).map_err(|_| E::invalid_value(Unexpected::Signed(value), &U64_EXPECTED))
    }

    fn visit_str<E: DeError>(self, value: &str) -> Result<u64, E> {
        value
            .trim()
            .parse::<u64>()
            .map_err(|_| E::invalid_value(Unexpected::Str(value), &U64_EXPECTED))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<u64, A::Error> {
        parse_from_precision_map(&mut access, &U64_EXPECTED, |text| {
            text.trim().parse::<u64>().ok()
        })
    }
}

struct F64Visitor;

impl<'de> Visitor<'de> for F64Visitor {
    type Value = f64;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(F64_EXPECTED.0)
    }

    fn visit_f64<E: DeError>(self, value: f64) -> Result<f64, E> {
        Ok(value)
    }

    fn visit_i64<E: DeError>(self, value: i64) -> Result<f64, E> {
        Ok(value as f64)
    }

    fn visit_u64<E: DeError>(self, value: u64) -> Result<f64, E> {
        Ok(value as f64)
    }

    fn visit_str<E: DeError>(self, value: &str) -> Result<f64, E> {
        value
            .trim()
            .parse::<f64>()
            .map_err(|_| E::invalid_value(Unexpected::Str(value), &F64_EXPECTED))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<f64, A::Error> {
        parse_from_precision_map(&mut access, &F64_EXPECTED, |text| {
            text.trim().parse::<f64>().ok()
        })
    }
}

/// 把 `arbitrary_precision` 私有 number map 里的数字文本按目标类型解析。
fn parse_from_precision_map<'de, A, T>(
    access: &mut A,
    expected: &CellNumberExpected,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<T, A::Error>
where
    A: MapAccess<'de>,
{
    let text = precision_number_text(access)
        .map_err(|_| A::Error::invalid_type(Unexpected::Map, expected))?;
    parse(&text).ok_or_else(|| A::Error::invalid_value(Unexpected::Str(&text), expected))
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct U64Cell {
        #[serde(deserialize_with = "super::u64_value")]
        value: u64,
    }

    #[derive(Debug, Deserialize)]
    struct I64Cell {
        #[serde(deserialize_with = "super::i64_value")]
        value: i64,
    }

    #[derive(Debug, Deserialize)]
    struct F64Cell {
        #[serde(deserialize_with = "super::f64_value")]
        value: f64,
    }

    fn parse<T: for<'de> Deserialize<'de>>(raw: &str) -> Result<T, serde_json::Error> {
        serde_json::from_str(raw)
    }

    #[test]
    fn unsigned_accepts_number_and_decimal_text() {
        assert_eq!(4, parse::<U64Cell>(r#"{"value":4}"#).unwrap().value);
        assert_eq!(4, parse::<U64Cell>(r#"{"value":"4"}"#).unwrap().value);
        assert_eq!(
            u64::MAX,
            parse::<U64Cell>(r#"{"value":"18446744073709551615"}"#)
                .unwrap()
                .value
        );
        assert_eq!(
            u64::MAX,
            parse::<U64Cell>(r#"{"value":18446744073709551615}"#)
                .unwrap()
                .value
        );
    }

    #[test]
    fn unsigned_rejects_sign_and_non_numeric_text() {
        for raw in [
            r#"{"value":"-1"}"#,
            r#"{"value":"abc"}"#,
            r#"{"value":-1}"#,
            r#"{"value":"18446744073709551616"}"#,
            r#"{"value":{"a":"4"}}"#,
        ] {
            assert!(
                parse::<U64Cell>(raw).is_err(),
                "{raw} must not be accepted as an u64 cell value"
            );
        }
    }

    #[test]
    fn signed_accepts_number_and_decimal_text() {
        assert_eq!(-42, parse::<I64Cell>(r#"{"value":-42}"#).unwrap().value);
        assert_eq!(-42, parse::<I64Cell>(r#"{"value":"-42"}"#).unwrap().value);
        assert_eq!(42, parse::<I64Cell>(r#"{"value":42}"#).unwrap().value);
        assert!(parse::<I64Cell>(r#"{"value":18446744073709551615}"#).is_err());
        assert!(parse::<I64Cell>(r#"{"value":"4.5"}"#).is_err());
    }

    #[test]
    fn float_accepts_number_and_numeric_text() {
        // 浮点走 serde_json 的私有 number map(arbitrary_precision),不是 visit_f64。
        assert_eq!(
            1234567890.1234567,
            parse::<F64Cell>(r#"{"value":1234567890.1234567}"#)
                .unwrap()
                .value
        );
        assert_eq!(1.5, parse::<F64Cell>(r#"{"value":"1.5"}"#).unwrap().value);
        assert_eq!(2.0, parse::<F64Cell>(r#"{"value":"2"}"#).unwrap().value);
        assert_eq!(2.0, parse::<F64Cell>(r#"{"value":2}"#).unwrap().value);
        assert!(parse::<F64Cell>(r#"{"value":"not-a-number"}"#).is_err());
        assert!(parse::<F64Cell>(r#"{"value":{"a":1.5}}"#).is_err());
    }
}
