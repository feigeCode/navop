use crate::extension::manifest::{
    ResourceWorkbenchContrib, ResourceWorkbenchLayout, ResourceWorkbenchOperation,
    ResourceWorkbenchPage,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredResourceWorkbenchContribution {
    pub extension_id: String,
    pub id: String,
    pub title: String,
    pub connection_ids: Vec<String>,
    pub runtime_id: String,
    pub resource_type: String,
    pub default_page: String,
    pub operations: std::collections::BTreeMap<String, ResourceWorkbenchOperation>,
    pub layout: Option<ResourceWorkbenchLayout>,
    pub pages: Vec<ResourceWorkbenchPage>,
}

impl RegisteredResourceWorkbenchContribution {
    /// 由 manifest 贡献构造(注册期与测试共用)。
    pub fn from_manifest(extension_id: &str, workbench: &ResourceWorkbenchContrib) -> Self {
        Self {
            extension_id: extension_id.to_string(),
            id: workbench.id.clone(),
            title: workbench.title.clone(),
            connection_ids: workbench.connection_ids.clone(),
            runtime_id: workbench.runtime_id.clone(),
            resource_type: workbench.resource_type.clone(),
            default_page: workbench.default_page.clone(),
            operations: workbench.operations.clone(),
            layout: workbench.layout.clone(),
            pages: workbench.pages.clone(),
        }
    }

    /// 按 id 查找页面。
    pub fn page(&self, page_id: &str) -> Option<&ResourceWorkbenchPage> {
        self.pages.iter().find(|page| page.id == page_id)
    }
}
