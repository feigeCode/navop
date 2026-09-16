use super::*;
use gpui::{point, size};

fn bounds(width: f32) -> Bounds<Pixels> {
    Bounds {
        origin: point(px(0.0), px(0.0)),
        size: size(px(width), px(24.0)),
    }
}

fn labels(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("kind-{index}")).collect()
}

#[test]
fn all_types_stay_visible_when_they_fit_without_the_more_button() {
    let widths = [Some(px(50.0)), Some(px(60.0)), Some(px(70.0))];
    assert_eq!(
        3,
        resolve_visible_count(px(300.0), &widths, Some(px(60.0)), px(4.0))
    );
}

#[test]
fn hidden_types_reserve_room_for_the_more_button() {
    let widths = [Some(px(50.0)), Some(px(60.0)), Some(px(70.0))];
    let more = Some(px(60.0));
    assert_eq!(3, resolve_visible_count(px(200.0), &widths, more, px(4.0)));
    assert_eq!(1, resolve_visible_count(px(150.0), &widths, more, px(4.0)));
    assert_eq!(0, resolve_visible_count(px(100.0), &widths, more, px(4.0)));
}

#[test]
fn unmeasured_widths_render_everything_until_the_next_measurement() {
    let widths = [None, Some(px(60.0)), None];
    assert_eq!(
        3,
        resolve_visible_count(px(120.0), &widths, Some(px(60.0)), px(4.0))
    );
    let widths = [Some(px(50.0)), Some(px(60.0))];
    assert_eq!(2, resolve_visible_count(px(120.0), &widths, None, px(4.0)));
}

#[test]
fn fit_check_keeps_a_pixel_of_safety_margin() {
    let widths = [Some(px(50.0)), Some(px(60.0))];
    assert_eq!(
        1,
        resolve_visible_count(px(114.0), &widths, Some(px(50.0)), px(4.0))
    );
}

#[test]
fn measurement_cache_invalidates_when_labels_or_rem_change() {
    let mut bar = ConnectionTypeFilterBar::default();
    let item_labels = labels(3);
    assert!(bar.measurement_stale(&item_labels, px(16.0)));

    let children = vec![bounds(50.0), bounds(60.0), bounds(70.0), bounds(60.0)];
    bar.record_available_width(px(150.0));
    assert!(bar.record(&FilterObservation {
        children: &children,
        labels: &item_labels,
        rem_size: px(16.0),
        gap: px(4.0),
        stale: true,
        visible_count: 3,
        show_more: true,
    }));
    assert_eq!(1, bar.visible_count);
    assert!(!bar.measurement_stale(&item_labels, px(16.0)));
    assert!(bar.measurement_stale(&item_labels, px(20.0)));
    assert!(bar.measurement_stale(&labels(4), px(16.0)));
}

#[test]
fn visible_count_grows_back_when_the_container_widens() {
    let mut bar = ConnectionTypeFilterBar::default();
    let item_labels = labels(3);
    let wide_children = vec![bounds(50.0), bounds(60.0), bounds(70.0), bounds(60.0)];
    bar.record_available_width(px(150.0));
    bar.record(&FilterObservation {
        children: &wide_children,
        labels: &item_labels,
        rem_size: px(16.0),
        gap: px(4.0),
        stale: true,
        visible_count: 3,
        show_more: true,
    });
    assert_eq!(1, bar.visible_count);

    let narrow_children = vec![bounds(50.0), bounds(60.0)];
    bar.record_available_width(px(260.0));
    assert!(bar.record(&FilterObservation {
        children: &narrow_children,
        labels: &item_labels,
        rem_size: px(16.0),
        gap: px(4.0),
        stale: false,
        visible_count: 1,
        show_more: true,
    }));
    assert_eq!(3, bar.visible_count);
}

#[test]
fn filter_bar_renders_every_type_with_a_measured_overflow_menu() {
    let source = include_str!("connection_type_filter_bar.rs");
    // 筛选项来自共享清单：All + 内置类型 + 扩展贡献。
    assert!(source.contains("filter_targets"));
    assert!(source.contains("ConnectionFilter::Builtin"));
    assert!(source.contains("ConnectionFilter::Extension"));
    assert!(source.contains("connection_type_navigation_icon"));
    assert!(source.contains("IconName::Apps"));
    assert!(source.contains(".rounded(cx.theme().radius_full())"));
    assert!(source.contains(".outline()"));
    assert!(!source.contains(".ghost()"));
    assert!(source.contains("selected_filter_style(button, cx)"));
    assert!(source.contains("SELECTED_BACKGROUND_OPACITY"));
    assert!(source.contains("on_prepaint"));
    assert!(source.contains("on_children_prepainted"));
    assert!(source.contains("resolve_visible_count"));
    assert!(source.contains("home-type-filter-more"));
    assert!(source.contains("skip(visible_count)"));
}
