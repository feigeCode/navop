//! 独立卡片视图：每张卡片一个 Entity，hover 状态变化只 notify 自己，
//! 避免 GPUI hover 的 `cx.notify(current_view)` 触发整个扩展管理页重渲染。

use gpui::{App, AppContext, Context, Entity, IntoElement, Render, WeakEntity, Window};
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

/// 卡片负载：二选一。
pub(crate) enum CardPayload {
    Installed(InstalledCardData),
    Marketplace(MarketplaceCardData),
}

/// 独立卡片视图。持有父视图的弱引用，操作按钮通过它回调，不产生反向强引用。
pub(crate) struct ExtensionCardView {
    payload: CardPayload,
    manager: WeakEntity<ExtensionManagerView>,
}

impl ExtensionCardView {
    pub(crate) fn new(
        payload: CardPayload,
        manager: WeakEntity<ExtensionManagerView>,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|_| Self { payload, manager })
    }

    /// 按钮回调统一出口：更新父实体；父实体不存在（页面已关闭）时静默忽略。
    fn update_manager(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut ExtensionManagerView, &mut Window, &mut Context<ExtensionManagerView>),
    ) {
        if let Some(manager) = self.manager.upgrade() {
            manager.update(cx, |view, cx| f(view, window, cx));
        }
    }

    fn render_actions(&self, cx: &Context<Self>) -> Vec<Button> {
        match &self.payload {
            CardPayload::Installed(data) => {
                let busy = data.action_busy;
                let summary_reload = data.summary.clone();
                let reload = Button::new(format!("extension-manager-reload-{}", data.summary.name))
                    .xsmall()
                    .ghost()
                    .icon(IconName::Refresh)
                    .label(t!("Extension.reload").to_string())
                    .disabled(busy)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.update_manager(window, cx, |manager, window, cx| {
                            manager.reload_extension(summary_reload.clone(), window, cx);
                        });
                    }));
                let summary_uninstall = data.summary.clone();
                let uninstall =
                    Button::new(format!("extension-manager-uninstall-{}", data.summary.name))
                        .xsmall()
                        .ghost()
                        .danger()
                        .label(t!("Extension.uninstall").to_string())
                        .disabled(busy)
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.update_manager(window, cx, |manager, window, cx| {
                                manager.uninstall_extension(summary_uninstall.clone(), window, cx);
                            });
                        }));
                vec![reload, uninstall]
            }
            CardPayload::Marketplace(data) => {
                let entry = data.entry.clone();
                let install = Button::new(format!("extension-manager-install-{}", data.entry.id))
                    .xsmall()
                    .primary()
                    .rounded(ButtonRounded::Large)
                    .label(data.install_label.clone())
                    .disabled(data.install_disabled)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.update_manager(window, cx, |manager, window, cx| {
                            manager.install_marketplace_entry(entry.clone(), window, cx);
                        });
                    }));
                vec![install]
            }
        }
    }
}

impl Render for ExtensionCardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (icon, kind, name, version, description, detail_entry) = match &self.payload {
            CardPayload::Installed(data) => (
                crate::cards::kind_icon(data.summary.kind),
                data.summary.kind,
                data.summary.name.clone(),
                data.summary.version.clone(),
                data.summary.description.clone(),
                None,
            ),
            CardPayload::Marketplace(data) => (
                crate::cards::kind_icon(data.entry.kind),
                data.entry.kind,
                data.entry.name.clone(),
                data.entry.version.clone(),
                crate::cards::marketplace_description_public(&data.entry),
                Some(data.entry.clone()),
            ),
        };
        let on_card_click = detail_entry.clone().map(|entry| {
            cx.listener(move |_view, _event, window, cx| {
                crate::detail_dialog::show_detail_dialog(entry.clone(), window, cx);
            })
        });
        crate::cards::render_card_shell(
            crate::cards::CardShell {
                icon,
                kind,
                name,
                version,
                description,
                actions: self.render_actions(cx),
            },
            on_card_click,
            cx,
        )
    }
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
