//! RocketMQ 连接表单:复用 connection_form 的通用中间件声明式表单引擎
//!
//! 结构与 MQTT 表单一致:常规/RocketMQ/高级/SSH/备注 标签页,
//! 钥匙串/工作区/团队/云同步由引擎统一渲染。本文件只提供:
//! - `rocketmq_form_tab_groups()`:RocketMQ 的声明式标签页配置
//! - `RocketmqFormAdapter`:`RocketmqParams` 与表单快照的双向映射 + 测试连接
//! - `RocketmqFormConfig`/`RocketmqFormWindow`:对通用窗口的薄封装

use std::collections::HashMap;
use std::sync::Arc;

use connection_form::credential::resolve_connection_for_runtime;
use connection_form::middleware_form::{
    FormField, FormFieldType, FormSnapshot, MiddlewareFormAdapter, MiddlewareFormSavedCallback,
    MiddlewareFormWindow, MiddlewareFormWindowConfig, TabGroup, notes_tab_group, ssh_tab_group,
};
use gpui::{App, AsyncApp, Task};
use one_core::cloud_sync::TeamOption;
use one_core::gpui_tokio::Tokio;
use one_core::storage::{
    ConnectionType, RocketmqParams, RocketmqSshTunnelConfig, StoredConnection, Workspace,
};
use rust_i18n::t;

use crate::manager::{GlobalRocketmqState, RocketmqManager};

/// RocketMQ 表单窗口(通用中间件窗口的类型别名)
pub type RocketmqFormWindow = MiddlewareFormWindow;

/// 保存成功回调(与通用中间件表单一致)
pub type RocketmqFormSavedCallback = MiddlewareFormSavedCallback;

/// RocketMQ 表单配置
pub struct RocketmqFormConfig {
    /// 正在编辑的连接(`None` 表示新建)
    pub editing_connection: Option<StoredConnection>,
    /// 预填连接(不进入编辑模式)
    pub initial_connection: Option<StoredConnection>,
    /// 保存成功回调
    pub on_saved: Option<RocketmqFormSavedCallback>,
    /// 可选工作区列表
    pub workspaces: Vec<Workspace>,
    /// 可选团队列表
    pub teams: Vec<TeamOption>,
    /// 可选 SSH 连接(用于隧道引用下拉)
    pub ssh_connections: Vec<StoredConnection>,
}

impl RocketmqFormConfig {
    /// 是否处于编辑模式(存在正在编辑的连接)
    ///
    /// 编辑模式下表单回填既有连接参数,保存时保留云同步元数据。
    pub fn is_editing(&self) -> bool {
        self.editing_connection.is_some()
    }

    /// 转换为通用中间件表单窗口配置(注入 RocketMQ 适配器与标签页)
    pub fn into_window_config(self) -> MiddlewareFormWindowConfig {
        MiddlewareFormWindowConfig {
            adapter: Arc::new(RocketmqFormAdapter),
            tab_groups: rocketmq_form_tab_groups(),
            editing_connection: self.editing_connection,
            initial_connection: self.initial_connection,
            on_saved: self.on_saved,
            workspaces: self.workspaces,
            teams: self.teams,
            ssh_connections: self.ssh_connections,
        }
    }
}

