use gpui::{
    Context, FontWeight, IntoElement, ParentElement, Styled, Window, div, linear_color_stop,
    linear_gradient, px, rems, prelude::FluentBuilder,
};
use gpui_component::{
    ActiveTheme, Icon, Sizable, Size, button::{Button, ButtonRounded, ButtonVariants},
    h_flex, input::Input, progress::Progress, scroll::ScrollableElement, v_flex,
};
use one_assets::IconName;
use one_ui::{IconSize, PanelHeader, PanelHeaderVariant};
use rust_i18n::t;

use crate::{
    ExtensionKind, ExtensionManagerMode, ExtensionManagerView,
    cards::{extension_kind_id, kind_label},
    chips,
    state::{EXTENSION_KINDS, install_progress_value, marketplace_filter_query},
};

/// 内容区最大宽度：宽窗口下保持目录式版心，避免卡片被拉散。
pub(crate) const CONTENT_MAX_WIDTH_PX: f32 = 1200.0;
/// 版心两侧留白合计，估算 chips 可用宽度时扣除。
const BODY_HORIZONTAL_PADDING: f32 = 32.0;
/// Market 目录搜索最大宽度（约 40rem）。
const SEARCH_MAX_WIDTH_REMS: f32 = 40.0;
/// 搜索框高度，明显高于普通输入，形成目录主入口。
const SEARCH_HEIGHT_PX: f32 = 56.0;
const PILL_ROUNDED_PX: f32 = 999.0;
/// Hero 标题字号（rem），对应网页 56px 视觉层级。
const HERO_TITLE_SIZE_REMS: f32 = 3.0;
/// Hero 标题行高（rem），两行紧凑排列。
const HERO_TITLE_LINE_HEIGHT_REMS: f32 = 3.4;
/// Hero 标题末尾的打字光标块，呼应 Market 网站的输入动效。
const HERO_CARET_WIDTH_PX: f32 = 5.0;
const HERO_CARET_HEIGHT_PX: f32 = 36.0;
const HERO_CARET_BOTTOM_OFFSET_PX: f32 = 10.0;
/// Hero 区留白：顶部宽松、底部收紧让分类 chips 紧贴搜索框。
const HERO_PADDING_TOP_REMS: f32 = 2.0;
const HERO_PADDING_BOTTOM_REMS: f32 = 0.25;
/// Hero 底部装饰性渐变带高度。
const HERO_TINT_BAND_HEIGHT_PX: f32 = 160.0;
/// 在线状态圆点尺寸。
const STATUS_DOT_SIZE_PX: f32 = 6.0;
/// 安装进度条宽度。
const INSTALL_PROGRESS_WIDTH: f32 = 144.0;

