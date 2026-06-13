//! 反向 Host API (`host/*`)。
//!
//! 「反向」是说调用方向相反——这些请求由**扩展进程**发起,**宿主**响应。
//! 用同样的 JSON-RPC envelope,只是 method 前缀统一 `host/`。
//!
//! 所有 Host API 调用都受 `permissions` 校验(扩展必须在 manifest 声明):
//!
//! ```text
//! permission                 | host API
//! ─────────────────────────────────────────────────────
//! secrets:read:<glob>        | host/request_credential(只读已存)
//! secrets:write:<glob>       | host/request_credential(save_as)
//! notifications:show         | host/notify
//! ui:dialog                  | host/quick_pick / host/confirm / host/open_view
//! host:ssh_tunnel            | host/ssh/open_tunnel
//! storage:read/write         | host/storage/*
//! logs:write                 | host/log
//! ```
//!
//! 详见 [`docs/design/extensions/api-database.md`] §16 与 `security.md`。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::conn::SecretRef;

// ============================================================================
// host/request_credential
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestCredentialParams {
    /// `password` / `keyfile` / `token` / `oauth_code` / ...。
    pub kind: String,
    /// 给用户看的提示文案。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// 如果填了,host 在拿到值后会写入 secret store 并把同一 ref 返回。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_as: Option<String>,
    /// 用户是否能勾「记住此密码」。
    #[serde(default)]
    pub remember_option: bool,
    /// 失败可重试次数(0 表示一次性)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestCredentialResult {
    /// 引用,不返回明文值——扩展继续调 `conn/open` 时把 ref 放 credentials map。
    pub secret_ref: SecretRef,
    /// 用户是否选了「记住」(只在 `remember_option` 为 true 时有意义)。
    #[serde(default)]
    pub remembered: bool,
}

// ============================================================================
// host/notify
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyLevel {
    Info,
    Warning,
    Error,
    Success,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyParams {
    pub level: NotifyLevel,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// 持续多少 ms;`None` 走 host 默认值;0 表示常驻。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u32>,
    /// 通知里附带的按钮(host 渲染,扩展不知用户是否点击)。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<NotifyAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyAction {
    /// 用户点击后扩展应该收到的回调命令 id。
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub primary: bool,
}

/// notify 无返回数据,但仍是请求(可能阻塞通知队列)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotifyResult {
    /// 用户点击的 action id(若有);超时关闭则为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clicked: Option<String>,
}

// ============================================================================
// host/quick_pick
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickPickParams {
    pub items: Vec<QuickPickItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// 是否允许多选(默认 false)。
    #[serde(default)]
    pub can_pick_many: bool,
    /// 是否模糊匹配(默认 true)。
    #[serde(default = "default_true_field")]
    pub fuzzy_match: bool,
    /// 用户按 Esc 是否允许(默认 true)。
    #[serde(default = "default_true_field")]
    pub cancellable: bool,
}