/// RocketMQ 声明式标签页配置
///
/// 常规 / RocketMQ(中间件特性扩展) / 高级 / SSH / 备注,
/// SSH 与备注页复用引擎提供的共享构造器。
pub fn rocketmq_form_tab_groups() -> Vec<TabGroup> {
    vec![
        TabGroup::new("general", t!("RocketmqForm.tab_general").to_string()).fields(vec![
            FormField::new("name", t!("RocketmqForm.name"), FormFieldType::Text)
                .placeholder(t!("RocketmqForm.name_placeholder"))
                .default("Local RocketMQ"),
            FormField::new(
                "namesrv_addrs",
                t!("RocketmqForm.namesrv_addrs"),
                FormFieldType::TextArea,
            )
            .placeholder(t!("RocketmqForm.namesrv_addrs_placeholder"))
            .default("127.0.0.1:9876"),
        ]),
        TabGroup::new("rocketmq", t!("RocketmqForm.tab_rocketmq").to_string()).fields(vec![
            FormField::new(
                "access_key",
                t!("RocketmqForm.access_key"),
                FormFieldType::Text,
            )
            .optional()
            .placeholder(t!("RocketmqForm.access_key_placeholder")),
            FormField::new(
                "secret_key",
                t!("RocketmqForm.secret_key"),
                FormFieldType::Password,
            )
            .optional()
            .placeholder(t!("RocketmqForm.secret_key_placeholder")),
            FormField::new("domain", t!("RocketmqForm.domain"), FormFieldType::Text)
                .optional()
                .placeholder(t!("RocketmqForm.domain_placeholder")),
        ]),
        TabGroup::new("advanced", t!("RocketmqForm.tab_advanced").to_string()).fields(vec![
            FormField::new(
                "connect_timeout",
                t!("RocketmqForm.connect_timeout"),
                FormFieldType::Number,
            )
            .optional()
            .placeholder("5")
            .default("5"),
            FormField::new(
                "request_timeout",
                t!("RocketmqForm.request_timeout"),
                FormFieldType::Number,
            )
            .optional()
            .placeholder("3000")
            .default("3000"),
        ]),
        ssh_tab_group(),
        notes_tab_group(),
    ]
}

/// RocketMQ 表单适配器
pub struct RocketmqFormAdapter;

