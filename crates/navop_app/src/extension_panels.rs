//! 扩展面板在两个侧栏之间共享的宿主逻辑。
//!
//! - 清单 `panels[].placement` → `SidebarPlacement`（left/right/bottom）；
//! - 侧栏「+」按钮 → 扩展市场的复合扩展列表。

use std::sync::Arc;

use extension_runtime::MainExtensionViewHost;
use extension_view::ExtensionManagerView;
use gpui::{App, AppContext, Entity, Window};
use one_core::tab_container::{GlobalTabContainer, TabContainer, TabItem};

/// 清单声明的位置 → 宿主侧栏停靠位置。
///
/// 只有启用了对应侧栏的构建才会用到它（终端侧栏 / 数据库侧栏），因此按
/// 同一组 feature 门控，避免默认构建里出现 dead_code 告警。
#[cfg(any(
    all(feature = "nsh-db", feature = "database"),
    all(feature = "nsh-terminal", feature = "shell")
))]
pub(crate) fn sidebar_placement(
    placement: extension_runtime::extension::manifest::PanelPlacement,
) -> one_core::sidebar_contribution::SidebarPlacement {
    use extension_runtime::extension::manifest::PanelPlacement;
    use one_core::sidebar_contribution::SidebarPlacement;

    match placement {
        PanelPlacement::Left => SidebarPlacement::Left,
        PanelPlacement::Right => SidebarPlacement::Right,
        PanelPlacement::Bottom => SidebarPlacement::Bottom,
    }
}

/// 注册侧栏「+」按钮的打开动作（app init 时调用一次）。
pub(crate) fn init(cx: &mut App) {
    one_core::extension_marketplace::init(open_composite_marketplace, cx);
}

fn open_composite_marketplace(window: &mut Window, cx: &mut App) {
    let Some(tab_container) = cx
        .try_global::<GlobalTabContainer>()
        .map(|global| global.tab_container.clone())
    else {
        return;
    };
    // 与插件更新入口同形：页签切换可能触发旧 view 的生命周期回调，因此
    // 建/激活页签与设置过滤都延后到当前更新结束之后。
    window.defer(cx, move |window, cx| {
        tab_container.update(cx, |tc, cx| {
            tc.activate_or_add_tab_lazy(
                "extensions-marketplace",
                |win, cx| {
                    let host = Arc::new(MainExtensionViewHost);
                    let view = cx
                        .new(|cx| ExtensionManagerView::new_marketplace_search(host, "", win, cx));
                    TabItem::new("extensions-marketplace", "home", view)
                },
                window,
                cx,
            );
            if let Some(view) = extension_manager_view(tc) {
                view.update(cx, |view, cx| view.show_composite_marketplace(window, cx));
            }
        });
    });
}

fn extension_manager_view(tc: &mut TabContainer) -> Option<Entity<ExtensionManagerView>> {
    let tab = tc
        .tabs()
        .iter()
        .find(|tab| tab.id() == "extensions-marketplace")?;
    tab.content().view().downcast::<ExtensionManagerView>().ok()
}

#[cfg(test)]
mod tests {
    #[cfg(any(
        all(feature = "nsh-db", feature = "database"),
        all(feature = "nsh-terminal", feature = "shell")
    ))]
    #[test]
    fn manifest_placements_map_to_all_three_sidebar_edges() {
        use extension_runtime::extension::manifest::PanelPlacement;
        use one_core::sidebar_contribution::SidebarPlacement;

        use super::sidebar_placement;

        assert_eq!(
            SidebarPlacement::Left,
            sidebar_placement(PanelPlacement::Left)
        );
        assert_eq!(
            SidebarPlacement::Right,
            sidebar_placement(PanelPlacement::Right)
        );
        assert_eq!(
            SidebarPlacement::Bottom,
            sidebar_placement(PanelPlacement::Bottom)
        );
    }
}
