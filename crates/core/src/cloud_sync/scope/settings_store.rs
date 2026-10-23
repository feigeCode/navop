//! 设置类 Singleton 的读写后端
//!
//! 同步跑在后台任务里，拿不到 `&App`，因此设置类数据直接以 `settings.json` 为
//! 事实来源读写（与 `AppSettings::load/save` 同一份文件）。这样无需把 `App`
//! 穿进同步链路，也避免与用户在同步进行中改设置产生交叉写。
//!
//! ⚠️ 同步写入后，进程内的 `AppSettings` 全局仍持有旧值，需要由 UI 侧在同步
//! 完成后重新 `AppSettings::load()` 并 `cx.set_global()` 刷新。

use anyhow::Result;
use serde_json::Value;

use crate::settings::AppSettings;

use super::settings_payload::{SettingsGroup, apply_payload, extract_payload};

/// 读取某分组当前载荷
pub fn read_group(group: SettingsGroup) -> Result<Value> {
    let settings = AppSettings::load();
    Ok(extract_payload(group, &settings)?)
}

/// 把载荷合并回 `settings.json`
pub fn write_group(group: SettingsGroup, payload: &Value) -> Result<()> {
    let mut settings = AppSettings::load();
    apply_payload(group, &mut settings, payload)?;
    settings.save();
    Ok(())
}

/// 基于 `settings.json` 的设置读写实现
#[derive(Clone, Copy, Default)]
pub struct FileSettingsStore;

impl FileSettingsStore {
    pub fn read(&self, group: SettingsGroup) -> Result<Value> {
        read_group(group)
    }

    pub fn write(&self, group: SettingsGroup, payload: &Value) -> Result<()> {
        write_group(group, payload)
    }
}

/// 计算载荷的稳定校验和（明文序列化后取 SHA-256，与 `CloudSyncService` 同口径）
pub fn payload_checksum(payload: &Value) -> String {
    let canonical = canonical_json(payload);
    crate::cloud_sync::service::CloudSyncService::calculate_blob_checksum(&canonical)
}

/// 稳定序列化：对象键排序，避免字段顺序变化造成假变更
pub fn canonical_json(payload: &Value) -> String {
    fn write(value: &Value, out: &mut String) {
        match value {
            Value::Null => out.push_str("null"),
            Value::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
            Value::Number(v) => out.push_str(&v.to_string()),
            Value::String(v) => {
                out.push('"');
                for ch in v.chars() {
                    match ch {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\r' => out.push_str("\\r"),
                        '\t' => out.push_str("\\t"),
                        other => out.push(other),
                    }
                }
                out.push('"');
            }
            Value::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                out.push('{');
                for (index, key) in keys.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write(&Value::String((*key).clone()), out);
                    out.push(':');
                    write(&map[*key], out);
                }
                out.push('}');
            }
        }
    }

    let mut out = String::new();
    write(payload, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::canonical_json;
    use serde_json::json;

    #[test]
    fn canonical_json_is_key_order_independent() {
        let a = json!({"b": 1, "a": 2});
        let b = json!({"a": 2, "b": 1});

        assert_eq!(canonical_json(&a), canonical_json(&b));
    }

    #[test]
    fn canonical_json_distinguishes_nested_values() {
        let a = json!({"a": {"x": 1}});
        let b = json!({"a": {"x": 2}});

        assert_ne!(canonical_json(&a), canonical_json(&b));
    }
}
