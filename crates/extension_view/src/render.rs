use gpui::{
    Context, FontWeight, IntoElement, ParentElement, Styled, Window, div, prelude::FluentBuilder,
    px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    progress::Progress,
    v_flex,
};

use crate::{
    ExtensionKind, ExtensionManagerMode, ExtensionManagerView, ExtensionSummary, MarketplaceEntry,
    MarketplaceInstallState, filter_installed, filter_marketplace, marketplace_entry_install_id,
    marketplace_install_state,
    state::{install_progress_value, marketplace_filter_query},
};

const INSTALL_PROGRESS_WIDTH: f32 = 144.0;

impl ExtensionManagerView {
    pub(crate) fn render_toolbar(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .gap_3()
            .child(self.render_title(cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("extension-manager-local")
                            .small()
                            .icon(IconName::File)
                            .label("本地安装")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.select_local_tarball(cx);
                            })),
                    )
                    .child(
                        Button::new("extension-manager-refresh")
                            .small()
                            .icon(IconName::Refresh)
                            .label("刷新")
                            .on_click(cx.listener(move |view, _, _, cx| match view.mode {
                                ExtensionManagerMode::Installed => view.refresh_installed(cx),
                                ExtensionManagerMode::Marketplace => view.load_marketplace(cx),
                            })),
                    ),
            )
    }

    pub(crate) fn render_body(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        self.ensure_marketplace_loaded(cx);
        let query_text = self.search.read(cx).text().to_string();
        let query = marketplace_filter_query(&query_text);
        v_flex()
            .gap_3()
            .child(self.render_tabs(cx))
            .child(
                Input::new(&self.search)
                    .small()
                    .prefix(Icon::new(IconName::Search)),
            )
            .child(match self.mode {
                ExtensionManagerMode::Installed => self.render_installed(query, cx),
                ExtensionManagerMode::Marketplace => self.render_marketplace(query, cx),
            })
            .into_any_element()
    }

    fn render_title(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("扩展管理"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.status.clone()),
            )
            .when_some(
                install_progress_value(self.busy.is_some()),
                |this, value| {
                    this.child(
                        div().w(px(INSTALL_PROGRESS_WIDTH)).child(
                            Progress::new("extension-install-progress")
                                .xsmall()
                                .value(value),
                        ),
                    )
                },
            )
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .child(self.render_mode_button(ExtensionManagerMode::Installed, "已安装", cx))
            .child(self.render_mode_button(ExtensionManagerMode::Marketplace, "扩展市场", cx))
    }

    fn render_mode_button(
        &self,
        mode: ExtensionManagerMode,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(format!("extension-manager-mode-{label}"))
            .small()
            .label(label)
            .when(self.mode == mode, |button| button.primary())
            .on_click(cx.listener(move |view, _, _, cx| {
                view.mode = mode;
                view.ensure_marketplace_loaded(cx);
                cx.notify();
            }))
    }

    fn render_installed(&self, query: &str, cx: &Context<Self>) -> gpui::AnyElement {
        let list = filter_installed(&self.installed, query, None);
        if list.is_empty() {
            return empty_state("尚未安装匹配的扩展", cx);
        }
        v_flex()
            .gap_3()
            .children(
                list.into_iter()
                    .map(|summary| self.render_installed_item(summary, cx)),
            )
            .into_any_element()
    }

    fn render_marketplace(&self, query: &str, cx: &Context<Self>) -> gpui::AnyElement {
        let list = filter_marketplace(&self.marketplace_entries, query, None);
        if list.is_empty() {
            let message = if self.loading {
                "正在加载扩展市场..."
            } else {
                "没有匹配的市场扩展"
            };
            return empty_state(message, cx);
        }
        v_flex()
            .gap_3()
            .children(
                list.into_iter()
                    .map(|entry| self.render_marketplace_item(entry, cx)),
            )
            .into_any_element()
    }

    fn render_installed_item(
        &self,
        summary: ExtensionSummary,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let summary_for_click = summary.clone();
        let action = Button::new(format!("extension-manager-uninstall-{}", summary.name))
            .small()
            .danger()
            .label("卸载")
            .on_click(cx.listener(move |view, _, window, cx| {
                view.uninstall_extension(summary_for_click.clone(), window, cx);
            }));
        extension_card(
            kind_label(summary.kind),
            summary.name,
            summary.version,
            summary.description,
            action,
            cx,
        )
    }

    fn render_marketplace_item(
        &self,
        entry: MarketplaceEntry,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let state = marketplace_install_state(&self.installed, &entry);
        let label = match state {
            MarketplaceInstallState::NotInstalled => "安装",
            MarketplaceInstallState::Installed => "已安装",
            MarketplaceInstallState::UpdateAvailable => "更新",
        };
        let disabled =
            self.loading || self.busy.is_some() || state == MarketplaceInstallState::Installed;
        let entry_for_click = entry.clone();
        let action = Button::new(format!("extension-manager-install-{}", entry.id))
            .small()
            .primary()
            .label(label)
            .disabled(disabled)
            .on_click(cx.listener(move |view, _, window, cx| {
                view.install_marketplace_entry(entry_for_click.clone(), window, cx);
            }));
        extension_card(
            kind_label(entry.kind),
            entry.name.clone(),
            entry.version.clone(),
            marketplace_description(&entry),
            action,
            cx,
        )
    }
}

fn extension_card(
    kind: &'static str,
    name: String,
    version: String,
    description: String,
    action: Button,
    cx: &Context<ExtensionManagerView>,
) -> gpui::AnyElement {
    v_flex()
        .gap_2()
        .p_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(cx.theme().radius)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_sm()
                        .bg(cx.theme().accent)
                        .text_xs()
                        .text_color(cx.theme().accent_foreground)
                        .child(kind),
                )
                .child(div().text_sm().child(name))
                .when(!version.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("v{version}")),
                    )
                }),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(description),
        )
        .child(h_flex().justify_end().child(action))
        .into_any_element()
}

fn marketplace_description(entry: &MarketplaceEntry) -> String {
    if !entry.description.trim().is_empty() {
        return entry.description.clone();
    }
    let id = marketplace_entry_install_id(entry);
    format!("{id} - {}", entry.asset_url)
}

fn empty_state(message: &'static str, cx: &Context<ExtensionManagerView>) -> gpui::AnyElement {
    v_flex()
        .items_center()
        .justify_center()
        .py_16()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(message)
        .into_any_element()
}

fn kind_label(kind: ExtensionKind) -> &'static str {
    match kind {
        ExtensionKind::Language => "语言",
        ExtensionKind::DatabaseDriver => "数据库驱动",
        ExtensionKind::Composite => "复合扩展",
    }
}
