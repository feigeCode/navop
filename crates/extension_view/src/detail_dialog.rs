//! 扩展详情弹窗：元信息分组展示 + 截图缩略图网格（hover 放大预览）+ 整页滚动。

use gpui::{
    App, AppContext, Context, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Styled, StyledImage, StatefulInteractiveElement, Window,
    div, img, px, rems,
};
use gpui_component::{ActiveTheme, Icon, Sizable, h_flex, scroll::ScrollableElement, v_flex};
use one_ui::IconSize;
use rust_i18n::t;

use crate::{
    ExtensionKind, MarketplaceEntry,
    cards::{kind_color, kind_icon, kind_label},
};

/// 详情弹窗尺寸。
const DIALOG_WIDTH: f32 = 720.0;
const DIALOG_HEIGHT: f32 = 620.0;
/// 缩略图网格：每行张数与格子高度。
const THUMB_COLUMNS: usize = 3;
const THUMB_HEIGHT_PX: f32 = 108.0;
/// 放大预览的最小高度（rem），flex 弹性时防止被压缩消失。
const PREVIEW_MIN_HEIGHT_REMS: f32 = 10.0;
const GALLERY_ROUNDED_PX: f32 = 10.0;
/// 元信息标签列宽。
const META_LABEL_WIDTH_PX: f32 = 96.0;
/// 弹窗底部留白。
const DIALOG_BOTTOM_PADDING_PX: f32 = 12.0;

/// 打开扩展详情弹窗。
///
/// 必须传入触发点击的真实父窗口：否则回退链路依赖 `active_window()`（macOS
/// mainWindow），多屏下可能解析到其他屏幕的窗口，导致弹窗跨屏。
pub(super) fn show_detail_dialog(
    entry: MarketplaceEntry,
    window: &mut Window,
    cx: &mut App,
) {
    let options = one_core::popup_window::PopupWindowOptions::new(t!("Extension.detail_title"))
        .size(DIALOG_WIDTH, DIALOG_HEIGHT);
    one_core::popup_window::open_popup_window(
        options,
        move |_window, cx| cx.new(|cx| ExtensionDetailView::new(entry.clone(), cx)),
        Some(window),
        cx,
    );
}

struct ExtensionDetailView {
    entry: MarketplaceEntry,
    /// hover 中的截图索引（放大预览用）。
    hovered_screenshot: Option<usize>,
    focus_handle: FocusHandle,
}

/// 轮播索引循环切换：到头后回到另一端（键盘左右键切换预览时复用）。
#[cfg(test)]
pub(crate) fn shift_index(current: usize, delta: i32, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    (current as i32 + delta).rem_euclid(count as i32) as usize
}

impl ExtensionDetailView {
    fn new(entry: MarketplaceEntry, cx: &mut Context<Self>) -> Self {
        Self {
            entry,
            hovered_screenshot: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// hover 进入 / 离开缩略图时切换放大预览。
    fn hover_screenshot(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        if self.hovered_screenshot == index {
            return;
        }
        self.hovered_screenshot = index;
        cx.notify();
    }

    fn render_header(&self, fg: gpui::Hsla, muted: gpui::Hsla, cx: &Context<Self>) -> impl IntoElement {
        let entry = &self.entry;
        h_flex()
            .gap_3()
            .items_center()
            .child(detail_icon_tile(entry.kind, cx))
            .child(
                v_flex()
                    .gap_1()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_baseline()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(fg)
                                    .truncate()
                                    .child(entry.name.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(format!("v{}", entry.version)),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(kind_label(entry.kind)),
                    ),
            )
    }

    fn render_section_title(&self, title: SharedString, fg: gpui::Hsla) -> impl IntoElement {
        div()
            .text_sm()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(fg)
            .child(title)
    }

    fn render_meta_row(
        &self,
        label: SharedString,
        value: String,
        muted: gpui::Hsla,
    ) -> impl IntoElement {
        h_flex()
            .gap_3()
            .items_baseline()
            .child(
                div()
                    .flex_none()
                    .w(px(META_LABEL_WIDTH_PX))
                    .text_xs()
                    .text_color(muted)
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .child(value),
            )
    }

    /// 完整元信息：所有可展示字段，缺失的跳过。
    fn render_meta(&self, muted: gpui::Hsla, cx: &Context<Self>) -> impl IntoElement {
        let entry = &self.entry;
        let mut rows = vec![
            self.render_meta_row(t!("Extension.detail_id").into(), entry.id.clone(), muted),
            self.render_meta_row(
                t!("Extension.detail_kind").into(),
                kind_label(entry.kind),
                muted,
            ),
        ];
        if !entry.version.is_empty() {
            rows.push(self.render_meta_row(
                t!("Extension.detail_version").into(),
                entry.version.clone(),
                muted,
            ));
        }
        if !entry.file_extensions.is_empty() {
            rows.push(self.render_meta_row(
                t!("Extension.detail_file_extensions").into(),
                entry.file_extensions.join(", "),
                muted,
            ));
        }
        if let Some(required) = entry
            .required_host_version
            .as_deref()
            .filter(|v| !v.trim().is_empty())
        {
            rows.push(self.render_meta_row(
                t!("Extension.detail_requires_host").into(),
                format!("Navop >= {required}"),
                muted,
            ));
        }
        if !entry.asset_url.is_empty() {
            rows.push(self.render_meta_row(
                t!("Extension.detail_download").into(),
                entry.asset_url.clone(),
                muted,
            ));
        }
        if let Some(sha) = entry.sha256.as_deref().filter(|s| !s.is_empty()) {
            rows.push(self.render_meta_row("SHA256".into(), sha.to_string(), muted));
        }
        let _ = cx;
        v_flex().gap_2p5().children(rows)
    }

    /// 底部截图区：预览占满弹窗剩余高度，hover 缩略图切换预览内容。
    fn render_gallery(&self, muted: gpui::Hsla, cx: &Context<Self>) -> impl IntoElement {
        let screenshots = &self.entry.screenshots;
        if screenshots.is_empty() {
            return v_flex()
                .w_full()
                .flex_1()
                .min_h_0()
                .rounded(px(GALLERY_ROUNDED_PX))
                .border_1()
                .border_dashed()
                .border_color(cx.theme().border)
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(t!("Extension.detail_no_screenshots").to_string()),
                )
                .into_any_element();
        }

        v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(
                // 放大预览：flex 弹性占满剩余高度，图片 Contain 不裁切。
                div()
                    .id("ext-detail-preview")
                    .w_full()
                    .flex_1()
                    .min_h(rems(PREVIEW_MIN_HEIGHT_REMS))
                    .rounded(px(GALLERY_ROUNDED_PX))
                    .overflow_hidden()
                    .bg(cx.theme().muted.opacity(0.35))
                    .child(
                        img(self.preview_source().unwrap_or_default())
                            .size_full()
                            .object_fit(gpui::ObjectFit::Contain),
                    ),
            )
            .child(
                // 缩略图网格：圆角小图，hover 高亮并切换预览；固定高度不参与弹性。
                v_flex()
                    .flex_none()
                    .w_full()
                    .gap_2()
                    .children(screenshots.chunks(THUMB_COLUMNS).enumerate().map(
                        |(row, chunk)| {
                            h_flex().gap_2().children(
                                chunk.iter().enumerate().map(|(column, source)| {
                                    let index = row * THUMB_COLUMNS + column;
                                    self.render_thumbnail(source, index, muted, cx)
                                }),
                            )
                        },
                    )),
            )
            .into_any_element()
    }

    /// 当前预览图源：hover 中的缩略图，无 hover 时默认第一张。
    fn preview_source(&self) -> Option<String> {
        let screenshots = &self.entry.screenshots;
        self.hovered_screenshot
            .and_then(|index| screenshots.get(index))
            .or_else(|| screenshots.first())
            .cloned()
    }

    fn render_thumbnail(
        &self,
        source: &str,
        index: usize,
        muted: gpui::Hsla,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let border = cx.theme().border;
        let highlight = cx.theme().primary.opacity(0.6);
        let is_hovered = self.hovered_screenshot == Some(index);
        div()
            .id(format!("ext-detail-thumb-{index}"))
            .flex_1()
            .min_w_0()
            .h(px(THUMB_HEIGHT_PX))
            .rounded(px(GALLERY_ROUNDED_PX))
            .overflow_hidden()
            .border_1()
            .border_color(if is_hovered { highlight } else { border })
            .bg(muted.opacity(0.25))
            .cursor_pointer()
            .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
                view.hover_screenshot(hovered.then_some(index), cx);
            }))
            .child(img(source.to_string()).size_full().object_fit(gpui::ObjectFit::Cover))
    }
}

