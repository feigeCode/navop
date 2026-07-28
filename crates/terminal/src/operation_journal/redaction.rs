use serde::de::Error as _;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use std::fmt;

pub const OPERATION_PAYLOAD_PREVIEW_CHARS: usize = 1_200;

const REDACTED_VALUE: &str = "***";
const NON_JSON_ARGUMENTS_MARKER_PREFIX: &str = "<non-json arguments redacted bytes=";
const TRUNCATED_PREVIEW_SUFFIX: &str = "…<truncated>";
const SENSITIVE_KEY_NEEDLES: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "token",
    "apikey",
    "secret",
    "authorization",
    "cookie",
    "privatekey",
    "accesskey",
];

pub struct SensitiveOperationPayload {
    inner: SensitiveOperationPayloadInner,
}

enum SensitiveOperationPayloadInner {
    Opaque(Vec<u8>),
    Structured(Value),
}

impl SensitiveOperationPayload {
    pub fn opaque(data: impl Into<Vec<u8>>) -> Self {
        Self {
            inner: SensitiveOperationPayloadInner::Opaque(data.into()),
        }
    }

    pub fn structured(value: Value) -> Self {
        Self {
            inner: SensitiveOperationPayloadInner::Structured(value),
        }
    }

    pub fn redact(self) -> RedactedOperationPayload {
        match self.inner {
            SensitiveOperationPayloadInner::Opaque(data) => RedactedOperationPayload {
                original_byte_len: byte_len(data.len()),
                representation: RedactedOperationPayloadRepresentation::OpaqueSummary,
            },
            SensitiveOperationPayloadInner::Structured(value) => {
                let original_byte_len = structured_byte_len(&value);
                let RedactionOutcome {
                    value,
                    redaction_applied,
                } = redact_value(value);
                RedactedOperationPayload {
                    original_byte_len,
                    representation: RedactedOperationPayloadRepresentation::StructuredJson {
                        value,
                        redaction_applied,
                    },
                }
            }
        }
    }
}

