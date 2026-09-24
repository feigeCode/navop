//! 已安装 / 市场列表与分区网格渲染。

use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::{
    ActiveTheme, Icon, Sizable, button::{Button, ButtonRounded, ButtonVariants},
    h_flex, v_flex,
};
use one_assets::IconName;
use one_ui::{ContentState, IconSize};
use rust_i18n::t;

use crate::{
    ExtensionManagerView, MarketplaceEntry,
    card_view::{installed_card, installed_card_data, marketplace_card, marketplace_card_data},
    cards::{extension_kind_id, kind_label, section_header},
    filter_installed, filter_marketplace, filter_updatable_marketplace, grid::card_grid_metrics,
    state::{MarketplaceLoadState, marketplace_sections},
};

/// body 已有 `p_4`，网格宽度按视口扣掉两侧 padding。
const BODY_HORIZONTAL_PADDING: f32 = 32.0;

impl ExtensionManagerView {
    pub(crate) fn render_installed(
        &self,
        query: &str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let list = filter_installed(&self.installed, query, self.selected_kind);
        if list.is_empty() {
            return empty_extension_state(t!("Extension.no_installed_matches").to_string());
        }
        let action_busy = self.busy.is_some();
        let cards: Vec<AnyElement> = list
            .into_iter()
            .map(|summary| installed_card(installed_card_data(summary, action_busy), cx))
            .collect();
        render_card_grid(cards, body_content_width(window), window)
    }

    pub(crate) fn render_marketplace(
        &self,
        query: &str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let filtered = self.filtered_marketplace_entries(query);
        if filtered.is_empty() {
            return self.empty_marketplace_state(cx);
        }
        let content_width = body_content_width(window);
        let sections = marketplace_sections(filtered, self.selected_kind, query, self.updates_only);
        let action_busy = self.busy.is_some();
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let fg = theme.foreground;
        let section_views: Vec<_> = sections
            .into_iter()
            .map(|section| {
                render_marketplace_section(
                    section,
                    &self.installed,
                    &self.marketplace_load_state,
                    action_busy,
                    content_width,
                    fg,
                    muted,
                    window,
                    cx,
                )
            })
            .collect();
        v_flex()
            .w_full()
            .gap_6()
            .children(section_views)
            .into_any_element()
    }

    fn filtered_marketplace_entries(&self, query: &str) -> Vec<MarketplaceEntry> {
        let updatable_entries;
        let entries: &[MarketplaceEntry] = if self.updates_only {
            updatable_entries =
                filter_updatable_marketplace(&self.marketplace_entries, &self.installed);
            &updatable_entries
        } else {
            &self.marketplace_entries
        };
        filter_marketplace(entries, query, self.selected_kind)
    }

    fn empty_marketplace_state(&self, cx: &Context<Self>) -> gpui::AnyElement {
        if self.updates_only && !self.marketplace_entries.is_empty() {
            return empty_extension_state(t!("Extension.no_updates_available").to_string());
        }
        if self.marketplace_entries.is_empty() {
            match &self.marketplace_load_state {
                MarketplaceLoadState::Loading => {
                    return ContentState::loading(t!("Extension.loading_marketplace").to_string())
                        .into_any_element();
                }
                MarketplaceLoadState::Failed(detail) => {
                    return ContentState::error(
                        t!("Extension.load_marketplace_failed").to_string(),
                    )
                    .detail(detail.clone())
                    .action(
                        Button::new("extension-marketplace-retry")
                            .small()
                            .label(t!("Extension.retry").to_string())
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.load_marketplace(cx);
                            })),
                    )
                    .into_any_element();
                }
                MarketplaceLoadState::NotLoaded | MarketplaceLoadState::Loaded => {}
            }
        }
        empty_extension_state(t!("Extension.no_marketplace_matches").to_string())
    }
}

#[allow(clippy::too_many_arguments)]
fn render_marketplace_section(
    section: crate::state::MarketplaceSection,
    installed: &[crate::ExtensionSummary],
    load_state: &MarketplaceLoadState,
    action_busy: bool,
    content_width: gpui::Pixels,
    fg: gpui::Hsla,
    muted: gpui::Hsla,
    window: &Window,
    cx: &mut Context<ExtensionManagerView>,
) -> gpui::AnyElement {
    let title = section
        .kind
        .map(kind_label)
        .unwrap_or_else(|| t!("Extension.marketplace").to_string());
    let count = section.entries.len();
    let view_all_kind = section.kind;
    let cards: Vec<AnyElement> = section
        .entries
        .into_iter()
        .map(|entry| {
            marketplace_card(
                marketplace_card_data(entry, installed, load_state, action_busy),
                cx,
            )
        })
        .collect();
    v_flex()
        .w_full()
        .gap_4()
        .child(
            // 分区标题行：标题 + 计数居左，「查看全部 N 个 →」胶囊居右。
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(section_header(title, count, fg, muted))
                .child(
                    Button::new(format!(
                        "extension-manager-view-all-{}",
                        view_all_kind.map_or("all", extension_kind_id)
                    ))
                    .xsmall()
                    .rounded(ButtonRounded::Size(px(999.0)))
                    .ghost()
                    .label(t!("Extension.view_all_count", count = count).to_string())
                    .child(
                        Icon::new(IconName::ArrowRight)
                            .with_size(IconSize::Small)
                            .text_color(muted),
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.selected_kind = view_all_kind;
                        cx.notify();
                    })),
                ),
        )
        .child(render_card_grid(cards, content_width, window))
        .into_any_element()
}

/// 网格可用宽度与实际容器保持一致：视口扣掉两侧 padding 后，再按版心上限截断。
/// 否则宽窗口下按视口算出的列数排不进 1200px 容器，换行后右侧留白。
fn grid_content_width(viewport_width: f32) -> f32 {
    (viewport_width - BODY_HORIZONTAL_PADDING).min(crate::render::CONTENT_MAX_WIDTH_PX)
}

fn body_content_width(window: &Window) -> gpui::Pixels {
    px(grid_content_width(f32::from(window.viewport_size().width)))
}

#[cfg(test)]
mod tests {
    use super::grid_content_width;

    #[test]
    fn wide_viewport_clamps_to_content_max_width() {
        // 宽窗口下网格宽度按版心截断，与实际容器宽度一致，避免换行后右侧留白。
        assert_eq!(1200.0, grid_content_width(1920.0));
        assert_eq!(1200.0, grid_content_width(2560.0));
    }

    #[test]
    fn narrow_viewport_uses_full_width_minus_padding() {
        assert_eq!(768.0, grid_content_width(800.0));
        assert_eq!(368.0, grid_content_width(400.0));
    }
}

fn empty_extension_state(message: String) -> gpui::AnyElement {
    ContentState::empty(message)
        .icon(
            Icon::new(IconName::ExtensionsColor)
                .color()
                .with_size(IconSize::Large),
        )
        .into_any_element()
}

fn render_card_grid(
    cards: Vec<AnyElement>,
    content_width: gpui::Pixels,
    window: &Window,
) -> gpui::AnyElement {
    if cards.is_empty() {
        return v_flex().into_any_element();
    }
    let (_columns, card_width) = card_grid_metrics(content_width, window.rem_size());
    let mut grid = div().w_full().flex().flex_wrap().gap_4();
    for card in cards {
        grid = grid.child(div().w(card_width).min_w(card_width).child(card));
    }
    grid.into_any_element()
}
