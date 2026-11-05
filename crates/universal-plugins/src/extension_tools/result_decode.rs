//! provider 结果的解码与截断。

use agent_runtime::ToolError;
use extension_host::{CancellationToken, RequestOptions, UniversalPluginClient};
use extension_protocol::blob::{BlobReadParams, MAX_BLOB_CHUNK_BYTES};
use serde_json::Value;

use super::error::{ExtensionToolErrorCode, execution_error};

/// blob 解码的兜底上限（结果最终仍受工具声明的 maxResultBytes 截断）。
pub(crate) const RESULT_GUARD_BYTES: usize = 8 * 1024 * 1024;

/// 截断后的值 + 被截掉的字节数。
pub(crate) struct TruncatedValue {
    pub value: Value,
    pub truncated: usize,
}

/// 把结果序列化后按 maxResultBytes 截断。
///
/// 超限时把 JSON 文本按字节截断成字符串，并记录被截掉的字节数
/// （由调用方在 summary 里附 `…[truncated N bytes]`）。
pub(crate) fn truncate_result(value: &Value, max_bytes: usize) -> TruncatedValue {
    let encoded = serde_json::to_vec(value).unwrap_or_default();
    if encoded.len() <= max_bytes {
        return TruncatedValue {
            value: value.clone(),
            truncated: 0,
        };
    }
    let cut = max_bytes.min(encoded.len());
    // 从切点向前回退到 UTF-8 字符边界，避免劈开多字节字符。
    let mut boundary = cut;
    while boundary > 0 && (encoded[boundary] & 0xC0) == 0x80 {
        boundary -= 1;
    }
    let text = String::from_utf8_lossy(&encoded[..boundary]).into_owned();
    TruncatedValue {
        value: Value::String(text),
        truncated: encoded.len() - boundary,
    }
}

/// 结果摘要：给对话流看的单行文本。
pub(crate) fn summarize(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_default();
    let mut summary: String = text.chars().take(200).collect();
    if text.chars().count() > 200 {
        summary.push('…');
    }
    summary
}

/// 解码 `ResultRef::Blob`：读完并尝试按 base64→JSON 解析。
pub(crate) async fn decode_blob(
    client: &UniversalPluginClient,
    id: &str,
    cancel: &CancellationToken,
) -> Result<Value, ToolError> {
    let mut raw = String::new();
    let mut bytes = 0usize;
    let mut complete = false;
    while bytes < RESULT_GUARD_BYTES {
        let chunk = client
            .read_blob_with_options(
                &BlobReadParams {
                    blob_id: id.to_owned(),
                    max_bytes: Some(MAX_BLOB_CHUNK_BYTES),
                },
                RequestOptions::default().with_cancel(cancel.clone()),
            )
            .await
            .map_err(|error| {
                execution_error(
                    ExtensionToolErrorCode::ProviderError,
                    format!("读取结果 blob 失败：{error}"),
                )
            })?;
        raw.push_str(&chunk.data);
        bytes += chunk.bytes_read as usize;
        if chunk.done {
            complete = true;
            break;
        }
    }
    if complete {
        let decoded = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            raw.as_bytes(),
        );
        if let Ok(bytes) = decoded
            && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
        {
            return Ok(value);
        }
        // 不是 JSON：原样作为字符串返回。
        return Ok(Value::String(raw));
    }
    Err(execution_error(
        ExtensionToolErrorCode::ResultTooLarge,
        format!("结果 blob 超过 {RESULT_GUARD_BYTES} 字节的解码上限"),
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn small_results_pass_through() {
        let value = json!({"ok": true});
        let result = truncate_result(&value, 1024);
        assert_eq!(result.value, value);
        assert_eq!(result.truncated, 0);
    }

    #[test]
    fn oversized_results_truncate_on_char_boundary() {
        let value = json!("数".repeat(2000));
        let max = 100;
        let result = truncate_result(&value, max);
        let Value::String(text) = result.value else {
            panic!("截断后应是字符串");
        };
        assert!(text.ends_with('数') || text.is_empty());
        assert!(result.truncated > 0);
    }

    #[test]
    fn summarize_caps_at_200_chars() {
        let value = json!({"blob": "x".repeat(400)});
        let summary = summarize(&value);
        assert!(summary.chars().count() <= 201);
        assert!(summary.ends_with('…'));
    }
}
