use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShellSurface {
    /// 从扩展管理页或 shell 入口打开的普通 tab 视图。
    #[default]
    Tab,
    /// 工具箱页面聚合的小工具卡片；与连接扩展(contributes.connections)
    /// 区分：无连接表单、无 shellViewId 关联、面向单机小工具。
    Toolbox,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ShellHostModule {
    Context,
    Resource,
    Job,
    Event,
    Blob,
    Log,
    Runtime,
    /// 开发者工具专用（navop.dev host 模块）。
    Dev,
}

impl ShellHostModule {
    pub fn requires_backend(self) -> bool {
        matches!(self, Self::Resource | Self::Job | Self::Event | Self::Blob)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ShellViewContrib {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    pub entry: String,
    #[serde(default)]
    pub surface: ShellSurface,
    #[serde(default)]
    pub singleton: bool,
    #[serde(default)]
    pub backends: BTreeMap<String, String>,
    #[serde(default)]
    pub modules: Vec<ShellHostModule>,
    /// toolbox surface 专用：卡片所属分类（如 `text`、`network`、`system`）。
    /// `tab` surface 忽略该字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// toolbox surface 专用：搜索关键词，补充 title/description 匹配。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
}