impl ExtensionManagerView {
    pub(crate) fn render_toolbar(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let title = self.render_title(cx);
        PanelHeader::new("extension-manager-toolbar")
            .variant(PanelHeaderVariant::Toolbar)
            .title(title)
            .trailing(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("extension-manager-offline-download")
                            .small()
                            .icon(IconName::Globe)
                            .label(t!("Extension.offline_download").to_string())
                            .on_click(|_, _, cx| {
                                crate::offline_package_dialog::show_offline_package_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new("extension-manager-local")
                            .small()
                            .icon(IconName::File)
                            .label(t!("Extension.local_install").to_string())
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.select_local_tarball(cx);
                            })),
                    )
                    .child(
                        Button::new("extension-manager-refresh")
                            .small()
                            .icon(IconName::Refresh)
                            .label(t!("Common.refresh").to_string())
                            .on_click(cx.listener(move |view, _, _, cx| match view.mode {
                                ExtensionManagerMode::Installed => view.refresh_installed(cx),
                                ExtensionManagerMode::Marketplace => view.load_marketplace(cx),
                            })),
                    ),
            )
    }

    pub(crate) fn render_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.ensure_marketplace_loaded(cx);
        let query_text = self.search.read(cx).text().to_string();
        let query = marketplace_filter_query(&query_text);
        let content = match self.mode {
            ExtensionManagerMode::Installed => self.render_installed(query, window, cx),
            ExtensionManagerMode::Marketplace => self.render_marketplace(query, window, cx),
        };

        // Market 结构：居中版心内 Hero → 分类 chips → 列表。
        // 模式切换在工具栏，不抢 Hero 视觉。
        div()
            .size_full()
            .overflow_y_scrollbar()
            .relative()
            .child(self.render_hero_tint_band(cx))
            .child(
                v_flex()
                    .relative()
                    .w_full()
                    .max_w(px(CONTENT_MAX_WIDTH_PX))
                    .mx_auto()
                    .gap_5()
                    .child(self.render_search_hero(window, cx))
                    .child(self.render_kind_filters(window, cx))
                    .child(content),
            )
            .into_any_element()
    }

    /// Hero 背后的浅色渐变带：绝对定位在顶部，给目录页一个柔和的视觉起点。
    fn render_hero_tint_band(&self, cx: &Context<Self>) -> impl IntoElement {
        let top = cx.theme().muted.opacity(0.55);
        let bottom = cx.theme().background.opacity(0.0);
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(HERO_TINT_BAND_HEIGHT_PX))
            .bg(linear_gradient(
                180.0,
                linear_color_stop(top, 0.0),
                linear_color_stop(bottom, 1.0),
            ))
    }

    fn render_title(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_1()
            .min_w_0()
            .gap_3()
            .items_center()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .whitespace_nowrap()
                    .child(t!("Extension.manager_title").to_string()),
            )
            .child(self.render_mode_pills(cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .items_end()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
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
                    ),
            )
    }

    /// Market Hero：统计徽章 → 两行大标题 → 副标题 → 居中大搜索；分类 chips 紧贴其下。
    fn render_search_hero(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let is_marketplace = self.mode == ExtensionManagerMode::Marketplace;
        let muted = cx.theme().muted_foreground;
        let fg = cx.theme().foreground;
        let accent = cx.theme().accent;
        let rem = window.rem_size();
        let search_width = (rem * SEARCH_MAX_WIDTH_REMS)
            .min(window.viewport_size().width - px(64.0))
            .max(px(280.0));

        let search = div()
            .w(search_width)
            .h(px(SEARCH_HEIGHT_PX))
            .child(
                Input::new(&self.search)
                    .with_size(Size::Large)
                    .size_full()
                    .rounded(px(12.0))
                    .prefix(Icon::new(IconName::Search).with_size(IconSize::Medium)),
            );

        let (title_top, title_bottom) = if is_marketplace {
            (
                t!("Extension.hero_title_top").to_string(),
                t!("Extension.hero_title_bottom").to_string(),
            )
        } else {
            (
                t!("Extension.hero_title_installed_top").to_string(),
                t!("Extension.hero_title_installed_bottom").to_string(),
            )
        };

        v_flex()
            .w_full()
            .pt(rems(HERO_PADDING_TOP_REMS))
            .pb(rems(HERO_PADDING_BOTTOM_REMS))
            .gap_3()
            .items_center()
            .child(self.marketplace_stats_label(cx))
            .child(
                div()
                    .text_center()
                    .text_size(rems(HERO_TITLE_SIZE_REMS))
                    .line_height(rems(HERO_TITLE_LINE_HEIGHT_REMS))
                    .font_weight(FontWeight::BOLD)
                    .text_color(fg)
                    .child(
                        // 末行追加打字光标块，复刻 Market 网站的输入动效。
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(title_top)
                            .child(
                                h_flex()
                                    .items_end()
                                    .gap_2()
                                    .child(title_bottom)
                                    .child(
                                        div()
                                            .w(px(HERO_CARET_WIDTH_PX))
                                            .h(px(HERO_CARET_HEIGHT_PX))
                                            .mb(px(HERO_CARET_BOTTOM_OFFSET_PX))
                                            .rounded(px(2.0))
                                            .bg(accent),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(self.hero_subtitle(is_marketplace)),
            )
            .child(search)
    }

    fn hero_subtitle(&self, is_marketplace: bool) -> String {
        if is_marketplace {
            t!("Extension.hero_subtitle").to_string()
        } else {
            t!("Extension.hero_subtitle_installed").to_string()
        }
    }

    /// 统计徽章：在线绿点 + 计数 + 更新时间，底色用主题 success 浅色。
    fn marketplace_stats_label(&self, cx: &Context<Self>) -> impl IntoElement {
        let count = self.marketplace_entries.len();
        let muted = cx.theme().muted_foreground;
        let border = gpui::Hsla { a: 0.25, ..muted };
        let badge_bg = if cx.theme().is_dark() {
            cx.theme().success.opacity(0.12)
        } else {
            cx.theme().success.opacity(0.08)
        };
        h_flex()
            .px_3()
            .py_1()
            .gap_2()
            .items_center()
            .rounded(px(PILL_ROUNDED_PX))
            .border_1()
            .border_color(border)
            .bg(badge_bg)
            .text_xs()
            .text_color(muted)
            .child(
                div()
                    .size(px(STATUS_DOT_SIZE_PX))
                    .rounded_full()
                    .bg(cx.theme().success),
            )
            .child(t!("Extension.hero_badge_extension_count", count = count).to_string())
            .child(
                div()
                    .whitespace_nowrap()
                    .child(t!("Extension.hero_badge_updated").to_string()),
            )
    }

    fn render_mode_pills(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_1()
            .child(self.render_mode_button(
                ExtensionManagerMode::Installed,
                t!("Extension.installed").to_string(),
                cx,
            ))
            .child(self.render_mode_button(
                ExtensionManagerMode::Marketplace,
                t!("Extension.marketplace").to_string(),
                cx,
            ))
    }

    fn render_mode_button(
        &self,
        mode: ExtensionManagerMode,
        label: String,
        cx: &mut Context<Self>,
    ) -> Button {
        let selected = self.mode == mode;
        Button::new(format!("extension-manager-mode-{label}"))
            .small()
            .rounded(ButtonRounded::Size(px(PILL_ROUNDED_PX)))
            .label(label)
            .when(selected, |button| button.primary())
            .when(!selected, |button| button.ghost())
            .on_click(cx.listener(move |view, _, _, cx| {
                view.set_mode(mode, cx);
            }))
    }

    /// 分类 chips：xsmall 居中单行；超宽折叠为「前缀 + 更多」，点击展开多行。
    fn render_kind_filters(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let is_marketplace = self.mode == ExtensionManagerMode::Marketplace;
        let mut entries: Vec<(Option<ExtensionKind>, String)> = vec![(
            None,
            t!("Extension.kind_all").to_string(),
        )];
        entries.extend(
            EXTENSION_KINDS
                .into_iter()
                .map(|kind| (Some(kind), kind_label(kind))),
        );
        if is_marketplace {
            entries.push((None, t!("Extension.updates_only").to_string()));
        }
        // “有更新”是特殊过滤项：None kind 已被“全部”占用，用 updates_only 标志单独识别。
        let is_updates_only = |kind: Option<ExtensionKind>, index: usize| -> bool {
            is_marketplace && index == entries.len() - 1 && kind.is_none()
        };

        let available_width = f32::from(window.viewport_size().width)
            .min(CONTENT_MAX_WIDTH_PX)
            - BODY_HORIZONTAL_PADDING;
        let labels: Vec<&str> = entries.iter().map(|(_, label)| label.as_str()).collect();
        // 选中索引：市场模式下“有更新”激活时定位到最后一项，否则按 kind 匹配。
        let updates_only_index = entries.len() - 1;
        let selected_index = if is_marketplace && self.updates_only {
            updates_only_index
        } else {
            entries
                .iter()
                .position(|(kind, _)| kind == &self.selected_kind)
                .unwrap_or(usize::MAX)
        };
        let selected_for_plan = (selected_index != usize::MAX).then_some(selected_index);

        let plan = chips::plan_chip_row(&labels, available_width, selected_for_plan);
        let collapsed = plan.hidden_count > 0 && !self.chips_expanded;

        let chip_ids: Vec<String> = entries
            .iter()
            .enumerate()
            .map(|(index, (kind, _))| {
                if is_updates_only(*kind, index) {
                    "updates-only".to_string()
                } else {
                    kind.map_or("all", extension_kind_id).to_string()
                }
            })
            .collect();

        let chip_iter: Vec<usize> = if collapsed {
            plan.visible
        } else {
            (0..entries.len()).collect()
        };

        let mut row = h_flex()
            .w_full()
            .flex_wrap()
            .gap_2()
            .justify_center()
            .items_center();
        for index in chip_iter {
            let (kind, label) = &entries[index];
            let selected = if is_updates_only(*kind, index) {
                self.updates_only
            } else {
                self.selected_kind == *kind
                    && !(is_marketplace && self.updates_only && kind.is_none())
            };
            row = row.child(self.render_kind_filter_button(
                format!("extension-manager-kind-{}", chip_ids[index]),
                *kind,
                label.clone(),
                selected,
                cx,
            ));
        }
        if collapsed {
            row = row.child(self.render_chips_toggle_button(false, cx));
        } else if plan.hidden_count > 0 {
            row = row.child(self.render_chips_toggle_button(true, cx));
        }
        row.into_any_element()
    }

    /// 「更多 N」/「收起」切换按钮，仅在溢出时出现。
    fn render_chips_toggle_button(&self, expanded: bool, cx: &mut Context<Self>) -> Button {
        let button = Button::new("extension-manager-chips-toggle")
            .xsmall()
            .rounded(ButtonRounded::Size(px(PILL_ROUNDED_PX)));
        if expanded {
            button
                .ghost()
                .label(t!("Extension.chips_collapse").to_string())
                .on_click(cx.listener(|view, _, _, cx| {
                    view.chips_expanded = false;
                    cx.notify();
                }))
        } else {
            button
                .outline()
                .label(t!("Extension.chips_more").to_string())
                .on_click(cx.listener(|view, _, _, cx| {
                    view.chips_expanded = true;
                    cx.notify();
                }))
        }
    }

    fn render_kind_filter_button(
        &self,
        id: String,
        kind: Option<ExtensionKind>,
        label: String,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let updates_only_target = kind.is_none() && id == "updates-only";
        Button::new(id)
            .xsmall()
            .rounded(ButtonRounded::Size(px(PILL_ROUNDED_PX)))
            .label(label)
            .when(selected, |button| button.primary())
            .when(!selected, |button| button.outline())
            .on_click(cx.listener(move |view, _, _, cx| {
                if updates_only_target {
                    view.updates_only = !view.updates_only;
                } else {
                    view.selected_kind = kind;
                    if kind.is_some() {
                        view.updates_only = false;
                    }
                }
                cx.notify();
            }))
    }
}
