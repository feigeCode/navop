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

/// 稳定序列化：对象键排序，避免字段顺序变化造成假变更。
///
/// 采用「键排序 → `serde_json` 序列化」两步而非手写转义器：`serde_json` 的
/// 字符串转义覆盖全部控制字符（如 `\u{1b}`），手写版漏掉它们会在设置值包含
/// 控制字符时产出无法反序列化的 JSON，让该分组从此永远同步失败。
pub fn canonical_json(payload: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let mut sorted_map = serde_json::Map::new();
                for key in keys {
                    sorted_map.insert(key.clone(), sorted(&map[key]));
                }
                Value::Object(sorted_map)
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }

    serde_json::to_string(&sorted(payload)).expect("serde_json 对 Value 序列化不会失败")
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

    #[test]
    fn canonical_json_escapes_control_characters() {
        // 控制字符（如 ESC）必须被转义，否则产出的 JSON 无法反序列化
        let value = json!({"theme": "a\u{1b}b"});
        let text = canonical_json(&value);

        assert!(text.contains("\\u001b"), "控制字符未被转义: {text}");
        // 产出的必须是合法 JSON
        assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok());
    }
}
