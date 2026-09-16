//! 实时消息事件(events 模板)的结构化模型。
//!
//! events 模板页的 `load` 操作返回事件流引用(标准 §3 `ResultRef::EventStream`),
//! 宿主长轮询读到的每条事件都是中间件标准消息模型(`MiddlewareMessage`)的 JSON。
//! 本模块把它解析成一个可读的行模型(topic + QoS/Retain 徽章 + 时间 + 消息体),
//! 让「实时消息」页呈现为消息列表,而不是一串裸 JSON。
//!
//! 解析是**尽力而为**的:events 模板对所有扩展开放,拿到不是消息模型的事件
//! (例如运维事件)时返回 `None`,UI 回落为原始 JSON 行 —— 模板的通用性不会
//! 被某个扩展的数据形状绑死。

use serde_json::Value;

/// `properties` 中已有专属展示位置的键(不再重复出现在「其他属性」里)。
const PROPERTY_QOS: &str = "qos";
const PROPERTY_RETAIN: &str = "retain";
const PROPERTY_RECEIVED_AT_MS: &str = "received_at_ms";

/// 一条按标准消息模型解析出来的实时事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MessageEvent {
    /// 消息 ID(如 `mqtt-live-<seq>`;缺失时不在 UI 展示)。
    pub(crate) message_id: Option<String>,
    /// Topic。消息模型里唯一必填字段,也是判定「这是消息事件」的依据。
    pub(crate) topic: String,
    /// QoS 文案(协议约定放在 `properties.qos`,如 `QoS 1`)。
    pub(crate) qos: Option<String>,
    /// 保留消息标记(协议约定放在 `properties.retain`);置真时 UI 出徽章。
    pub(crate) retain: bool,
    /// 时间文案:`store_time` → `born_time` → `properties.received_at_ms`。
    pub(crate) timestamp: Option<String>,
    /// 消息体文本(`body_text`;只有二进制体时退化为 `<N bytes>`)。
    pub(crate) body: Option<String>,
    /// 其余 `properties`(qos / retain / received_at_ms 之外的键值)。
    pub(crate) extra: Vec<(String, String)>,
}

impl MessageEvent {
    /// 解析一条事件 JSON;不是消息模型时返回 `None`(由调用方回落原始 JSON)。
    pub(crate) fn parse(event: &Value) -> Option<Self> {
        let object = event.as_object()?;
        let topic = object.get("topic")?.as_str()?;
        if topic.is_empty() {
            return None;
        }
        let mut qos = None;
        let mut retain = false;
        let mut received_at_ms = None;
        let mut extra = Vec::new();
        for (key, value) in properties_of(object) {
            match key.as_str() {
                PROPERTY_QOS => qos = Some(value),
                PROPERTY_RETAIN => retain = parse_flag(&value),
                // 毫秒时间戳只作为 `store_time` 缺失时的兜底,避免同一时间出现两次。
                PROPERTY_RECEIVED_AT_MS => received_at_ms = Some(value),
                _ => extra.push((key, value)),
            }
        }
        let timestamp = string_field(object, "store_time")
            .or_else(|| string_field(object, "born_time"))
            .or(received_at_ms);
        let body = string_field(object, "body_text").or_else(|| match object.get("body") {
            None | Some(Value::Null) => None,
            Some(value) => Some(describe_body(value)),
        });
        Some(Self {
            message_id: string_field(object, "message_id"),
            topic: topic.to_string(),
            qos,
            retain,
            timestamp,
            body,
            extra,
        })
    }
}

/// 取一个非空字符串字段。
fn string_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    match object.get(key) {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

/// `properties` 是 `Vec<(String, String)>` 的 JSON 形式(`[["qos","QoS 1"], …]`)。
/// 非数组、非「两元素字符串对」的条目直接跳过,脏数据不会让整条事件解析失败。
fn properties_of(object: &serde_json::Map<String, Value>) -> Vec<(String, String)> {
    let Some(Value::Array(entries)) = object.get("properties") else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let [key, value] = entry.as_array()?.as_slice() else {
                return None;
            };
            Some((key.as_str()?.to_string(), value.as_str()?.to_string()))
        })
        .collect()
}

