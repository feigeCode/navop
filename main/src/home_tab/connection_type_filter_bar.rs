//! 横向平铺的连接类型筛选条。容器与 chip 在 prepaint 阶段实测，窗口 Resize
//! 后自动重算；溢出的类型进入最右侧的「更多」菜单。

use super::*;
use gpui::Bounds;
use gpui_component::Selectable as _;
use one_ui::IconSize;

/// `gap_1` 的 rem 系数（0.25rem），与渲染时的水平间距保持一致。
const FILTER_GAP_REM: f32 = 0.25;
/// 像素取整误差的安全余量：宁可少放一个 chip，也要先保证「更多」放得下。
const FIT_SAFETY_MARGIN: f32 = 1.0;
/// 选中胶囊使用轻量主色底，避免 Button active 色过重。
const SELECTED_BACKGROUND_OPACITY: f32 = 0.10;
const SELECTED_BORDER_OPACITY: f32 = 0.55;

/// 筛选条的测量与可见性状态，随 `HomePage` 生命周期保留。
#[derive(Default)]
pub(crate) struct ConnectionTypeFilterBar {
    /// 容器实测可用宽度；`None` 表示尚未测量。
    available_width: Option<Pixels>,
    /// 各类型 chip 的实测宽度，与 `ConnectionType::all()` 顺序一一对应。
    chip_widths: Vec<Option<Pixels>>,
    /// 「更多」按钮的实测宽度。
    more_width: Option<Pixels>,
    /// 当前应平铺展示的类型数量（`ConnectionType::all()` 的前 N 项）。
    visible_count: usize,
    /// 测量缓存对应的标签与 rem 尺寸；语言切换或缩放变化时整表重测。
    measured_labels: Vec<String>,
    measured_rem: Pixels,
}

/// 单帧渲染计划：平铺数量与是否需要「更多」。
#[derive(Clone, Copy)]
struct FilterBarPlan {
    visible_count: usize,
    show_more: bool,
}

/// prepaint 阶段上报的一次测量结果。
struct FilterObservation<'a> {
    children: &'a [Bounds<Pixels>],
    labels: &'a [String],
    rem_size: Pixels,
    gap: Pixels,
    stale: bool,
    visible_count: usize,
    show_more: bool,
}

impl ConnectionTypeFilterBar {
    /// 测量缓存是否失效（首次渲染、语言切换或 rem 缩放变化）。
    fn measurement_stale(&self, labels: &[String], rem_size: Pixels) -> bool {
        self.measured_labels != labels
            || self.measured_rem != rem_size
            || self.chip_widths.len() != labels.len()
    }

    /// 根据缓存宽度决定本帧计划；缓存失效或「更多」宽度未知时先全量渲染以测量。
    fn plan(&self, stale: bool, item_count: usize) -> FilterBarPlan {
        let visible_count = if stale {
            item_count
        } else {
            self.visible_count.min(item_count)
        };
        FilterBarPlan {
            visible_count,
            show_more: stale || self.more_width.is_none() || visible_count < item_count,
        }
    }

    /// 记录容器宽度（由外层容器的 prepaint 回调上报）。
    fn record_available_width(&mut self, width: Pixels) {
        self.available_width = Some(width);
    }

    /// 记录本次 prepaint 的子项宽度并重算可见数量；返回可见数量是否变化。
    fn record(&mut self, observation: &FilterObservation<'_>) -> bool {
        if observation.stale {
            self.chip_widths.clear();
            self.more_width = None;
        }
        self.chip_widths.resize(observation.labels.len(), None);
        for (index, bounds) in observation
            .children
            .iter()
            .take(observation.visible_count)
            .enumerate()
        {
            self.chip_widths[index] = Some(bounds.size.width);
        }
        if observation.show_more {
            if let Some(bounds) = observation.children.get(observation.visible_count) {
                self.more_width = Some(bounds.size.width);
            }
        }
        self.measured_labels = observation.labels.to_vec();
        self.measured_rem = observation.rem_size;
        let available_width = self.available_width.unwrap_or(Pixels::ZERO);
        let next = resolve_visible_count(
            available_width,
            &self.chip_widths,
            self.more_width,
            observation.gap,
        );
        let changed = next != self.visible_count;
        self.visible_count = next;
        changed
    }
}

/// 计算最多可平铺的 chip 数；未测量时先返回全量，溢出时为「更多」预留空间。
pub(crate) fn resolve_visible_count(
    available_width: Pixels,
    widths: &[Option<Pixels>],
    more_width: Option<Pixels>,
    gap: Pixels,
) -> usize {
    let item_count = widths.len();
    let Some(more_width) = more_width else {
        return item_count;
    };
    let mut row_width = Pixels::ZERO;
    for (index, width) in widths.iter().enumerate() {
        let Some(width) = *width else {
            return item_count;
        };
        row_width = if index == 0 {
            width
        } else {
            row_width + gap + width
        };
    }
    let budget = available_width - px(FIT_SAFETY_MARGIN);
    if row_width <= budget {
        return item_count;
    }
    let mut used = Pixels::ZERO;
    let mut count = 0;
    for (index, width) in widths.iter().enumerate() {
        let Some(width) = *width else {
            return item_count;
        };
        let next = if index == 0 {
            width
        } else {
            used + gap + width
        };
        if next + gap + more_width <= budget {
            used = next;
            count = index + 1;
        } else {
            break;
        }
    }
    count
}

