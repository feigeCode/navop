//! 运行时资源契约：领域把「当前活动连接」交给公共壳聚合。

use gpui::App;

/// 某一类资源的活动连接 id（`kind` 与 `tool_runtime::ResourceKind::as_str` 对齐）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveResourceIds {
    pub kind: &'static str,
    pub connection_ids: Vec<String>,
}

/// 领域实现的只读快照接口。
pub trait ActiveResourceProvider: Send + Sync {
    fn active_resources(&self, cx: &App) -> Vec<ActiveResourceIds>;
}
