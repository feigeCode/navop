//! Stateless menu shared by independent connection views.
//!
//! 类型筛选只显示纯文本（无图标、全英文名）；「Extension」不再作为单一入口，
//! 而是按已安装扩展贡献逐项列出显示名，方便按具体扩展筛选连接；扩展显示名与
//! 内置类型同名时（如 MQTT 扩展与内置 MQTT），只保留扩展项，避免重复显示。
//! 侧栏树的筛选菜单与主页标题行的平铺筛选条共用同一份筛选项清单
//! （见 [`filter_targets`]），因此胶囊与菜单条目的顺序、命名完全一致。
use crate::home_tab::ConnectionFilter;
use crate::home_tab::connection_filter::ExtensionFilterTarget;
use extension_runtime::GlobalExtensionRuntimeCatalog;
use gpui::{App, Window};
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use one_core::storage::ConnectionType;
use std::rc::Rc;

/// 从全局扩展目录收集可参与筛选的扩展连接贡献（友好显示名）。
pub(crate) fn extension_filter_targets(cx: &App) -> Vec<ExtensionFilterTarget> {
    let catalog = cx
        .try_global::<GlobalExtensionRuntimeCatalog>()
        .and_then(|catalog| catalog.get());
    let mut targets: Vec<ExtensionFilterTarget> = Vec::new();
    if let Some(catalog) = catalog {
        for contribution in catalog.resource_connections() {
            targets.push(ExtensionFilterTarget {
                extension_id: contribution.extension_id.clone(),
                contribution_id: contribution.id.clone(),
                label: contribution.label.clone(),
            });
        }
    }
    targets
}

/// 内置类型筛选项：若某扩展显示名与内置类型名一致，去掉内置项（扩展项负责命中）。
pub(crate) fn builtin_filter_types(extensions: &[ExtensionFilterTarget]) -> Vec<ConnectionType> {
    ConnectionType::all()
        .into_iter()
        // All 由筛选项清单顶部单独添加，Extension 已按扩展贡献逐项列出，
        // 两者都不应再从 all() 迭代中出现，避免菜单里出现两个 All。
        .filter(|kind| *kind != ConnectionType::Extension && *kind != ConnectionType::All)
        .filter(|kind| {
            !extensions
                .iter()
                .any(|ext| ext.label.eq_ignore_ascii_case(ConnectionType::label(kind)))
        })
        .collect()
}

/// 完整筛选项清单：All + 内置类型 + 扩展贡献（顺序即菜单与筛选条的展示顺序）。
pub(crate) fn filter_targets_for(extensions: &[ExtensionFilterTarget]) -> Vec<ConnectionFilter> {
    let mut targets = Vec::with_capacity(extensions.len() + ConnectionType::all().len() + 1);
    targets.push(ConnectionFilter::All);
    targets.extend(
        builtin_filter_types(extensions)
            .into_iter()
            .map(ConnectionFilter::Builtin),
    );
    targets.extend(extensions.iter().cloned().map(ConnectionFilter::Extension));
    targets
}

/// 从全局扩展目录收集完整筛选项清单（主页标题行筛选条使用）。
pub(crate) fn filter_targets(cx: &App) -> Vec<ConnectionFilter> {
    filter_targets_for(&extension_filter_targets(cx))
}

/// 完整筛选菜单：All + 内置类型 + 扩展贡献（常驻侧栏树的筛选按钮使用）。
pub(crate) fn build_filter_menu(
    menu: PopupMenu,
    selected: &ConnectionFilter,
    extensions: &[ExtensionFilterTarget],
    activate: Rc<dyn Fn(ConnectionFilter, &mut Window, &mut App)>,
) -> PopupMenu {
    build_filter_menu_for(menu, &filter_targets_for(extensions), selected, activate)
}

/// 只为给定筛选项构建菜单；平铺筛选条的「更多」用它收纳放不下的项。
pub(crate) fn build_filter_menu_for(
    menu: PopupMenu,
    items: &[ConnectionFilter],
    selected: &ConnectionFilter,
    activate: Rc<dyn Fn(ConnectionFilter, &mut Window, &mut App)>,
) -> PopupMenu {
    items.iter().cloned().fold(menu, |menu, filter| {
        let activate = activate.clone();
        let checked = *selected == filter;
        menu.item(
            PopupMenuItem::new(filter.label())
                .checked(checked)
                .on_click(move |_, window, cx| {
                    activate(filter.clone(), window, cx);
                }),
        )
    })
}
