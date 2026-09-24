//! 卡片构造：把数据快照渲染成卡片元素。
//!
//! 卡片必须由管理页视图**直接**渲染，不要在 `render` 里 `cx.new` 子实体：
//! GPUI 的点击检测把 mousedown 状态记在元素 id（含所属实体的 `ElementId::View`）上，
//! 子实体每帧重建会让整棵卡片子树的 id 变化，mouseup 时查不到 mousedown 记录，
//! 点击被静默丢弃——表现为卡片上的安装/卸载按钮和「点卡片开详情」全部无响应，
//! 而同一页面里由视图直接渲染的搜索框 / chips 正常。

use gpui::{AnyElement, Context};
use gpui_component::{Disableable, Sizable, button::{Button, ButtonRounded, ButtonVariants}};
use one_assets::IconName;
use rust_i18n::t;

use crate::{
    ExtensionManagerView, ExtensionSummary, MarketplaceEntry,
    MarketplaceInstallState, marketplace_install_state, state::MarketplaceLoadState,
};

/// 已安装扩展卡片的数据快照。
pub(crate) struct InstalledCardData {
    pub summary: ExtensionSummary,
    pub action_busy: bool,
}

/// 市场扩展卡片的数据快照。
pub(crate) struct MarketplaceCardData {
    pub entry: MarketplaceEntry,
    pub install_label: String,
    pub install_disabled: bool,
}

/// 渲染一张已安装扩展卡片（重载 / 卸载）。
pub(crate) fn installed_card(
    data: InstalledCardData,
    cx: &mut Context<ExtensionManagerView>,
) -> AnyElement {
    let InstalledCardData {
        summary,
        action_busy,
    } = data;
    let summary_reload = summary.clone();
    let reload = Button::new(format!("extension-manager-reload-{}", summary.name))
        .xsmall()
        .ghost()
        .icon(IconName::Refresh)
        .label(t!("Extension.reload").to_string())
        .disabled(action_busy)
        .on_click(cx.listener(move |view, _, window, cx| {
            view.reload_extension(summary_reload.clone(), window, cx);
        }));
    let summary_uninstall = summary.clone();
    let uninstall = Button::new(format!("extension-manager-uninstall-{}", summary.name))
        .xsmall()
        .ghost()
        .danger()
        .label(t!("Extension.uninstall").to_string())
        .disabled(action_busy)
        .on_click(cx.listener(move |view, _, window, cx| {
            view.uninstall_extension(summary_uninstall.clone(), window, cx);
        }));
    crate::cards::card_shell(
        card_shell_data(
            summary.kind,
            summary.name.clone(),
            summary.version.clone(),
            summary.description.clone(),
            vec![reload, uninstall],
        ),
        format!("ext-card-installed-{}", summary.name),
        cx,
    )
}

/// 渲染一张市场扩展卡片（安装 / 更新 / 已安装 / 需升级），整卡可点开详情。
pub(crate) fn marketplace_card(
    data: MarketplaceCardData,
    cx: &mut Context<ExtensionManagerView>,
) -> AnyElement {
    let MarketplaceCardData {
        entry,
        install_label,
        install_disabled,
    } = data;
    let install_entry = entry.clone();
    let install = Button::new(format!("extension-manager-install-{}", entry.id))
        .xsmall()
        .primary()
        .rounded(ButtonRounded::Large)
        .label(install_label)
        .disabled(install_disabled)
        .on_click(cx.listener(move |view, _, window, cx| {
            // 整卡可点（打开详情），按钮点击必须阻止冒泡，否则点安装会连详情窗口一起弹出。
            cx.stop_propagation();
            view.install_marketplace_entry(install_entry.clone(), window, cx);
        }));
    let detail_entry = entry.clone();
    let on_card_click = cx.listener(move |_view, _event, window, cx| {
        crate::detail_dialog::show_detail_dialog(detail_entry.clone(), window, cx);
    });
    crate::cards::clickable_card_shell(
        card_shell_data(
            entry.kind,
            entry.name.clone(),
            entry.version.clone(),
            crate::cards::marketplace_description_public(&entry),
            vec![install],
        ),
        format!("ext-card-market-{}", entry.id),
        on_card_click,
        cx,
    )
}

/// 从市场条目计算卡片所需快照（安装按钮文案 / 是否可点）。
pub(crate) fn marketplace_card_data(
    entry: MarketplaceEntry,
    installed: &[ExtensionSummary],
    load_state: &MarketplaceLoadState,
    busy: bool,
) -> MarketplaceCardData {
    let state = marketplace_install_state(installed, &entry);
    let install_disabled = load_state.is_loading()
        || busy
        || state == MarketplaceInstallState::Installed
        || !entry.host_compatible;
    MarketplaceCardData {
        install_label: crate::cards::marketplace_action_label_public(state, entry.host_compatible),
        install_disabled,
        entry,
    }
}

/// 已安装卡片快照。
pub(crate) fn installed_card_data(
    summary: ExtensionSummary,
    action_busy: bool,
) -> InstalledCardData {
    InstalledCardData {
        summary,
        action_busy,
    }
}

fn card_shell_data(
    kind: crate::ExtensionKind,
    name: String,
    version: String,
    description: String,
    actions: Vec<Button>,
) -> crate::cards::CardShell {
    crate::cards::CardShell {
        icon: crate::cards::kind_icon(kind),
        kind,
        name,
        version,
        description,
        actions,
    }
}

#[cfg(test)]
mod tests {
    /// 卡片按钮在 `ExtensionManagerView` 上挂 listener，且安装按钮要阻止冒泡
    /// 到「整卡点击打开详情」。
    #[test]
    fn card_actions_listen_on_the_manager_view() {
        let source = include_str!("card_view.rs");
        // 模式拼出来，避免测试文本自己也命中。
        let listener = format!("cx.{}move", "listener(");
        assert_eq!(
            4,
            source.matches(&listener).count(),
            "reload / uninstall / install / 整卡点击都应挂在管理页视图上"
        );
        assert!(
            source.contains("cx.stop_propagation();"),
            "安装按钮必须阻止冒泡，否则点安装会同时弹出详情窗口"
        );
    }

    /// 回归护栏：卡片子树里的交互元素 id 必须跨帧稳定，所以卡片不能是每帧新建的
    /// 子实体（那会让 id 变化、点击状态丢失，卡片整体点不动）。
    #[test]
    fn cards_do_not_create_child_entities_while_rendering() {
        // 同上：模式拼出来，避免断言文本自匹配。
        let new_entity = format!("cx.{}new(", "");
        for source in [
            include_str!("card_view.rs"),
            include_str!("cards.rs"),
            include_str!("lists.rs"),
        ] {
            assert!(
                !source.contains(&new_entity),
                "卡片渲染期间不要新建实体，否则元素 id 每帧变化，点击会被丢弃"
            );
        }
    }
}