impl fmt::Debug for SensitiveOperationPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (format, byte_len) = match &self.inner {
            SensitiveOperationPayloadInner::Opaque(data) => ("opaque", byte_len(data.len())),
            SensitiveOperationPayloadInner::Structured(value) => {
                ("structured_json", structured_byte_len(value))
            }
        };
        formatter
            .debug_struct("SensitiveOperationPayload")
            .field("format", &format)
            .field("byte_len", &byte_len)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationPayloadFormat {
    OpaqueSummary,
    StructuredJson,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationPayloadCompleteness {
    Complete,
    Redacted,
    SummaryOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedactedOperationPayload {
    original_byte_len: u64,
    representation: RedactedOperationPayloadRepresentation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RedactedOperationPayloadRepresentation {
    OpaqueSummary,
    StructuredJson {
        value: Value,
        redaction_applied: bool,
    },
}

impl RedactedOperationPayload {
    pub fn format(&self) -> OperationPayloadFormat {
        match self.representation {
            RedactedOperationPayloadRepresentation::OpaqueSummary => {
                OperationPayloadFormat::OpaqueSummary
            }
            RedactedOperationPayloadRepresentation::StructuredJson { .. } => {
                OperationPayloadFormat::StructuredJson
            }
        }
    }

    pub fn completeness(&self) -> OperationPayloadCompleteness {
        match &self.representation {
            RedactedOperationPayloadRepresentation::OpaqueSummary => {
                OperationPayloadCompleteness::SummaryOnly
            }
            RedactedOperationPayloadRepresentation::StructuredJson {
                redaction_applied: true,
                ..
            } => OperationPayloadCompleteness::Redacted,
            RedactedOperationPayloadRepresentation::StructuredJson {
                redaction_applied: false,
                ..
            } => OperationPayloadCompleteness::Complete,
        }
    }

    pub fn original_byte_len(&self) -> u64 {
        self.original_byte_len
    }

    pub fn redaction_applied(&self) -> bool {
        match &self.representation {
            RedactedOperationPayloadRepresentation::OpaqueSummary => true,
            RedactedOperationPayloadRepresentation::StructuredJson {
                redaction_applied, ..
            } => *redaction_applied,
        }
    }

    pub fn structured_value(&self) -> Option<&Value> {
        match &self.representation {
            RedactedOperationPayloadRepresentation::OpaqueSummary => None,
            RedactedOperationPayloadRepresentation::StructuredJson { value, .. } => Some(value),
        }
    }

    pub fn preview(&self) -> String {
        let preview = match &self.representation {
            RedactedOperationPayloadRepresentation::OpaqueSummary => {
                format!("<opaque payload redacted bytes={}>", self.original_byte_len)
            }
            RedactedOperationPayloadRepresentation::StructuredJson { value, .. } => {
                serde_json::to_string(value)
                    .unwrap_or_else(|_| "<structured payload unavailable>".to_string())
            }
        };
        bounded_preview(preview)
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), &'static str> {
        let RedactedOperationPayloadRepresentation::StructuredJson {
            value,
            redaction_applied,
        } = &self.representation
        else {
            return Ok(());
        };

        let sanitized = redact_value(value.clone());
        if sanitized.value != *value {
            return Err("structured payload contains unredacted sensitive data");
        }
        if sanitized.redaction_applied != *redaction_applied {
            return Err("structured payload redaction metadata does not match content");
        }
        Ok(())
    }
}

impl Serialize for RedactedOperationPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match &self.representation {
            RedactedOperationPayloadRepresentation::OpaqueSummary => {
                let mut state = serializer.serialize_struct("RedactedOperationPayload", 2)?;
                state.serialize_field("format", &OperationPayloadFormat::OpaqueSummary)?;
                state.serialize_field("original_byte_len", &self.original_byte_len)?;
                state.end()
            }
            RedactedOperationPayloadRepresentation::StructuredJson {
                value,
                redaction_applied,
            } => {
                let mut state = serializer.serialize_struct("RedactedOperationPayload", 4)?;
                state.serialize_field("format", &OperationPayloadFormat::StructuredJson)?;
                state.serialize_field("original_byte_len", &self.original_byte_len)?;
                state.serialize_field("value", value)?;
                state.serialize_field("redaction_applied", redaction_applied)?;
                state.end()
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RedactedOperationPayloadSnapshot {
    format: OperationPayloadFormat,
    original_byte_len: u64,
    #[serde(default)]
    value: Option<Value>,
    #[serde(default)]
    redaction_applied: Option<bool>,
}

impl<'de> Deserialize<'de> for RedactedOperationPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let snapshot = RedactedOperationPayloadSnapshot::deserialize(deserializer)?;
        let representation = match snapshot.format {
            OperationPayloadFormat::OpaqueSummary => {
                if snapshot.value.is_some() || snapshot.redaction_applied.is_some() {
                    return Err(D::Error::custom(
                        "opaque payload summary contains structured payload fields",
                    ));
                }
                RedactedOperationPayloadRepresentation::OpaqueSummary
            }
            OperationPayloadFormat::StructuredJson => {
                let value = snapshot
                    .value
                    .ok_or_else(|| D::Error::missing_field("value"))?;
                let redaction_applied = snapshot
                    .redaction_applied
                    .ok_or_else(|| D::Error::missing_field("redaction_applied"))?;
                RedactedOperationPayloadRepresentation::StructuredJson {
                    value,
                    redaction_applied,
                }
            }
        };
        let payload = Self {
            original_byte_len: snapshot.original_byte_len,
            representation,
        };
        payload.validate_snapshot().map_err(D::Error::custom)?;
        Ok(payload)
    }
}

struct RedactionOutcome {
    value: Value,
    redaction_applied: bool,
}

fn redact_value(value: Value) -> RedactionOutcome {
    match value {
        Value::Array(values) => {
            let mut redaction_applied = false;
            let values = values
                .into_iter()
                .map(|value| {
                    let outcome = redact_value(value);
                    redaction_applied |= outcome.redaction_applied;
                    outcome.value
                })
                .collect();
            RedactionOutcome {
                value: Value::Array(values),
                redaction_applied,
            }
        }
        Value::Object(values) => redact_object(values),
        value => RedactionOutcome {
            value,
            redaction_applied: false,
        },
    }
}

fn redact_object(values: Map<String, Value>) -> RedactionOutcome {
    let mut redacted_values = Map::new();
    let mut redaction_applied = false;

    for (key, value) in values {
        let normalized_key = normalize_key(&key);
        let outcome = if !key.is_ascii() || is_sensitive_key(&normalized_key) {
            RedactionOutcome {
                value: Value::String(REDACTED_VALUE.to_string()),
                redaction_applied: true,
            }
        } else if normalized_key == "arguments" {
            redact_arguments(value)
        } else {
            redact_value(value)
        };
        redaction_applied |= outcome.redaction_applied;
        redacted_values.insert(key, outcome.value);
    }

    RedactionOutcome {
        value: Value::Object(redacted_values),
        redaction_applied,
    }
}

fn redact_arguments(value: Value) -> RedactionOutcome {
    let Value::String(source) = value else {
        return redact_value(value);
    };

    if is_non_json_arguments_marker(&source) {
        return RedactionOutcome {
            value: Value::String(source),
            redaction_applied: true,
        };
    }

    let Ok(Value::Object(parsed)) = serde_json::from_str::<Value>(&source) else {
        return RedactionOutcome {
            value: Value::String(non_json_arguments_marker(source.len())),
            redaction_applied: true,
        };
    };
    let redacted = redact_value(Value::Object(parsed));
    if !redacted.redaction_applied {
        return RedactionOutcome {
            value: Value::String(source),
            redaction_applied: false,
        };
    }

    let value = serde_json::to_string(&redacted.value)
        .map(Value::String)
        .unwrap_or_else(|_| Value::String(non_json_arguments_marker(source.len())));
    RedactionOutcome {
        value,
        redaction_applied: true,
    }
}

fn normalize_key(key: &str) -> String {
    key.bytes()
        .filter(u8::is_ascii_alphanumeric)
        .map(|byte| char::from(byte.to_ascii_lowercase()))
        .collect()
}

fn is_sensitive_key(normalized_key: &str) -> bool {
    SENSITIVE_KEY_NEEDLES
        .iter()
        .any(|needle| normalized_key.contains(needle))
}

fn non_json_arguments_marker(byte_len: usize) -> String {
    format!("{NON_JSON_ARGUMENTS_MARKER_PREFIX}{byte_len}>")
}

fn is_non_json_arguments_marker(value: &str) -> bool {
    value
        .strip_prefix(NON_JSON_ARGUMENTS_MARKER_PREFIX)
        .and_then(|value| value.strip_suffix('>'))
        .is_some_and(|byte_len| {
            !byte_len.is_empty()
                && byte_len.bytes().all(|byte| byte.is_ascii_digit())
                && byte_len.parse::<u64>().is_ok()
        })
}

fn bounded_preview(preview: String) -> String {
    if preview.chars().count() <= OPERATION_PAYLOAD_PREVIEW_CHARS {
        return preview;
    }
    let mut bounded = preview
        .chars()
        .take(OPERATION_PAYLOAD_PREVIEW_CHARS)
        .collect::<String>();
    bounded.push_str(TRUNCATED_PREVIEW_SUFFIX);
    bounded
}

fn structured_byte_len(value: &Value) -> u64 {
    serde_json::to_vec(value)
        .map(|encoded| byte_len(encoded.len()))
        .unwrap_or(u64::MAX)
}

fn byte_len(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
