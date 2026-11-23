//! 终端侧边栏的扩展面板状态。
//!
//! [`ExtensionPanelEntry`] 是内建 [`SidebarPanel`](super::SidebarPanel) 之外的
//! 动态面板条目：id/title/icon 由宿主（navop_app）注入，视图在首次打开时
//! 经宿主创建并缓存。open/placement 语义与内建 dock 状态一致，但键是
//! `SharedString`（不能并入 `Copy` 的 `SidebarPanel` 枚举），因此独立记账。

use gpui::{AnyView, SharedString};
use gpui_component::Icon;
use one_assets::IconName;
use one_core::sidebar_contribution::SidebarPlacement;

use super::extension_panels::ExtensionPanelSpec;

/// 扩展图标：资产文件存在才生效，否则退占位图标。
fn icon_from_path(path: Option<&std::path::Path>) -> Icon {
    match path {
        Some(path) if path.is_file() => {
            let asset = one_core::storage::connection_icon::local_icon_asset_path(path);
            Icon::default().path(asset).color()
        }
        _ => Icon::new(IconName::ExtensionsLine),
    }
}

/// 把全局 specs 同步进现有条目列表：保留仍存在面板的 view/open/placement
/// 状态，新增的按 spec 建 entry，消失的丢弃。返回新列表（按 order、id 排序）。
pub(crate) fn sync_extension_panels(
    current: Vec<ExtensionPanelEntry>,
    specs: Vec<ExtensionPanelSpec>,
) -> Vec<ExtensionPanelEntry> {
    let mut specs = specs;
    specs.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
    specs
        .into_iter()
        .map(|spec| {
            if let Some(existing) = current.iter().find(|entry| entry.id.as_ref() == spec.id) {
                let mut kept = existing.clone();
                kept.title = spec.title;
                kept.icon = icon_from_path(spec.icon_path.as_deref());
                kept.order = spec.order;
                kept
            } else {
                ExtensionPanelEntry::from_spec(spec)
            }
        })
        .collect()
}

/// 关闭与 `placement` 同一边的其它扩展面板，`except` 指定的面板保持不动。
///
/// 终端侧边栏（内建与扩展共用）遵循「每一边只展示一个面板」的语义：
/// 内建面板由 [`TerminalToolDockState`](super::TerminalToolDockState) 记账，
/// 扩展面板也必须按同样的规则互斥，否则同一停靠边会同时存在多个
/// `open = true` 的条目，而渲染层每边只取一个，界面上的按钮高亮与
/// 实际展示内容就会脱节，点击其它扩展看起来「无法切换」。
///
/// 返回状态是否发生变化。
pub(crate) fn close_open_panels_at_placement(
    entries: &mut [ExtensionPanelEntry],
    placement: SidebarPlacement,
    except: Option<&str>,
) -> bool {
    let mut changed = false;
    for entry in entries {
        if entry.open && entry.placement == placement && except != Some(entry.id.as_ref()) {
            entry.open = false;
            changed = true;
        }
    }
    changed
}

/// 一个扩展面板的运行时条目。
#[derive(Clone)]
pub(crate) struct ExtensionPanelEntry {
    /// 面板全局唯一键（`"{extension_id}:{panel_id}"`）。
    pub(crate) id: SharedString,
    /// 面板标题（工具栏 tooltip 与面板标题）。
    pub(crate) title: SharedString,
    /// 面板图标（宿主已解析成 Icon；未提供时用占位图标）。
    pub(crate) icon: Icon,
    /// 排序权重（越小越靠前）。
    pub(crate) order: i64,
    /// 面板视图（首次打开时创建并缓存）。
    pub(crate) view: Option<AnyView>,
    /// 是否打开。
    pub(crate) open: bool,
    /// 停靠位置（默认右侧）。
    pub(crate) placement: SidebarPlacement,
}

impl ExtensionPanelEntry {
    pub(crate) fn from_spec(spec: ExtensionPanelSpec) -> Self {
        Self {
            id: SharedString::new(spec.id),
            title: spec.title,
            icon: icon_from_path(spec.icon_path.as_deref()),
            order: spec.order,
            view: None,
            open: false,
            placement: SidebarPlacement::Right,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, order: i64) -> ExtensionPanelSpec {
        ExtensionPanelSpec {
            id: id.to_string(),
            title: SharedString::new(format!("面板 {id}")),
            icon_path: None,
            order,
        }
    }

    fn entry_with_state(id: &str, open: bool, placement: SidebarPlacement) -> ExtensionPanelEntry {
        let mut entry = ExtensionPanelEntry::from_spec(spec(id, 0));
        entry.open = open;
        entry.placement = placement;
        entry
    }

    #[test]
    fn sync_sorts_specs_by_order_then_id() {
        let merged = sync_extension_panels(
            Vec::new(),
            vec![spec("b:two", 10), spec("a:one", 10), spec("c:zero", -5)],
        );
        let ids: Vec<&str> = merged.iter().map(|e| e.id.as_ref()).collect();
        assert_eq!(ids, vec!["c:zero", "a:one", "b:two"]);
    }

    #[test]
    fn sync_keeps_state_of_existing_panels() {
        let current = vec![entry_with_state("ext.a:p", true, SidebarPlacement::Bottom)];
        let merged = sync_extension_panels(current, vec![spec("ext.a:p", 3)]);
        assert_eq!(merged.len(), 1);
        assert!(merged[0].open);
        assert_eq!(merged[0].placement, SidebarPlacement::Bottom);
        assert_eq!(merged[0].order, 3);
    }

    #[test]
    fn close_open_panels_at_placement_respects_edge_and_except() {
        let mut entries = vec![
            entry_with_state("ext.a:p", true, SidebarPlacement::Right),
            entry_with_state("ext.b:q", true, SidebarPlacement::Right),
            entry_with_state("ext.c:r", true, SidebarPlacement::Bottom),
        ];

        assert!(close_open_panels_at_placement(
            &mut entries,
            SidebarPlacement::Right,
            Some("ext.b:q"),
        ));

        assert!(!entries[0].open, "同边的其它扩展面板应关闭");
        assert!(entries[1].open, "except 指定的面板保持打开");
        assert!(entries[2].open, "其它停靠边不受影响");
    }

    #[test]
    fn close_open_panels_at_placement_reports_no_change_when_already_exclusive() {
        let mut entries = vec![
            entry_with_state("ext.a:p", true, SidebarPlacement::Right),
            entry_with_state("ext.b:q", false, SidebarPlacement::Right),
        ];

        assert!(!close_open_panels_at_placement(
            &mut entries,
            SidebarPlacement::Right,
            Some("ext.a:p"),
        ));
    }

    #[test]
    fn sync_drops_disappeared_panels() {
        let current = vec![entry_with_state("ext.a:p", true, SidebarPlacement::Right)];
        let merged = sync_extension_panels(current, vec![spec("ext.b:q", 0)]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id.as_ref(), "ext.b:q");
        assert!(!merged[0].open);
    }

    #[test]
    fn sync_updates_title_of_existing_panel() {
        let current = vec![ExtensionPanelEntry::from_spec(ExtensionPanelSpec {
            id: "ext.a:p".to_string(),
            title: SharedString::new("旧标题"),
            icon_path: None,
            order: 0,
        })];
        let merged = sync_extension_panels(current, vec![spec("ext.a:p", 0)]);
        assert_eq!(merged[0].title.as_ref(), "面板 ext.a:p");
    }
}