impl Focusable for ExtensionDetailView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ExtensionDetailView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fg = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;

        // 布局：上半部分（头部/描述/元信息）固定内容高度内滚动，
        // 下半部分截图区 flex_1 占满弹窗剩余高度，底部留白。
        v_flex()
            .size_full()
            .gap_4()
            .px_5()
            .pt_4()
            .pb(px(DIALOG_BOTTOM_PADDING_PX))
            .overflow_hidden()
            .child(
                // 上半部分：内容超高时内部滚动，不挤压图片区。
                v_flex()
                    .w_full()
                    .flex_none()
                    .max_h(rems(28.0))
                    .min_h_0()
                    .gap_4()
                    .overflow_y_scrollbar()
                    .child(self.render_header(fg, muted, cx))
                    .child(
                        div()
                            .text_sm()
                            .line_height(rems(1.5))
                            .text_color(muted)
                            .child(if self.entry.description.trim().is_empty() {
                                t!("Extension.no_description").to_string()
                            } else {
                                self.entry.description.clone()
                            }),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .pt_2()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .child(self.render_section_title(
                                t!("Extension.detail_info").into(),
                                fg,
                            ))
                            .child(self.render_meta(muted, cx)),
                    ),
            )
            .child(
                // 下半部分：截图区标题 + 占满剩余高度的预览。
                v_flex()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .gap_2()
                    .child(self.render_section_title(
                        t!("Extension.detail_screenshots").into(),
                        fg,
                    ))
                    .child(self.render_gallery(muted, cx)),
            )
    }
}

/// 详情图标底座：与卡片同款品牌色浅底。
fn detail_icon_tile(kind: ExtensionKind, cx: &App) -> impl IntoElement {
    let color = kind_color(kind);
    let tint = if cx.theme().is_dark() {
        color.scale(950).opacity(0.5)
    } else {
        color.scale(50)
    };
    div()
        .size(rems(2.5))
        .rounded(px(10.0))
        .bg(tint)
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(kind_icon(kind)).with_size(IconSize::Medium).color())
}

#[cfg(test)]
mod tests {
    use super::shift_index;

    #[test]
    fn shift_index_wraps_in_both_directions() {
        // 3 张截图：末尾右切回到 0，开头左切回到末尾。
        assert_eq!(0, shift_index(2, 1, 3));
        assert_eq!(2, shift_index(0, -1, 3));
        assert_eq!(1, shift_index(0, 1, 3));
    }

    #[test]
    fn shift_index_single_screenshot_stays_put() {
        assert_eq!(0, shift_index(0, 1, 1));
        assert_eq!(0, shift_index(0, -1, 1));
    }

    #[test]
    fn shift_index_zero_count_is_safe() {
        assert_eq!(0, shift_index(3, 1, 0));
    }
}
