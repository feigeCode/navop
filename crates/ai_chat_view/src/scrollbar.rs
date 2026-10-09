//! 滚动条:对齐 waku 的 overlay scroller 观感。
//!
//! waku 的滚动条是「只有滑块、没有轨道底」的 overlay:静置时隐形,滚动时淡入,
//! 停手一会儿再淡出,指针悬上去变粗。gpui 的底座已经把这套动作都实现了
//! (`ScrollbarMode::Scrolling` + 主题里投影的动效),差的只是**度量与配色**:
//! 底座默认轨道 16 / 滑块 6 / 内缩 4 / 最短 48,而且轨道带底色,看起来比 waku
//! 重一档。这里只覆盖这些都能量化的部分,不再自研一遍绘制。

use gpui::{App, IntoElement, Pixels, ScrollHandle, px};
use gpui_component::{
    ActiveTheme,
    scroll::{Scrollbar, ScrollbarMode, ScrollbarStyles},
};

/// 滚动条占的横向空间。
///
/// 外层容器要按它留位,否则滑块会被裁掉一条边。
pub(crate) const SCROLLBAR_TRACK_WIDTH: f32 = 11.0;

/// 滑块静置宽度(waku 是 5)。
const THUMB_WIDTH: f32 = 5.0;
/// 悬停/拖动时的宽度。
const THUMB_ACTIVE_WIDTH: f32 = 8.0;
/// 滑块相对轨道的内缩。
const THUMB_INSET: f32 = 2.0;
/// 滑块最短长度:太短就没法抓。
const THUMB_MIN_LENGTH: f32 = 28.0;
/// 滑块圆角(取足够大的值就是胶囊形)。
const THUMB_RADIUS: f32 = 999.0;
/// 轨道的宽度。
const TRACK_WIDTH: f32 = SCROLLBAR_TRACK_WIDTH;

/// 覆盖层滚动条。
///
/// `handle` 是它所覆盖的滚动容器自己的句柄——位置和可视高度都从那里读。
pub(crate) fn overlay_scrollbar<'a>(handle: &'a ScrollHandle, cx: &App) -> impl IntoElement + 'a {
    let theme = cx.theme();
    let thumb = theme.scrollbar_thumb;
    let thumb_hover = theme.scrollbar_thumb_hover;

    Scrollbar::vertical(handle)
        // 显式写出来:底座默认就是它,但这条注释帮下一个读者确认「静置会隐,不是 bug」。
        .mode(ScrollbarMode::Scrolling)
        .styles(move |styles: ScrollbarStyles| {
            styles
                // 轨道不画底:overlay 的观感来自滑块本身,加底色会变成一条常驻的竖线。
                .track(|style| style.width(px(TRACK_WIDTH)).bg(gpui::transparent_black()))
                .track_hover(|style| style.width(px(TRACK_WIDTH)).bg(gpui::transparent_black()))
                .track_active(|style| style.width(px(TRACK_WIDTH)).bg(gpui::transparent_black()))
                .thumb(|style| {
                    style
                        .width(px(THUMB_WIDTH))
                        .inset(px(THUMB_INSET))
                        .radius(px(THUMB_RADIUS))
                        .min_length(px(THUMB_MIN_LENGTH))
                        .bg(thumb)
                })
                .thumb_hover(|style| {
                    style
                        .width(px(THUMB_ACTIVE_WIDTH))
                        .radius(px(THUMB_RADIUS))
                        .bg(thumb_hover)
                })
                .thumb_active(|style| {
                    style
                        .width(px(THUMB_ACTIVE_WIDTH))
                        .radius(px(THUMB_RADIUS))
                        .bg(thumb_hover)
                })
        })
        .into_any_element()
}

/// 滚动条外层容器该用的宽度(`px`)。
pub(crate) fn track_extent() -> Pixels {
    px(SCROLLBAR_TRACK_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 度量只有一处定义:外层留位的宽度必须与轨道宽度一致,否则滑块贴边被裁。
    #[test]
    fn track_extent_matches_the_track_width() {
        assert_eq!(px(TRACK_WIDTH), track_extent());
        assert_eq!(pixels_to_f32(track_extent()), SCROLLBAR_TRACK_WIDTH);
    }

    fn pixels_to_f32(value: Pixels) -> f32 {
        f32::from(value)
    }
}
