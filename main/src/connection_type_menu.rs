//! Stateless menu shared by independent connection views.
//!
//! 类型筛选菜单只显示纯文本（无图标、全英文名）；「Extension」不再作为单一入口，
//! 而是按已安装扩展贡献逐项列出显示名，方便按具体扩展筛选连接；扩展显示名与
//! 内置类型同名时（如 MQTT 扩展与内置 MQTT），只保留扩展项，避免重复显示。
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
        // All 由 build_filter_menu 顶部单独添加，Extension 已按扩展贡献逐项列出，
        // 两者都不应再从 all() 迭代中出现，避免菜单里出现两个 All。
        .filter(|kind| *kind != ConnectionType::Extension && *kind != ConnectionType::All)
        .filter(|kind| {
            !extensions
                .iter()
                .any(|ext| ext.label.eq_ignore_ascii_case(ConnectionType::label(kind)))
        })
        .collect()
}

pub(crate) fn build_filter_menu(
    menu: PopupMenu,
    selected: &ConnectionFilter,
    extensions: &[ExtensionFilterTarget],
    activate: Rc<dyn Fn(ConnectionFilter, &mut Window, &mut App)>,
) -> PopupMenu {
    let all_activate = activate.clone();
    let mut menu = menu.item(
        PopupMenuItem::new(ConnectionType::All.label().to_string())
            .checked(*selected == ConnectionFilter::All)
            .on_click(move |_, window, cx| {
                all_activate(ConnectionFilter::All, window, cx);
            }),
    );

    for kind in builtin_filter_types(extensions) {
        let filter = ConnectionFilter::Builtin(kind);
        let activate = activate.clone();
        menu = menu.item(
            PopupMenuItem::new(kind.label().to_string())
                .checked(*selected == filter)
                .on_click(move |_, window, cx| {
                    activate(filter.clone(), window, cx);
                }),
        );
    }

    for extension in extensions {
        let filter = ConnectionFilter::Extension(extension.clone());
        let activate = activate.clone();
        menu = menu.item(
            PopupMenuItem::new(extension.label.clone())
                .checked(*selected == filter)
                .on_click(move |_, window, cx| {
                    activate(filter.clone(), window, cx);
                }),
        );
    }

    menu
}
