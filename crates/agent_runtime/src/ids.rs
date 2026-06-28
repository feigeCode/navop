//! 运行时各类实体的强类型标识符。
//!
//! 全部为对 `String` 的 newtype 封装,默认通过 UUID v4 生成,既能保证唯一性,
//! 又能在日志 / 事件中携带可读信息。

use serde::{Deserialize, Serialize};
use std::fmt;

/// 生成一个标准的 newtype 字符串 ID。
macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(String);

        impl $name {
            /// 生成一个带前缀的新 ID(基于 UUID v4)。
            pub fn new() -> Self {
                Self(format!("{}_{}", $prefix, uuid::Uuid::new_v4().simple()))
            }

            /// 从已有字符串构造(用于反序列化 / 持久化恢复)。
            pub fn from_string(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// 取出底层字符串切片。
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }
    };
}

string_id!(
    /// 一次会话的唯一标识。会话可包含多轮(turn)交互。
    SessionId,
    "sess"
);
string_id!(
    /// 一轮交互的唯一标识。
    TurnId,
    "turn"
);
string_id!(
    /// 一个计划的唯一标识。
    PlanId,
    "plan"
);
string_id!(
    /// 计划中单个步骤的唯一标识。
    PlanStepId,
    "step"
);
string_id!(
    /// 一次工具调用的唯一标识(与模型返回的 tool_call id 对应)。
    ToolCallId,
    "call"
);
string_id!(
    /// 运行时或外部 ACP agent 派发的子代理任务标识。
    SubAgentId,
    "subagent"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_unique_prefixed_ids() {
        let a = SessionId::new();
        let b = SessionId::new();
        assert_ne!(a, b);
        assert!(a.as_str().starts_with("sess_"));
    }

    #[test]
    fn round_trips_through_string() {
        let id = PlanId::from_string("plan_fixed");
        assert_eq!(id.as_str(), "plan_fixed");
        assert_eq!(id.to_string(), "plan_fixed");
    }
}