fn optional_input(value: Option<&String>) -> Option<String> {
    value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn bool_field(fields: &HashMap<String, String>, name: &str) -> bool {
    fields
        .get(name)
        .is_some_and(|value| value == "true" || value == "1")
}

fn number_field<T>(fields: &HashMap<String, String>, name: &str) -> Option<T>
where
    T: std::str::FromStr,
{
    fields.get(name).and_then(|value| value.trim().parse().ok())
}

/// 解析 NameServer 地址多值输入:按换行/分号/逗号分隔,去空白
fn parse_namesrv_addrs(raw: &str) -> Vec<String> {
    raw.lines()
        .flat_map(|line| line.split([';', ',']))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

/// NameServer 地址列表序列化为多行文本(编辑回填)
fn format_namesrv_addrs(addrs: &[String]) -> String {
    addrs.join("\n")
}

impl MiddlewareFormAdapter for RocketmqFormAdapter {
    fn connection_type(&self) -> ConnectionType {
        ConnectionType::Rocketmq
    }

    fn load_fields(&self, connection: &StoredConnection) -> Result<FormSnapshot, String> {
        let params = connection
            .to_rocketmq_params()
            .map_err(|error| error.to_string())?;

        let mut fields = HashMap::new();
        fields.insert(
            "namesrv_addrs".to_string(),
            format_namesrv_addrs(&params.namesrv_addrs),
        );
        fields.insert(
            "access_key".to_string(),
            params.access_key.clone().unwrap_or_default(),
        );
        fields.insert(
            "secret_key".to_string(),
            params.secret_key.clone().unwrap_or_default(),
        );
        fields.insert(
            "domain".to_string(),
            params.domain.clone().unwrap_or_default(),
        );
        fields.insert(
            "connect_timeout".to_string(),
            params
                .connect_timeout
                .map(|value| value.to_string())
                .unwrap_or_default(),
        );
        fields.insert(
            "request_timeout".to_string(),
            params
                .request_timeout
                .map(|value| value.to_string())
                .unwrap_or_default(),
        );

        if let Some(tunnel) = &params.ssh_tunnel {
            fields.insert(
                "ssh_tunnel_enabled".to_string(),
                if tunnel.enabled { "true" } else { "false" }.to_string(),
            );
            fields.insert(
                "ssh_connection_id".to_string(),
                tunnel
                    .connection_id
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
            );
            fields.insert("ssh_host".to_string(), tunnel.host.clone());
            fields.insert("ssh_port".to_string(), tunnel.port.to_string());
            fields.insert("ssh_username".to_string(), tunnel.username.clone());
            fields.insert("ssh_auth_type".to_string(), tunnel.auth_type.clone());
            fields.insert(
                "ssh_password".to_string(),
                tunnel.password.clone().unwrap_or_default(),
            );
            fields.insert(
                "ssh_private_key_path".to_string(),
                tunnel.private_key_path.clone().unwrap_or_default(),
            );
            fields.insert(
                "ssh_private_key_content".to_string(),
                tunnel.private_key_content.clone().unwrap_or_default(),
            );
            fields.insert(
                "ssh_private_key_passphrase".to_string(),
                tunnel.private_key_passphrase.clone().unwrap_or_default(),
            );
            fields.insert(
                "ssh_target_host".to_string(),
                tunnel.target_host.clone().unwrap_or_default(),
            );
            fields.insert(
                "ssh_target_port".to_string(),
                tunnel
                    .target_port
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
            );
        }

        Ok(FormSnapshot {
            fields,
            extras: HashMap::new(),
            credential_reference: params.credential_reference.clone(),
        })
    }

    fn build_connection(
        &self,
        snapshot: &FormSnapshot,
        name: String,
        workspace_id: Option<i64>,
    ) -> Result<StoredConnection, String> {
        let fields = &snapshot.fields;

        let namesrv_addrs = parse_namesrv_addrs(
            fields
                .get("namesrv_addrs")
                .map(String::as_str)
                .unwrap_or_default(),
        );
        if namesrv_addrs.is_empty() {
            return Err(t!("RocketmqForm.namesrv_required").to_string());
        }

        // 选择钥匙串引用时引擎会省略手动 ACL 凭据字段
        let (access_key, secret_key) = if snapshot.credential_reference.is_some() {
            (None, None)
        } else {
            (
                optional_input(fields.get("access_key")),
                optional_input(fields.get("secret_key")),
            )
        };

        let ssh_tunnel = if bool_field(fields, "ssh_tunnel_enabled") {
            Some(RocketmqSshTunnelConfig {
                enabled: true,
                connection_id: number_field(fields, "ssh_connection_id"),
                host: fields.get("ssh_host").cloned().unwrap_or_default(),
                port: number_field(fields, "ssh_port").unwrap_or(22),
                username: fields.get("ssh_username").cloned().unwrap_or_default(),
                auth_type: fields
                    .get("ssh_auth_type")
                    .cloned()
                    .unwrap_or_else(|| "password".to_string()),
                password: optional_input(fields.get("ssh_password")),
                private_key_path: optional_input(fields.get("ssh_private_key_path")),
                private_key_content: optional_input(fields.get("ssh_private_key_content")),
                private_key_passphrase: optional_input(fields.get("ssh_private_key_passphrase")),
                target_host: optional_input(fields.get("ssh_target_host")),
                target_port: number_field(fields, "ssh_target_port"),
                timeout: None,
            })
        } else {
            None
        };

        let params = RocketmqParams {
            namesrv_addrs,
            access_key,
            secret_key,
            credential_reference: snapshot.credential_reference.clone(),
            domain: optional_input(fields.get("domain")),
            connect_timeout: number_field(fields, "connect_timeout"),
            request_timeout: number_field(fields, "request_timeout"),
            ssh_tunnel,
        };

        Ok(StoredConnection::new_rocketmq(name, params, workspace_id))
    }

    fn default_name(&self, snapshot: &FormSnapshot) -> String {
        // 默认名称回退首个 NameServer 地址
        snapshot
            .fields
            .get("namesrv_addrs")
            .map(|raw| parse_namesrv_addrs(raw))
            .and_then(|addrs| addrs.first().cloned())
            .unwrap_or_else(|| "127.0.0.1:9876".to_string())
    }

    fn test_connection(
        &self,
        connection: &StoredConnection,
        cx: &mut App,
    ) -> Task<Result<(), String>> {
        let params = resolve_connection_for_runtime(connection.clone(), cx).and_then(|resolved| {
            RocketmqManager::params_from_stored(&resolved).map_err(|error| error.to_string())
        });
        let params = match params {
            Ok(params) => params,
            Err(error) => {
                return cx.spawn(async move |_cx: &mut AsyncApp| Err::<(), String>(error));
            }
        };

        let Some(global_state) = cx.try_global::<GlobalRocketmqState>().cloned() else {
            return cx.spawn(async move |_cx: &mut AsyncApp| {
                Err::<(), String>(t!("RocketmqForm.state_missing").to_string())
            });
        };

        cx.spawn(async move |cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, async move {
                global_state
                    .test_connection(params)
                    .await
                    .map_err(anyhow::Error::new)
            })
            .await;

            result.map_err(|error| format!("{error:#}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_for(params: &RocketmqParams) -> FormSnapshot {
        let mut stored = StoredConnection::new_rocketmq("测试".to_string(), params.clone(), None);
        stored.id = Some(7);
        RocketmqFormAdapter
            .load_fields(&stored)
            .expect("回填应成功")
    }

    fn params() -> RocketmqParams {
        RocketmqParams {
            namesrv_addrs: vec!["10.0.0.1:9876".to_string(), "10.0.0.2:9876".to_string()],
            access_key: Some("rocketmq".to_string()),
            secret_key: Some("12345678".to_string()),
            credential_reference: None,
            domain: Some("prod".to_string()),
            connect_timeout: Some(8),
            request_timeout: Some(5000),
            ssh_tunnel: None,
        }
    }

    #[test]
    fn tab_groups_declare_expected_tabs_and_fields() {
        let groups = rocketmq_form_tab_groups();
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();

        assert_eq!(
            names,
            vec!["general", "rocketmq", "advanced", "ssh", "notes"]
        );
        assert!(groups[0].fields.iter().any(|f| f.name == "namesrv_addrs"));
        assert!(groups[1].fields.iter().any(|f| f.name == "access_key"));
        assert!(groups[1].fields.iter().any(|f| f.name == "secret_key"));
        assert!(groups[2].fields.iter().any(|f| f.name == "connect_timeout"));
        assert!(groups[2].fields.iter().any(|f| f.name == "request_timeout"));
        assert!(
            groups[3]
                .fields
                .iter()
                .any(|f| f.name == "ssh_tunnel_enabled")
        );
        assert!(groups[4].fields.iter().any(|f| f.name == "remark"));
    }

    #[test]
    fn load_and_build_round_trip_all_fields() {
        let snapshot = snapshot_for(&params());

        assert_eq!(
            snapshot.fields.get("namesrv_addrs").unwrap(),
            "10.0.0.1:9876\n10.0.0.2:9876"
        );
        assert_eq!(snapshot.fields.get("access_key").unwrap(), "rocketmq");
        assert_eq!(snapshot.fields.get("secret_key").unwrap(), "12345678");
        assert_eq!(snapshot.fields.get("domain").unwrap(), "prod");
        assert_eq!(snapshot.fields.get("connect_timeout").unwrap(), "8");
        assert_eq!(snapshot.fields.get("request_timeout").unwrap(), "5000");

        let stored = RocketmqFormAdapter
            .build_connection(&snapshot, "生产集群".to_string(), Some(3))
            .expect("构建应成功");
        let rebuilt = stored.to_rocketmq_params().expect("参数应可解析");

        assert_eq!(rebuilt.namesrv_addrs, params().namesrv_addrs);
        assert_eq!(rebuilt.access_key.as_deref(), Some("rocketmq"));
        assert_eq!(rebuilt.secret_key.as_deref(), Some("12345678"));
        assert_eq!(rebuilt.domain.as_deref(), Some("prod"));
        assert_eq!(rebuilt.connect_timeout, Some(8));
        assert_eq!(rebuilt.request_timeout, Some(5000));
        assert_eq!(stored.workspace_id, Some(3));
        assert_eq!(stored.connection_type, ConnectionType::Rocketmq);
    }

    #[test]
    fn parse_namesrv_addrs_supports_multiline_and_separators() {
        assert_eq!(
            parse_namesrv_addrs("10.0.0.1:9876\n10.0.0.2:9876"),
            vec!["10.0.0.1:9876".to_string(), "10.0.0.2:9876".to_string()]
        );
        assert_eq!(
            parse_namesrv_addrs("10.0.0.1; 10.0.0.2,10.0.0.3"),
            vec![
                "10.0.0.1".to_string(),
                "10.0.0.2".to_string(),
                "10.0.0.3".to_string()
            ]
        );
        assert!(parse_namesrv_addrs("  \n ; ").is_empty());
    }

    #[test]
    fn round_trip_preserves_ssh_tunnel() {
        let mut source = params();
        source.ssh_tunnel = Some(RocketmqSshTunnelConfig {
            enabled: true,
            connection_id: Some(42),
            host: String::new(),
            port: 22,
            username: String::new(),
            auth_type: "password".to_string(),
            password: None,
            private_key_path: None,
            private_key_content: None,
            private_key_passphrase: None,
            target_host: None,
            target_port: None,
            timeout: None,
        });

        let snapshot = snapshot_for(&source);
        assert_eq!(snapshot.fields.get("ssh_tunnel_enabled").unwrap(), "true");
        assert_eq!(snapshot.fields.get("ssh_connection_id").unwrap(), "42");

        let stored = RocketmqFormAdapter
            .build_connection(&snapshot, "隧道".to_string(), None)
            .expect("构建应成功");
        let rebuilt = stored.to_rocketmq_params().unwrap();

        let tunnel = rebuilt.ssh_tunnel.expect("隧道应保留");
        assert!(tunnel.enabled);
        assert_eq!(tunnel.connection_id, Some(42));
    }

    #[test]
    fn build_requires_namesrv_and_applies_defaults() {
        let mut snapshot = FormSnapshot::default();
        // 地址缺失时报错
        let error = RocketmqFormAdapter
            .build_connection(&snapshot, "x".to_string(), None)
            .unwrap_err();
        assert!(!error.is_empty());

        snapshot
            .fields
            .insert("namesrv_addrs".to_string(), "127.0.0.1:9876".to_string());
        let stored = RocketmqFormAdapter
            .build_connection(&snapshot, "x".to_string(), None)
            .unwrap();
        let rebuilt = stored.to_rocketmq_params().unwrap();

        // 数值字段缺失时保持 None(由运行时取默认值)
        assert_eq!(rebuilt.namesrv_addrs, vec!["127.0.0.1:9876".to_string()]);
        assert_eq!(rebuilt.connect_timeout, None);
        assert_eq!(rebuilt.request_timeout, None);
        // 默认名称回退首个 NameServer 地址
        assert_eq!(
            RocketmqFormAdapter.default_name(&snapshot),
            "127.0.0.1:9876"
        );
    }

    #[test]
    fn credential_reference_suppresses_manual_credentials() {
        let mut snapshot = snapshot_for(&params());
        snapshot
            .fields
            .insert("access_key".to_string(), "手动输入".to_string());
        snapshot.credential_reference = Some(one_core::storage::CredentialReference::new(100));

        let stored = RocketmqFormAdapter
            .build_connection(&snapshot, "凭据".to_string(), None)
            .unwrap();
        let rebuilt = stored.to_rocketmq_params().unwrap();

        assert_eq!(rebuilt.access_key, None);
        assert_eq!(rebuilt.secret_key, None);
        assert!(rebuilt.credential_reference.is_some());
    }
}
