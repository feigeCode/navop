//! Shared operation catalog for resource workbenches.
//!
//! GUI 页面、shell workbench 与未来的 MCP/自动化入口都从这份目录推导操作
//! 描述,避免各层各自解释 manifest。目录只做描述与暴露策略;真正的执行仍
//! 经由 `extension-plugin-adapter` 的统一 dispatch。

use serde_json::Value;

use crate::extension::manifest::{
    ResourceWorkbenchBindingSource, ResourceWorkbenchEffect, ResourceWorkbenchOperationMode,
};

use super::resource_workbench::RegisteredResourceWorkbenchContribution;

/// 一次 workbench 操作的自动化描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchOperationEntry {
    pub operation_id: String,
    pub method: String,
    pub mode: ResourceWorkbenchOperationMode,
    pub effect: ResourceWorkbenchEffect,
    /// 执行前必须满足的 resource capability。
    pub requires: Vec<String>,
    /// 参数声明(名称 → 绑定来源)。
    pub params: Vec<WorkbenchOperationParam>,
}

/// 参数声明:输入来源 + 值类型,供自动化入口推导输入 schema。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchOperationParam {
    pub name: String,
    pub source: ResourceWorkbenchBindingSource,
    /// `%`-style manifest 的 value 字符串,仅 literal 常用。
    pub value: Option<Value>,
    pub value_type: &'static str,
}

/// 单个 workbench 的操作目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchOperationCatalog {
    pub extension_id: String,
    pub workbench_id: String,
    pub resource_type: String,
    pub operations: Vec<WorkbenchOperationEntry>,
}

impl WorkbenchOperationCatalog {
    /// 从已注册的 workbench 派生目录。
    pub fn from_contribution(contribution: &RegisteredResourceWorkbenchContribution) -> Self {
        let mut operations: Vec<WorkbenchOperationEntry> = contribution
            .operations
            .iter()
            .map(|(operation_id, operation)| WorkbenchOperationEntry {
                operation_id: operation_id.clone(),
                method: operation.method.clone(),
                mode: operation.mode,
                effect: operation.effect,
                requires: operation.requires.clone(),
                params: operation
                    .params
                    .iter()
                    .map(|(name, binding)| WorkbenchOperationParam {
                        name: name.clone(),
                        source: binding.source,
                        value: binding.value.clone(),
                        value_type: match binding.value_type {
                            crate::extension::manifest::ResourceWorkbenchValueType::String => {
                                "string"
                            }
                            crate::extension::manifest::ResourceWorkbenchValueType::Number => {
                                "number"
                            }
                            crate::extension::manifest::ResourceWorkbenchValueType::Boolean => {
                                "boolean"
                            }
                            crate::extension::manifest::ResourceWorkbenchValueType::Json => "json",
                        },
                    })
                    .collect(),
            })
            .collect();
        operations.sort_by(|a, b| a.operation_id.cmp(&b.operation_id));
        Self {
            extension_id: contribution.extension_id.clone(),
            workbench_id: contribution.id.clone(),
            resource_type: contribution.resource_type.clone(),
            operations,
        }
    }

    /// 默认自动化暴露策略:只允许 read 级操作,免确认。
    ///
    /// 非 read 操作故意不在 MCP 等自动化通道默认暴露;后续如需放通,
    /// 应走宿主审批流程并显式确认。
    pub fn automation_entries(&self) -> Vec<&WorkbenchOperationEntry> {
        self.operations
            .iter()
            .filter(|entry| entry.effect == ResourceWorkbenchEffect::Read)
            .collect()
    }

    pub fn entry(&self, operation_id: &str) -> Option<&WorkbenchOperationEntry> {
        self.operations
            .iter()
            .find(|entry| entry.operation_id == operation_id)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::extension::manifest::{
        ResourceWorkbenchBinding, ResourceWorkbenchOperation, ResourceWorkbenchValueType,
    };

    fn contribution() -> RegisteredResourceWorkbenchContribution {
        let mut operations = BTreeMap::new();
        operations.insert(
            "listItems".to_string(),
            ResourceWorkbenchOperation {
                mode: ResourceWorkbenchOperationMode::Invoke,
                method: "example/list".into(),
                requires: vec![],
                effect: ResourceWorkbenchEffect::Read,
                params: BTreeMap::new(),
            },
        );
        operations.insert(
            "deleteItem".to_string(),
            ResourceWorkbenchOperation {
                mode: ResourceWorkbenchOperationMode::Invoke,
                method: "example/delete".into(),
                requires: vec!["example/delete".into()],
                effect: ResourceWorkbenchEffect::Destructive,
                params: [(
                    "id".to_string(),
                    ResourceWorkbenchBinding {
                        source: ResourceWorkbenchBindingSource::Selection,
                        path: "/id".into(),
                        value_type: ResourceWorkbenchValueType::String,
                        value: None,
                    },
                )]
                .into_iter()
                .collect(),
            },
        );
        RegisteredResourceWorkbenchContribution {
            extension_id: "com.example".into(),
            id: "example.workbench".into(),
            title: "Example".into(),
            connection_ids: vec![],
            runtime_id: "runtime".into(),
            resource_type: "example".into(),
            default_page: "items".into(),
            operations,
            navigation: vec![],
            tree: vec![],
            pages: vec![],
            status_bar: None,
        }
    }

    #[test]
    fn catalog_derives_sorted_entries_from_contribution() {
        let catalog = WorkbenchOperationCatalog::from_contribution(&contribution());

        assert_eq!("com.example", catalog.extension_id);
        assert_eq!("example", catalog.resource_type);
        assert_eq!(2, catalog.operations.len());
        assert_eq!("deleteItem", catalog.operations[0].operation_id);
        assert_eq!("listItems", catalog.operations[1].operation_id);
    }

    #[test]
    fn automation_entries_expose_only_read_operations() {
        let catalog = WorkbenchOperationCatalog::from_contribution(&contribution());
        let exposed = catalog.automation_entries();

        assert_eq!(1, exposed.len());
        assert_eq!("listItems", exposed[0].operation_id);
        assert_eq!(ResourceWorkbenchEffect::Read, exposed[0].effect);
    }

    #[test]
    fn entries_capture_method_capability_and_param_bindings() {
        let catalog = WorkbenchOperationCatalog::from_contribution(&contribution());
        let delete = catalog.entry("deleteItem").unwrap();

        assert_eq!("example/delete", delete.method);
        assert_eq!(vec!["example/delete".to_string()], delete.requires);
        assert_eq!(1, delete.params.len());
        assert_eq!("id", delete.params[0].name);
        assert_eq!(
            ResourceWorkbenchBindingSource::Selection,
            delete.params[0].source
        );
        assert_eq!("string", delete.params[0].value_type);
    }
}
