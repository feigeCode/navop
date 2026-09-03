//! MQTT 连接表单:复用 connection_form 的通用中间件声明式表单引擎
//!
//! 结构与 TDengine 等数据库表单一致:常规/MQTT/高级/SSL/SSH/备注 标签页,
//! 钥匙串/工作区/团队/云同步由引擎统一渲染。本文件只提供:
//! - `mqtt_form_tab_groups()`:MQTT 的声明式标签页配置
//! - `MqttFormAdapter`:`MqttParams` 与表单快照的双向映射 + 测试连接
//! - `MqttFormConfig`/`MqttFormWindow`:对通用窗口的薄封装

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
    ConnectionType, MqttParams, MqttSshTunnelConfig, MqttVersion, StoredConnection, Workspace,
};
use rust_i18n::t;

use crate::manager::{GlobalMqttState, MqttManager};

/// MQTT 表单窗口(通用中间件窗口的类型别名)
pub type MqttFormWindow = MiddlewareFormWindow;

/// 保存成功回调(与通用中间件表单一致)
pub type MqttFormSavedCallback = MiddlewareFormSavedCallback;

/// MQTT 表单配置
pub struct MqttFormConfig {
    /// 正在编辑的连接(`None` 表示新建)
    pub editing_connection: Option<StoredConnection>,
    /// 预填连接(不进入编辑模式)
    pub initial_connection: Option<StoredConnection>,
    /// 保存成功回调
    pub on_saved: Option<MqttFormSavedCallback>,
    /// 可选工作区列表
    pub workspaces: Vec<Workspace>,
    /// 可选团队列表
    pub teams: Vec<TeamOption>,
    /// 可选 SSH 连接(用于隧道引用下拉)
    pub ssh_connections: Vec<StoredConnection>,
}

impl MqttFormConfig {
    pub fn is_editing(&self) -> bool {
        self.editing_connection.is_some()
    }

    /// 转换为通用中间件表单窗口配置(注入 MQTT 适配器与标签页)
    pub fn into_window_config(self) -> MiddlewareFormWindowConfig {
        MiddlewareFormWindowConfig {
            adapter: Arc::new(MqttFormAdapter),
            tab_groups: mqtt_form_tab_groups(),
            editing_connection: self.editing_connection,
            initial_connection: self.initial_connection,
            on_saved: self.on_saved,
            workspaces: self.workspaces,
            teams: self.teams,
            ssh_connections: self.ssh_connections,
        }
    }
}

/// MQTT 声明式标签页配置
///
/// 常规 / MQTT(中间件特性扩展) / 高级 / SSL / SSH / 备注,
/// SSH 与备注页复用引擎提供的共享构造器。
pub fn mqtt_form_tab_groups() -> Vec<TabGroup> {
    vec![
        TabGroup::new("general", t!("MqttForm.tab_general").to_string()).fields(vec![
            FormField::new("name", t!("MqttForm.name"), FormFieldType::Text)
                .placeholder(t!("MqttForm.name_placeholder"))
                .default("Local MQTT"),
            FormField::new("host", t!("MqttForm.host"), FormFieldType::Text)
                .placeholder("127.0.0.1")
                .default("127.0.0.1"),
            FormField::new("port", t!("MqttForm.port"), FormFieldType::Number)
                .placeholder("1883")
                .default("1883"),
            FormField::new("username", t!("MqttForm.username"), FormFieldType::Text)
                .optional()
                .placeholder(t!("MqttForm.username_placeholder")),
            FormField::new("password", t!("MqttForm.password"), FormFieldType::Password)
                .optional()
                .placeholder(t!("MqttForm.password_placeholder")),
        ]),
        TabGroup::new("mqtt", t!("MqttForm.tab_mqtt").to_string()).fields(vec![
            FormField::new("client_id", t!("MqttForm.client_id"), FormFieldType::Text)
                .optional()
                .placeholder(t!("MqttForm.client_id_placeholder")),
            FormField::new(
                "keep_alive",
                t!("MqttForm.keep_alive"),
                FormFieldType::Number,
            )
            .optional()
            .placeholder("30")
            .default("30"),
            FormField::new(
                "clean_session",
                t!("MqttForm.clean_session"),
                FormFieldType::Select,
            )
            .optional()
            .default("true")
            .options(vec![
                ("true".to_string(), t!("Common.yes").to_string()),
                ("false".to_string(), t!("Common.no").to_string()),
            ]),
        ]),
        TabGroup::new("advanced", t!("MqttForm.tab_advanced").to_string()).fields(vec![
            FormField::new(
                "connect_timeout",
                t!("MqttForm.connect_timeout"),
                FormFieldType::Number,
            )
            .optional()
            .placeholder("10")
            .default("10"),
        ]),
        TabGroup::new("ssl", t!("MqttForm.tab_ssl").to_string()).fields(vec![
            FormField::new("use_tls", t!("MqttForm.use_tls"), FormFieldType::Select)
                .optional()
                .default("false")
                .options(vec![
                    ("false".to_string(), t!("Common.no").to_string()),
                    ("true".to_string(), t!("Common.yes").to_string()),
                ]),
        ]),
        ssh_tab_group(),
        notes_tab_group(),
    ]
}

