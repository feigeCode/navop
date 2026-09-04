//! 中间件表单适配器协议
//!
//! 引擎只认识"字段名 -> 字符串值"的扁平映射;各中间件的参数结构
//! (如 `MqttParams`,后续 RocketMQ/Kafka 的参数模型)由适配器负责
//! 与 `FormSnapshot` 之间的双向转换。

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, Task};
use one_core::storage::{ConnectionType, CredentialReference, StoredConnection};

/// 表单当前值的快照
#[derive(Clone, Debug, Default)]
pub struct FormSnapshot {
    /// 声明字段的当前值(字段名 -> 值)
    pub fields: HashMap<String, String>,
    /// 透传字段(不在任何标签页中,编辑时保留、保存时原样回传,
    /// 例如 MQTT 的协议版本)
    pub extras: HashMap<String, String>,
    /// 钥匙串选择的凭据引用(`None` 表示手动输入)
    pub credential_reference: Option<CredentialReference>,
}

/// 中间件连接表单适配器
///
/// 由各中间件视图 crate 实现;引擎侧不感知具体参数模型。
pub trait MiddlewareFormAdapter: Send + Sync + 'static {
    /// 连接类型(决定保存后的 `StoredConnection.connection_type`)
    fn connection_type(&self) -> ConnectionType;

    /// 从既有连接提取表单值(编辑/预填回填)
    ///
    /// 返回的 `fields` 中未声明的键会进入 `extras` 并在保存时透传;
    /// `name`/`remark`/工作区/团队/云同步由引擎统一处理,无需返回。
    fn load_fields(&self, connection: &StoredConnection) -> Result<FormSnapshot, String>;

    /// 由表单值构建新的存储连接
    ///
    /// 引擎随后会统一覆盖 workspace/team/sync/remark 与编辑模式的
    /// 云同步元数据(id/cloud_id/last_synced_at/owner_id)。
    fn build_connection(
        &self,
        snapshot: &FormSnapshot,
        name: String,
        workspace_id: Option<i64>,
    ) -> Result<StoredConnection, String>;

    /// 用户未填写连接名称时的默认名称(如 `host:port`)
    fn default_name(&self, snapshot: &FormSnapshot) -> String;

    /// 测试连接:发起一次真实的连接尝试并返回结果
    fn test_connection(
        &self,
        connection: &StoredConnection,
        cx: &mut App,
    ) -> Task<Result<(), String>>;
}

/// 保存成功回调
pub type MiddlewareFormSavedCallback =
    Arc<dyn Fn(StoredConnection, &mut App) + Send + Sync + 'static>;
