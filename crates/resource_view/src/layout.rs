//! layout 声明 → 渲染用区域枚举。
//!
//! `RegisteredResourceWorkbenchContribution.layout` 在工作台创建时一次性
//! 解析为 `ResolvedLayout`;`None` 布局等价默认 `{left: List(全部页面),
//! center: Pages}`。渲染层只消费 Resolved 类型,不再触碰 manifest DTO。

use extension_runtime::extension::manifest::{
    ResourceWorkbenchBottomSource, ResourceWorkbenchCenterSource, ResourceWorkbenchNavSource,
    ResourceWorkbenchSideSource, ResourceWorkbenchTab, ResourceWorkbenchTabGroup,
};

/// 布局区域标识:Shell mount 缓存 key 与 `page_context.regionId` 共用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RegionId {
    Left,
    Center,
    Right,
    Bottom,
    /// root Shell 覆盖(整个工作台主体)。
    Root,
}

impl RegionId {
    pub fn as_str(self) -> &'static str {
        match self {
            RegionId::Left => "left",
            RegionId::Center => "center",
            RegionId::Right => "right",
            RegionId::Bottom => "bottom",
            RegionId::Root => "root",
        }
    }
}

/// 树根解析产物:静态声明 + 子节点拉取协议。
#[derive(Debug, Clone)]
pub struct ResolvedTreeRoot {
    pub id: String,
    pub title: String,
    pub page_id: String,
    pub children: Option<extension_runtime::extension::manifest::ResourceWorkbenchTreeChildren>,
}

/// 导航树子节点数据:一次 lazy 拉取的行数据快照。
#[derive(Debug, Clone)]
pub struct TreeChildRow {
    pub key: String,
    pub label: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum NavContent {
    List { entries: Vec<(String, String)> },
    Tree { roots: Vec<ResolvedTreeRoot> },
    Shell { view_id: String },
    None,
}

#[derive(Debug, Clone)]
pub enum CenterContent {
    Pages {
        tab_groups: Vec<ResourceWorkbenchTabGroup>,
    },
    Shell {
        view_id: String,
    },
}

#[derive(Debug, Clone)]
pub enum BottomContent {
    Status {
        operation: String,
        items: Vec<extension_runtime::extension::manifest::ResourceWorkbenchStatusItem>,
    },
    Shell {
        view_id: String,
    },
    None,
}

#[derive(Debug, Clone)]
pub enum SideContent {
    Shell { view_id: String },
    None,
}

/// 渲染用布局快照。
#[derive(Debug, Clone)]
pub struct ResolvedLayout {
    pub root_shell: Option<String>,
    pub left: Option<NavRegion>,
    pub center: Option<CenterRegion>,
    pub right: Option<SideRegion>,
    pub bottom: Option<BottomRegion>,
}

#[derive(Debug, Clone)]
pub struct NavRegion {
    pub width: u32,
    pub content: NavContent,
}

#[derive(Debug, Clone)]
pub struct CenterRegion {
    pub content: CenterContent,
}

#[derive(Debug, Clone)]
pub struct SideRegion {
    pub width: u32,
    pub content: SideContent,
}

#[derive(Debug, Clone)]
pub struct BottomRegion {
    pub height: u32,
    pub content: BottomContent,
}

pub const DEFAULT_NAV_WIDTH: u32 = 208;
pub const DEFAULT_STATUS_HEIGHT: u32 = 28;

impl ResolvedLayout {
    pub fn resolve(
        descriptor: &extension_runtime::RegisteredResourceWorkbenchContribution,
    ) -> Self {
        let Some(layout) = descriptor.layout.as_ref() else {
            return Self::default_list(descriptor);
        };
        if let Some(root) = root_shell_view(layout) {
            return Self {
                root_shell: Some(root),
                left: None,
                center: None,
                right: None,
                bottom: None,
            };
        }
        Self {
            root_shell: None,
            left: layout.left.as_ref().map(|region| NavRegion {
                width: region.width.unwrap_or(DEFAULT_NAV_WIDTH),
                content: match &region.source {
                    ResourceWorkbenchNavSource::List { items } => NavContent::List {
                        entries: items
                            .iter()
                            .filter_map(|entry| {
                                descriptor
                                    .page(&entry.page_id)
                                    .map(|page| (page.id.clone(), page.title.clone()))
                            })
                            .collect(),
                    },
                    ResourceWorkbenchNavSource::Tree { roots } => NavContent::Tree {
                        roots: roots
                            .iter()
                            .map(|root| ResolvedTreeRoot {
                                id: root.id.clone(),
                                title: root.title.clone(),
                                page_id: root.page_id.clone(),
                                children: root.children.clone(),
                            })
                            .collect(),
                    },
                    ResourceWorkbenchNavSource::Shell(source) => NavContent::Shell {
                        view_id: source.view_id.clone(),
                    },
                    ResourceWorkbenchNavSource::None => NavContent::None,
                },
            }),
            center: layout.center.as_ref().map(|region| CenterRegion {
                content: match &region.source {
                    ResourceWorkbenchCenterSource::Pages { tab_groups } => CenterContent::Pages {
                        tab_groups: tab_groups.clone(),
                    },
                    ResourceWorkbenchCenterSource::Shell(source) => CenterContent::Shell {
                        view_id: source.view_id.clone(),
                    },
                },
            }),
            right: layout.right.as_ref().map(|region| SideRegion {
                width: region.width.unwrap_or(DEFAULT_NAV_WIDTH),
                content: match &region.source {
                    ResourceWorkbenchSideSource::Shell(source) => SideContent::Shell {
                        view_id: source.view_id.clone(),
                    },
                    ResourceWorkbenchSideSource::None => SideContent::None,
                },
            }),
            bottom: layout.bottom.as_ref().map(|region| BottomRegion {
                height: region.height.unwrap_or(DEFAULT_STATUS_HEIGHT),
                content: match &region.source {
                    ResourceWorkbenchBottomSource::Status { operation, items } => {
                        BottomContent::Status {
                            operation: operation.clone(),
                            items: items.clone(),
                        }
                    }
                    ResourceWorkbenchBottomSource::Shell(source) => BottomContent::Shell {
                        view_id: source.view_id.clone(),
                    },
                    ResourceWorkbenchBottomSource::None => BottomContent::None,
                },
            }),
        }
    }