fn default_true_field() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuickPickItem {
    /// 扩展自定义 id,host 回 `selected`。
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// 图标资源标识(host 自有 icon set 中的名字)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 默认选中(can_pick_many 时多个可同时为 true)。
    #[serde(default)]
    pub picked: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuickPickResult {
    /// 用户选中的 item id 列表。
    #[serde(default)]
    pub selected: Vec<String>,
    /// 用户取消(按 Esc)。
    #[serde(default)]
    pub cancelled: bool,
}

// ============================================================================
// host/confirm
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmParams {
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// 危险操作(确认按钮变红)。
    #[serde(default)]
    pub danger: bool,
    /// 确认按钮文案,默认 "确认" / "OK"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_label: Option<String>,
    /// 取消按钮文案,默认 "取消" / "Cancel"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmResult {
    pub confirmed: bool,
}

// ============================================================================
// host/open_view
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenViewParams {
    /// 由扩展 manifest 在 `contributes.views[]` 注册的 view id。
    pub view_id: String,
    /// 任意状态对象,host 转交给目标 view。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub state: Value,
    /// 是否在新 tab / 新 panel 打开。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<OpenViewTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenViewTarget {
    /// 当前 active tab 替换。
    Current,
    /// 新 tab。
    NewTab,
    /// 侧边面板。
    SidePanel,
    /// 弹窗。
    Dialog,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenViewResult {
    /// host 分配的 view 实例 id,扩展可用于发后续指令(广播事件等)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
}

// ============================================================================
// host/ssh/open_tunnel
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshOpenTunnelParams {
    /// 宿主已有的 SSH 连接(配置 id)。
    pub ssh_connection_id: String,
    /// 隧道目标主机,默认 `127.0.0.1`。
    #[serde(default = "default_tunnel_host")]
    pub remote_host: String,
    pub remote_port: u16,
    /// 本地映射端口,`None` 表示自动分配。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_port: Option<u16>,
    /// 隧道空闲超时(秒);0 表示常驻直到扩展关闭。
    #[serde(default = "default_tunnel_idle_secs")]
    pub idle_secs: u32,
}

fn default_tunnel_host() -> String {
    "127.0.0.1".into()
}

fn default_tunnel_idle_secs() -> u32 {
    600
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshOpenTunnelResult {
    pub session_ref: String,
    /// 实际本地端口(可能与请求不同)。
    pub local_port: u16,
}

// ============================================================================
// host/storage/*
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageGetParams {
    pub key: String,
    /// 命名空间;默认是扩展 id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageGetResult {
    /// 不存在则为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageSetParams {
    pub key: String,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// 过期时间(秒),`None` 永久。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<u32>,
}

// ============================================================================
// host/log
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogParams {
    pub level: LogLevel,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// 结构化字段(`tracing` 风格)。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub fields: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_credential_params_minimal() {
        let p = RequestCredentialParams {
            kind: "password".into(),
            prompt: Some("Enter password".into()),
            save_as: None,
            remember_option: false,
            retry_count: None,
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: RequestCredentialParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.kind, "password");
        assert_eq!(parsed.prompt.as_deref(), Some("Enter password"));
        assert!(parsed.save_as.is_none());
    }

    #[test]
    fn request_credential_result_round_trip() {
        let r = RequestCredentialResult {
            secret_ref: SecretRef::new("kss://x/y"),
            remembered: true,
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: RequestCredentialResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.secret_ref.secret_ref, "kss://x/y");
        assert!(parsed.remembered);
    }

    #[test]
    fn notify_level_serde_lowercase() {
        assert_eq!(
            serde_json::to_string(&NotifyLevel::Info).unwrap(),
            r#""info""#
        );
        assert_eq!(
            serde_json::to_string(&NotifyLevel::Warning).unwrap(),
            r#""warning""#
        );
        let parsed: NotifyLevel = serde_json::from_str(r#""error""#).unwrap();
        assert_eq!(parsed, NotifyLevel::Error);
    }

    #[test]
    fn notify_params_with_actions() {
        let p = NotifyParams {
            level: NotifyLevel::Info,
            title: "Backup completed".into(),
            body: "All tables backed up".into(),
            duration_ms: Some(5_000),
            actions: vec![NotifyAction {
                id: "open_folder".into(),
                label: "Open Folder".into(),
                primary: true,
            }],
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: NotifyParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.actions.len(), 1);
        assert!(parsed.actions[0].primary);
    }

    #[test]
    fn notify_params_skip_empty_body_and_actions() {
        let p = NotifyParams {
            level: NotifyLevel::Success,
            title: "ok".into(),
            body: String::new(),
            duration_ms: None,
            actions: vec![],
        };
        let j = serde_json::to_string(&p).unwrap();
        assert!(!j.contains("body"));
        assert!(!j.contains("actions"));
        assert!(!j.contains("duration_ms"));
    }

    #[test]
    fn notify_result_optional_clicked() {
        let r = NotifyResult {
            clicked: Some("open_folder".into()),
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains(r#""clicked":"open_folder""#));

        let r2 = NotifyResult::default();
        let j2 = serde_json::to_string(&r2).unwrap();
        assert!(!j2.contains("clicked"));
    }

    #[test]
    fn quick_pick_params_default_flags() {
        let p: QuickPickParams =
            serde_json::from_str(r#"{"items":[{"id":"a","label":"A"}]}"#).unwrap();
        assert_eq!(p.items.len(), 1);
        assert!(!p.can_pick_many);
        assert!(p.fuzzy_match);
        assert!(p.cancellable);
    }

    #[test]
    fn quick_pick_item_round_trip() {
        let i = QuickPickItem {
            id: "a".into(),
            label: "Alice".into(),
            description: Some("user".into()),
            detail: Some("ID 1".into()),
            icon: Some("user.svg".into()),
            picked: true,
        };
        let j = serde_json::to_string(&i).unwrap();
        let parsed: QuickPickItem = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.label, "Alice");
        assert!(parsed.picked);
    }

    #[test]
    fn quick_pick_result_with_selection() {
        let r = QuickPickResult {
            selected: vec!["a".into(), "b".into()],
            cancelled: false,
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: QuickPickResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.selected.len(), 2);
        assert!(!parsed.cancelled);
    }

    #[test]
    fn confirm_params_with_danger() {
        let p = ConfirmParams {
            title: "Drop table?".into(),
            body: "This action is irreversible".into(),
            danger: true,
            confirm_label: Some("Drop".into()),
            cancel_label: Some("Cancel".into()),
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: ConfirmParams = serde_json::from_str(&j).unwrap();
        assert!(parsed.danger);
        assert_eq!(parsed.confirm_label.as_deref(), Some("Drop"));
    }

    #[test]
    fn confirm_result_round_trip() {
        let r = ConfirmResult { confirmed: true };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: ConfirmResult = serde_json::from_str(&j).unwrap();
        assert!(parsed.confirmed);
    }

    #[test]
    fn open_view_target_serde() {
        assert_eq!(
            serde_json::to_string(&OpenViewTarget::NewTab).unwrap(),
            r#""new_tab""#
        );
        assert_eq!(
            serde_json::to_string(&OpenViewTarget::SidePanel).unwrap(),
            r#""side_panel""#
        );
        let parsed: OpenViewTarget = serde_json::from_str(r#""dialog""#).unwrap();
        assert_eq!(parsed, OpenViewTarget::Dialog);
    }

    #[test]
    fn open_view_params_with_state() {
        let p = OpenViewParams {
            view_id: "ext.cassandra.backup".into(),
            state: serde_json::json!({"keyspace": "ks1"}),
            target: Some(OpenViewTarget::SidePanel),
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: OpenViewParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.view_id, "ext.cassandra.backup");
        assert_eq!(parsed.target, Some(OpenViewTarget::SidePanel));
    }

    #[test]
    fn ssh_open_tunnel_params_defaults() {
        let p: SshOpenTunnelParams =
            serde_json::from_str(r#"{"ssh_connection_id":"s1","remote_port":3306}"#).unwrap();
        assert_eq!(p.remote_host, "127.0.0.1");
        assert_eq!(p.idle_secs, 600);
        assert!(p.local_port.is_none());
    }

    #[test]
    fn ssh_open_tunnel_result_round_trip() {
        let r = SshOpenTunnelResult {
            session_ref: "sht://onetcli/ssh/abc".into(),
            local_port: 13306,
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: SshOpenTunnelResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.local_port, 13306);
    }

    #[test]
    fn storage_get_result_with_value() {
        let r = StorageGetResult {
            value: Some(serde_json::json!({"x": 1})),
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: StorageGetResult = serde_json::from_str(&j).unwrap();
        assert!(parsed.value.is_some());
    }

    #[test]
    fn storage_get_result_none_omits_field() {
        let r = StorageGetResult { value: None };
        let j = serde_json::to_string(&r).unwrap();
        assert_eq!(j, "{}");
    }

    #[test]
    fn storage_set_params_with_ttl() {
        let p = StorageSetParams {
            key: "x".into(),
            value: serde_json::json!(true),
            namespace: Some("ext.cass".into()),
            ttl_secs: Some(3600),
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: StorageSetParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.ttl_secs, Some(3600));
    }

    #[test]
    fn log_level_serde() {
        assert_eq!(
            serde_json::to_string(&LogLevel::Debug).unwrap(),
            r#""debug""#
        );
        assert_eq!(serde_json::to_string(&LogLevel::Warn).unwrap(), r#""warn""#);
    }

    #[test]
    fn log_params_with_structured_fields() {
        let p = LogParams {
            level: LogLevel::Info,
            message: "query executed".into(),
            module: Some("driver".into()),
            fields: serde_json::json!({"rows": 100, "ms": 5}),
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: LogParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.level, LogLevel::Info);
        assert_eq!(parsed.module.as_deref(), Some("driver"));
        assert_eq!(parsed.fields, serde_json::json!({"rows": 100, "ms": 5}));
    }
}