/// 布尔型属性文案(`"true"` / `"1"` / `"yes"` / `"on"` 都算真)。
fn parse_flag(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on"
    )
}

/// 没有 `body_text` 时的消息体描述:字节数组退化为长度占位。
fn describe_body(value: &Value) -> String {
    match value {
        Value::Array(bytes) => format!("<{} bytes>", bytes.len()),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// provider 实测推送的形状(见 mqtt `admin::live_to_model`)。
    fn mqtt_event() -> Value {
        json!({
            "message_id": "mqtt-live-7",
            "topic": "sensors/temp",
            "tag": null,
            "key": null,
            "body": null,
            "body_text": "23.5",
            "store_time": "2026-09-14 10:47:12",
            "born_time": null,
            "properties": [
                ["qos", "QoS 1"],
                ["retain", "false"],
                ["received_at_ms", "1757818032000"],
            ],
        })
    }

    #[test]
    fn parses_provider_message_event() {
        let event = MessageEvent::parse(&mqtt_event()).expect("message event");
        assert_eq!(Some("mqtt-live-7".to_string()), event.message_id);
        assert_eq!("sensors/temp", event.topic);
        assert_eq!(Some("QoS 1".to_string()), event.qos);
        assert!(!event.retain);
        assert_eq!(Some("2026-09-14 10:47:12".to_string()), event.timestamp);
        assert_eq!(Some("23.5".to_string()), event.body);
        // 三个已知属性都有专属位置,不重复进 extra
        assert!(event.extra.is_empty());
    }

    #[test]
    fn rejects_events_that_are_not_messages() {
        // events 模板对任意扩展开放:没有 topic 就回落原始 JSON 行
        assert!(MessageEvent::parse(&json!({"event": "started"})).is_none());
        assert!(MessageEvent::parse(&json!({"topic": ""})).is_none());
        assert!(MessageEvent::parse(&json!({"topic": 42})).is_none());
        assert!(MessageEvent::parse(&json!("plain string")).is_none());
        assert!(MessageEvent::parse(&Value::Null).is_none());
    }

    #[test]
    fn retain_and_missing_fields_are_optional() {
        let event = MessageEvent::parse(&json!({
            "topic": "a/b",
            "properties": [["retain", "true"]],
        }))
        .expect("message event");
        assert!(event.retain);
        assert!(event.message_id.is_none());
        assert!(event.qos.is_none());
        assert!(event.timestamp.is_none());
        assert!(event.body.is_none());
    }

    #[test]
    fn timestamp_falls_back_to_received_at_ms() {
        let event = MessageEvent::parse(&json!({
            "topic": "a/b",
            "properties": [["received_at_ms", "1757818032000"]],
        }))
        .expect("message event");
        assert_eq!(Some("1757818032000".to_string()), event.timestamp);
        // 兜底用掉之后不再重复出现在 extra
        assert!(event.extra.is_empty());
    }

    #[test]
    fn binary_body_becomes_a_length_placeholder() {
        let event = MessageEvent::parse(&json!({
            "topic": "a/b",
            "body": [0, 255, 1],
        }))
        .expect("message event");
        assert_eq!(Some("<3 bytes>".to_string()), event.body);
    }

    #[test]
    fn unknown_properties_land_in_extra() {
        let event = MessageEvent::parse(&json!({
            "topic": "a/b",
            "properties": [
                ["qos", "QoS 2"],
                ["client_id", "navop-1"],
                ["nonsense"],
                ["dup", 7],
            ],
        }))
        .expect("message event");
        assert_eq!(Some("QoS 2".to_string()), event.qos);
        assert_eq!(
            vec![("client_id".to_string(), "navop-1".to_string())],
            event.extra
        );
    }
}