    fn default_list(
        descriptor: &extension_runtime::RegisteredResourceWorkbenchContribution,
    ) -> Self {
        Self {
            root_shell: None,
            left: Some(NavRegion {
                width: DEFAULT_NAV_WIDTH,
                content: NavContent::List {
                    entries: descriptor
                        .pages
                        .iter()
                        .map(|page| (page.id.clone(), page.title.clone()))
                        .collect(),
                },
            }),
            center: Some(CenterRegion {
                content: CenterContent::Pages {
                    tab_groups: Vec::new(),
                },
            }),
            right: None,
            bottom: None,
        }
    }

    /// 当前页面所属 tab 组的 tabs 声明(v2:组只声明一份,page 经
    /// `tabGroupId` 引用)。无 layout 的 center 也命中(空组列表 → None)。
    pub fn tab_group_for(
        &self,
        page: &extension_runtime::extension::manifest::ResourceWorkbenchPage,
    ) -> Option<&[ResourceWorkbenchTab]> {
        let group_id = page.tab_group_id.as_deref()?;
        let center = self.center.as_ref()?;
        match &center.content {
            CenterContent::Pages { tab_groups } => tab_groups
                .iter()
                .find(|group| group.id == group_id)
                .map(|group| group.tabs.as_slice()),
            CenterContent::Shell { .. } => None,
        }
    }
}

fn root_shell_view(
    layout: &extension_runtime::extension::manifest::ResourceWorkbenchLayout,
) -> Option<String> {
    use extension_runtime::extension::manifest::ResourceWorkbenchRendererKind;
    let renderer = layout.renderer.as_ref()?;
    (renderer.kind == ResourceWorkbenchRendererKind::Shell)
        .then(|| renderer.view_id.clone())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension_runtime::RegisteredResourceWorkbenchContribution;
    use extension_runtime::extension::manifest::{
        ResourceWorkbenchContrib, ResourceWorkbenchEffect, ResourceWorkbenchOperation,
        ResourceWorkbenchOperationMode, ResourceWorkbenchPage, ResourceWorkbenchRenderer,
        ResourceWorkbenchRendererKind, ResourceWorkbenchTemplate,
    };
    use std::collections::BTreeMap;

    fn descriptor(
        layout: Option<extension_runtime::extension::manifest::ResourceWorkbenchLayout>,
    ) -> RegisteredResourceWorkbenchContribution {
        let operations = BTreeMap::from([(
            "list".to_string(),
            ResourceWorkbenchOperation {
                mode: ResourceWorkbenchOperationMode::Invoke,
                method: "example/list".into(),
                requires: vec![],
                effect: ResourceWorkbenchEffect::Read,
                params: BTreeMap::new(),
            },
        )]);
        RegisteredResourceWorkbenchContribution::from_manifest(
            "com.example",
            &ResourceWorkbenchContrib {
                schema_version: 2,
                id: "example".into(),
                title: "Example".into(),
                connection_ids: vec![],
                runtime_id: "main".into(),
                resource_type: "example".into(),
                default_page: "overview".into(),
                operations,
                layout,
                pages: vec![ResourceWorkbenchPage {
                    id: "overview".into(),
                    title: "Overview".into(),
                    template: ResourceWorkbenchTemplate::Json,
                    renderer: ResourceWorkbenchRenderer {
                        kind: ResourceWorkbenchRendererKind::Native,
                        view_id: None,
                        fallback: None,
                    },
                    tab_group_id: None,
                    load: None,
                    execute: None,
                    collection: None,
                    inputs: vec![],
                    scope: None,
                    terminal: None,
                    route: None,
                    links: vec![],
                }],
            },
        )
    }

    #[test]
    fn missing_layout_defaults_to_list_left_and_pages_center() {
        let resolved = ResolvedLayout::resolve(&descriptor(None));
        assert!(resolved.root_shell.is_none());
        match resolved.left.expect("default left").content {
            NavContent::List { entries } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].0, "overview");
            }
            other => panic!("expected list nav, got {other:?}"),
        }
        assert!(matches!(
            resolved.center.expect("default center").content,
            CenterContent::Pages { .. }
        ));
        assert!(resolved.right.is_none());
        assert!(resolved.bottom.is_none());
    }

    #[test]
    fn root_shell_layout_resolves_without_regions() {
        let layout = extension_runtime::extension::manifest::ResourceWorkbenchLayout {
            renderer: Some(ResourceWorkbenchRenderer {
                kind: ResourceWorkbenchRendererKind::Shell,
                view_id: Some("es-workspace".into()),
                fallback: None,
            }),
            left: None,
            center: None,
            right: None,
            bottom: None,
        };
        let resolved = ResolvedLayout::resolve(&descriptor(Some(layout)));
        assert_eq!(resolved.root_shell.as_deref(), Some("es-workspace"));
        assert!(resolved.left.is_none());
        assert!(resolved.center.is_none());
    }

    #[test]
    fn tab_group_lookup_matches_page_tab_group_id() {
        use extension_runtime::extension::manifest::{
            ResourceWorkbenchCenterRegion, ResourceWorkbenchCenterSource, ResourceWorkbenchTab,
            ResourceWorkbenchTabGroup,
        };
        let layout = extension_runtime::extension::manifest::ResourceWorkbenchLayout {
            renderer: None,
            left: None,
            center: Some(ResourceWorkbenchCenterRegion {
                source: ResourceWorkbenchCenterSource::Pages {
                    tab_groups: vec![ResourceWorkbenchTabGroup {
                        id: "main".into(),
                        tabs: vec![ResourceWorkbenchTab {
                            id: "overview-tab".into(),
                            title: "Overview".into(),
                            page_id: "overview".into(),
                            route: BTreeMap::new(),
                        }],
                    }],
                },
            }),
            right: None,
            bottom: None,
        };
        let descriptor = descriptor(Some(layout));
        let mut page = descriptor.page("overview").unwrap().clone();
        page.tab_group_id = Some("main".into());
        let resolved = ResolvedLayout::resolve(&descriptor);
        let tabs = resolved.tab_group_for(&page).expect("group must resolve");
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].page_id, "overview");
        // 未声明 tabGroupId 的页面命中 None。
        assert!(
            resolved
                .tab_group_for(&{
                    let mut ungrouped = page.clone();
                    ungrouped.tab_group_id = None;
                    ungrouped
                })
                .is_none()
        );
    }
}