impl HomePage {
    /// 横向平铺的类型筛选条；放不下的类型收入最右侧的「更多」菜单。
    pub(super) fn render_connection_type_filter_bar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kinds = ConnectionType::all();
        let labels: Vec<String> = kinds
            .iter()
            .map(|kind| connection_type_label(*kind))
            .collect();
        let rem_size = window.rem_size();
        let stale = self
            .connection_type_filter
            .measurement_stale(&labels, rem_size);
        let plan = self.connection_type_filter.plan(stale, kinds.len());
        let mut chips: Vec<AnyElement> = Vec::with_capacity(plan.visible_count + 1);
        for kind in kinds.iter().take(plan.visible_count) {
            chips.push(self.render_connection_type_chip(*kind, cx));
        }
        if plan.show_more {
            chips.push(self.render_connection_type_more(&kinds, plan.visible_count, cx));
        }
        let view = cx.entity();
        let row_view = view.clone();
        let row = h_flex()
            .items_center()
            .gap_1()
            .on_children_prepainted(move |children, window, cx| {
                let changed = row_view.update(cx, |home, _| {
                    home.connection_type_filter.record(&FilterObservation {
                        children: &children,
                        labels: &labels,
                        rem_size,
                        gap: rem_size * FILTER_GAP_REM,
                        stale,
                        visible_count: plan.visible_count,
                        show_more: plan.show_more,
                    })
                });
                if changed {
                    let view = row_view.clone();
                    window.on_next_frame(move |_, cx| cx.notify(view.entity_id()));
                }
            })
            .children(chips);
        gpui_component::ElementExt::on_prepaint(
            div()
                .id("home-type-filter-bar")
                .flex_1()
                .min_w_0()
                .overflow_hidden(),
            move |bounds, _, cx| {
                view.update(cx, |home, _| {
                    home.connection_type_filter
                        .record_available_width(bounds.size.width);
                });
            },
        )
        .child(row)
        .into_any_element()
    }

    /// 所有 chip 都保留 1px 胶囊边框，避免选中后宽度变化破坏测量缓存。
    fn render_connection_type_chip(
        &self,
        kind: ConnectionType,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.selected_filter == kind;
        Button::new(SharedString::from(format!("home-type-filter-{kind}")))
            .small()
            .rounded(cx.theme().radius_full())
            .outline()
            .icon(connection_type_filter_icon(kind))
            .label(connection_type_label(kind))
            .selected(selected)
            .when(selected, |button| selected_filter_style(button, cx))
            .on_click(cx.listener(move |home, _, _, cx| home.set_selected_filter(kind, cx)))
            .into_any_element()
    }

    /// 「更多」：仅收纳当前放不下的类型；选中项被收纳时按钮保持选中态。
    fn render_connection_type_more(
        &self,
        kinds: &[ConnectionType],
        visible_count: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.selected_filter;
        let hidden: Vec<ConnectionType> = kinds.iter().skip(visible_count).copied().collect();
        let hidden_selected = hidden.contains(&selected);
        let view = cx.entity();
        Button::new("home-type-filter-more")
            .small()
            .rounded(cx.theme().radius_full())
            .outline()
            .icon(IconName::Ellipsis.mono().with_size(IconSize::Small))
            .label(t!("Home.connection_filter_more"))
            .selected(hidden_selected)
            .when(hidden_selected, |button| selected_filter_style(button, cx))
            .dropdown_caret(true)
            .tooltip(t!("Home.connection_filter"))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let view = view.clone();
                crate::connection_type_menu::build_filter_menu(
                    menu,
                    &hidden,
                    selected,
                    std::rc::Rc::new(move |filter, _, cx| {
                        view.update(cx, |home, cx| home.set_selected_filter(filter, cx));
                    }),
                )
            })
            .into_any_element()
    }
}

/// 截图中的选中态：浅主色背景、柔和主色边框、清晰主色前景。
fn selected_filter_style(button: Button, cx: &App) -> Button {
    button
        .primary()
        .bg(cx.theme().primary.opacity(SELECTED_BACKGROUND_OPACITY))
        .border_color(cx.theme().primary.opacity(SELECTED_BORDER_OPACITY))
        .text_color(cx.theme().primary)
}

/// 筛选 chip 的统一线稿图标；「全部类型」沿用工具栏的 Apps 网格图标。
fn connection_type_filter_icon(kind: ConnectionType) -> Icon {
    if kind == ConnectionType::All {
        IconName::Apps.mono().with_size(IconSize::Small)
    } else {
        connection_type_navigation_icon(kind, ConnectionVisualSize::Tree)
    }
}

#[cfg(test)]
#[path = "connection_type_filter_bar_tests.rs"]
mod tests;