/// MQTT 表单适配器
pub struct MqttFormAdapter;

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

impl MiddlewareFormAdapter for MqttFormAdapter {
    fn connection_type(&self) -> ConnectionType {
        ConnectionType::Mqtt
    }

    fn load_fields(&self, connection: &StoredConnection) -> Result<FormSnapshot, String> {
        let params = connection
            .to_mqtt_params()
            .map_err(|error| error.to_string())?;

        let mut fields = HashMap::new();
        fields.insert("host".to_string(), params.host.clone());
        fields.insert("port".to_string(), params.port.to_string());
        fields.insert("client_id".to_string(), params.client_id.clone());
        fields.insert(
            "username".to_string(),
            params.username.clone().unwrap_or_default(),
        );
        fields.insert(
            "password".to_string(),
            params.password.clone().unwrap_or_default(),
        );
        fields.insert(
            "use_tls".to_string(),
            if params.use_tls { "true" } else { "false" }.to_string(),
        );
        fields.insert(
            "keep_alive".to_string(),
            params
                .keep_alive
                .map(|value| value.to_string())
                .unwrap_or_default(),
        );
        fields.insert(
            "clean_session".to_string(),
            if params.clean_session {
                "true"
            } else {
                "false"
            }
            .to_string(),
        );
        fields.insert(
            "connect_timeout".to_string(),
            params
                .connect_timeout
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

        // 协议版本经透传字段保留(rumqttc 仅支持 3.1.1,UI 暂不暴露选择)
        let mut extras = HashMap::new();
        extras.insert(
            "mqtt_version".to_string(),
            params.mqtt_version.as_str().to_string(),
        );

        Ok(FormSnapshot {
            fields,
            extras,
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
        let host = fields
            .get("host")
            .map(|value| value.trim().to_string())
            .unwrap_or_default();
        if host.is_empty() {
            return Err(t!("MqttForm.host_required").to_string());
        }
        let port: u16 = number_field(fields, "port").unwrap_or(1883);

        // 选择钥匙串引用时引擎会省略手动账号密码字段
        let (username, password) = if snapshot.credential_reference.is_some() {
            (None, None)
        } else {
            (
                optional_input(fields.get("username")),
                optional_input(fields.get("password")),
            )
        };

        let mqtt_version = match snapshot.extras.get("mqtt_version").map(String::as_str) {
            Some("5.0") => MqttVersion::V5,
            _ => MqttVersion::V311,
        };

        let ssh_tunnel = if bool_field(fields, "ssh_tunnel_enabled") {
            Some(MqttSshTunnelConfig {
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

        let params = MqttParams {
            host,
            port,
            client_id: fields.get("client_id").cloned().unwrap_or_default(),
            username,
            password,
            credential_reference: snapshot.credential_reference.clone(),
            use_tls: bool_field(fields, "use_tls"),
            connect_timeout: number_field(fields, "connect_timeout"),
            keep_alive: number_field(fields, "keep_alive"),
            mqtt_version,
            // 未显式选择"否"时默认清除会话
            clean_session: fields.get("clean_session").map(String::as_str) != Some("false"),
            ssh_tunnel,
        };

        Ok(StoredConnection::new_mqtt(name, params, workspace_id))
    }

    fn default_name(&self, snapshot: &FormSnapshot) -> String {
        let host = snapshot
            .fields
            .get("host")
            .map(String::as_str)
            .unwrap_or_default();
        let port = snapshot
            .fields
            .get("port")
            .map(String::as_str)
            .unwrap_or("1883");
        format!("{host}:{port}")
    }

    fn test_connection(
        &self,
        connection: &StoredConnection,
        cx: &mut App,
    ) -> Task<Result<(), String>> {
        let config = resolve_connection_for_runtime(connection.clone(), cx).and_then(|resolved| {
            MqttManager::config_from_stored(&resolved).map_err(|error| error.to_string())
        });
        let config = match config {
            Ok(config) => config,
            Err(error) => {
                return cx.spawn(async move |_cx: &mut AsyncApp| Err::<(), String>(error));
            }
        };

        let Some(global_state) = cx.try_global::<GlobalMqttState>().cloned() else {
            return cx.spawn(async move |_cx: &mut AsyncApp| {
                Err::<(), String>(t!("MqttForm.state_missing").to_string())
            });
        };

        cx.spawn(async move |cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, async move {
                global_state
                    .test_connection(&config)
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
    use connection_form::middleware_form::FormVisibilityRule;
    use one_core::storage::MqttParams;

    fn snapshot_for(params: &MqttParams) -> FormSnapshot {
        let mut stored = StoredConnection::new_mqtt("测试".to_string(), params.clone(), None);
        stored.id = Some(7);
        MqttFormAdapter.load_fields(&stored).expect("回填应成功")
    }

    fn params() -> MqttParams {
        MqttParams {
            host: "broker.example.com".to_string(),
            port: 8883,
            client_id: "navop-client".to_string(),
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
            credential_reference: None,
            use_tls: true,
            connect_timeout: Some(5),
            keep_alive: Some(60),
            mqtt_version: MqttVersion::V311,
            clean_session: false,
            ssh_tunnel: None,
        }
    }

    #[test]
    fn tab_groups_declare_expected_tabs_and_fields() {
        let groups = mqtt_form_tab_groups();
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();

        assert_eq!(
            names,
            vec!["general", "mqtt", "advanced", "ssl", "ssh", "notes"]
        );
        assert!(groups[0].fields.iter().any(|f| f.name == "host"));
        assert!(groups[1].fields.iter().any(|f| f.name == "client_id"));
        assert!(groups[1].fields.iter().any(|f| f.name == "clean_session"));
        assert!(groups[2].fields.iter().any(|f| f.name == "connect_timeout"));
        assert!(groups[3].fields.iter().any(|f| f.name == "use_tls"));
        assert!(
            groups[4]
                .fields
                .iter()
                .any(|f| f.name == "ssh_tunnel_enabled")
        );
        assert!(groups[5].fields.iter().any(|f| f.name == "remark"));
    }

    #[test]
    fn load_and_build_round_trip_all_fields() {
        let snapshot = snapshot_for(&params());

        assert_eq!(snapshot.fields.get("host").unwrap(), "broker.example.com");
        assert_eq!(snapshot.fields.get("port").unwrap(), "8883");
        assert_eq!(snapshot.fields.get("use_tls").unwrap(), "true");
        assert_eq!(snapshot.fields.get("clean_session").unwrap(), "false");
        assert_eq!(snapshot.fields.get("keep_alive").unwrap(), "60");
        assert_eq!(snapshot.fields.get("connect_timeout").unwrap(), "5");
        assert_eq!(snapshot.extras.get("mqtt_version").unwrap(), "3.1.1");

        let stored = MqttFormAdapter
            .build_connection(&snapshot, "回环".to_string(), Some(3))
            .expect("构建应成功");
        let rebuilt = stored.to_mqtt_params().expect("参数应可解析");

        assert_eq!(rebuilt.host, params().host);
        assert_eq!(rebuilt.port, 8883);
        assert_eq!(rebuilt.client_id, "navop-client");
        assert_eq!(rebuilt.username.as_deref(), Some("user"));
        assert_eq!(rebuilt.password.as_deref(), Some("pass"));
        assert!(rebuilt.use_tls);
        assert_eq!(rebuilt.connect_timeout, Some(5));
        assert_eq!(rebuilt.keep_alive, Some(60));
        assert!(!rebuilt.clean_session);
        assert_eq!(rebuilt.mqtt_version, MqttVersion::V311);
        assert_eq!(stored.workspace_id, Some(3));
        assert_eq!(stored.connection_type, ConnectionType::Mqtt);
    }

    #[test]
    fn round_trip_preserves_ssh_tunnel_and_version() {
        let mut source = params();
        source.ssh_tunnel = Some(MqttSshTunnelConfig {
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
        source.mqtt_version = MqttVersion::V5;

        let snapshot = snapshot_for(&source);
        assert_eq!(snapshot.fields.get("ssh_tunnel_enabled").unwrap(), "true");
        assert_eq!(snapshot.fields.get("ssh_connection_id").unwrap(), "42");
        assert_eq!(snapshot.extras.get("mqtt_version").unwrap(), "5.0");

        let stored = MqttFormAdapter
            .build_connection(&snapshot, "隧道".to_string(), None)
            .expect("构建应成功");
        let rebuilt = stored.to_mqtt_params().unwrap();

        let tunnel = rebuilt.ssh_tunnel.expect("隧道应保留");
        assert!(tunnel.enabled);
        assert_eq!(tunnel.connection_id, Some(42));
        assert_eq!(rebuilt.mqtt_version, MqttVersion::V5);
    }

    #[test]
    fn build_requires_host_and_applies_defaults() {
        let mut snapshot = FormSnapshot::default();
        // host 缺失时报错
        let error = MqttFormAdapter
            .build_connection(&snapshot, "x".to_string(), None)
            .unwrap_err();
        assert!(!error.is_empty());

        snapshot
            .fields
            .insert("host".to_string(), "127.0.0.1".into());
        let stored = MqttFormAdapter
            .build_connection(&snapshot, "x".to_string(), None)
            .unwrap();
        let params = stored.to_mqtt_params().unwrap();

        // 数值字段缺失时回退默认:端口 1883、清除会话开启
        assert_eq!(params.port, 1883);
        assert!(params.clean_session);
        assert_eq!(params.keep_alive, None);
        // 默认名称回退 host:port
        assert_eq!(MqttFormAdapter.default_name(&snapshot), "127.0.0.1:1883");
    }

    #[test]
    fn credential_reference_suppresses_manual_credentials() {
        let mut snapshot = snapshot_for(&params());
        snapshot
            .fields
            .insert("username".to_string(), "手动输入".to_string());
        snapshot.credential_reference = Some(one_core::storage::CredentialReference::new(100));

        let stored = MqttFormAdapter
            .build_connection(&snapshot, "凭据".to_string(), None)
            .unwrap();
        let params = stored.to_mqtt_params().unwrap();

        assert_eq!(params.username, None);
        assert_eq!(params.password, None);
        assert!(params.credential_reference.is_some());
    }

    #[test]
    fn visibility_rule_helper_exists_for_future_conditional_fields() {
        // 引擎支持条件字段(供后续 Kafka SASL 等场景使用)
        let rule = FormVisibilityRule::field_equals("use_tls", "true");
        assert!(rule.matches(Some("true")));
    }
}
