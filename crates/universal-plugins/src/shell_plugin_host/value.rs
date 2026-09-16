use anyhow::Result;
use gpui_shell::{HostError, HostObject, HostValue};

use super::error::{ErrorCode, navop_error};

pub(super) fn host_to_json(value: &HostValue) -> Result<serde_json::Value, HostError> {
    if let Some(tagged) = tagged_number_to_json(value)? {
        return Ok(tagged);
    }
    Ok(match value {
        HostValue::Null => serde_json::Value::Null,
        HostValue::Bool(value) => serde_json::Value::Bool(*value),
        HostValue::Number(value) => {
            // JS 数字统一是 f64,但 JSON 语义下整数值必须序列化为整数
            // (等价于 JSON.stringify(49.0) === "49"),否则字节体 `[49,49]`
            // 会变成 `[49.0,49.0]`,对端 `Vec<u8>` 反序列化报 "invalid number"。
            if value.is_finite() && value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER as f64 {
                serde_json::Value::Number(serde_json::Number::from(*value as i64))
            } else {
                serde_json::Number::from_f64(*value)
                    .map(serde_json::Value::Number)
                    .ok_or_else(|| {
                        navop_error(ErrorCode::InvalidArgument, "number is not finite")
                    })?
            }
        }
        HostValue::Str(value) => serde_json::Value::String(value.clone()),
        HostValue::Array(values) => {
            serde_json::Value::Array(values.iter().map(host_to_json).collect::<Result<_, _>>()?)
        }
        HostValue::Object(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(key, value)| Ok((key.clone(), host_to_json(value)?)))
                .collect::<Result<_, HostError>>()?,
        ),
    })
}

pub(super) fn json_to_host(value: &serde_json::Value) -> Result<HostValue, HostError> {
    Ok(match value {
        serde_json::Value::Null => HostValue::Null,
        serde_json::Value::Bool(value) => HostValue::Bool(*value),
        serde_json::Value::Number(value) => number_to_host(value)?,
        serde_json::Value::String(value) => HostValue::Str(value.clone()),
        serde_json::Value::Array(values) => {
            HostValue::Array(values.iter().map(json_to_host).collect::<Result<_, _>>()?)
        }
        serde_json::Value::Object(fields) => HostValue::Object(
            fields
                .iter()
                .map(|(key, value)| Ok((key.clone(), json_to_host(value)?)))
                .collect::<Result<_, HostError>>()?,
        ),
    })
}

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

fn number_to_host(value: &serde_json::Number) -> Result<HostValue, HostError> {
    if let Some(value) = value.as_i64()
        && value < -MAX_SAFE_INTEGER
    {
        return Ok(tagged_number("i64", value.to_string()));
    }
    if let Some(value) = value.as_u64()
        && value > MAX_SAFE_INTEGER as u64
    {
        return Ok(tagged_number("u64", value.to_string()));
    }
    value
        .as_f64()
        .map(HostValue::Number)
        .ok_or_else(|| navop_error(ErrorCode::ProtocolError, "unsupported JSON number"))
}

fn tagged_number(kind: &str, value: String) -> HostValue {
    HostObject::new()
        .field("$navop", kind)
        .field("value", value)
        .into()
}

fn tagged_number_to_json(value: &HostValue) -> Result<Option<serde_json::Value>, HostError> {
    let Some(kind) = value.get("$navop").and_then(HostValue::as_str) else {
        return Ok(None);
    };
    let text = value
        .get("value")
        .and_then(HostValue::as_str)
        .ok_or_else(|| {
            navop_error(
                ErrorCode::InvalidArgument,
                "tagged number requires a string value",
            )
        })?;
    let number =
        match kind {
            "i64" => serde_json::Number::from(text.parse::<i64>().map_err(|_| {
                navop_error(ErrorCode::InvalidArgument, "invalid tagged i64 value")
            })?),
            "u64" => serde_json::Number::from(text.parse::<u64>().map_err(|_| {
                navop_error(ErrorCode::InvalidArgument, "invalid tagged u64 value")
            })?),
            _ => return Ok(None),
        };
    Ok(Some(serde_json::Value::Number(number)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_value_json_round_trip_preserves_nested_data() {
        let value = HostObject::new()
            .field("name", "orders")
            .field("rows", vec![1, 2, 3])
            .field("ok", true)
            .into();

        let json = host_to_json(&value).unwrap();

        assert_eq!(value, json_to_host(&json).unwrap());
    }

    #[test]
    fn unsafe_integer_round_trips_as_tagged_value() {
        let json = serde_json::json!(9_007_199_254_740_992_u64);

        let host = json_to_host(&json).unwrap();

        assert_eq!(Some("u64"), host.get("$navop").and_then(HostValue::as_str));
        assert_eq!(json, host_to_json(&host).unwrap());
    }

    #[test]
    fn integral_host_numbers_serialize_as_json_integers() {
        // 字节体场景:UI 传来的 [49,49] 必须落成 JSON 整数,而不是 49.0。
        let bytes = HostValue::Array(vec![
            HostValue::Number(49.0),
            HostValue::Number(49.0),
            HostValue::Number(-1.0),
        ]);
        let json = host_to_json(&bytes).unwrap();
        assert_eq!(serde_json::json!([49, 49, -1]), json);
        assert!(json.to_string().contains("[49,49,-1]"));

        // 整数值超出 JS 安全整数范围时保持浮点表示。
        let huge = host_to_json(&HostValue::Number(9_007_199_254_740_992.0)).unwrap();
        assert_eq!(huge.to_string(), "9007199254740992.0");

        // 小数保持浮点表示。
        let frac = host_to_json(&HostValue::Number(1.5)).unwrap();
        assert_eq!(frac.to_string(), "1.5");

        // 非有限数仍然报错。
        assert!(host_to_json(&HostValue::Number(f64::NAN)).is_err());
    }
}
