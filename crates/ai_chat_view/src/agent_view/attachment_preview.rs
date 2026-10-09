//! 图片放大预览覆盖层。
//!
//! 覆盖层挂在**面板根部**，不在输入框组件自己的树里：输入框只占面板底部
//! 一条，绝对定位的子元素跳不出它的尺寸，画在那边只能得到一条被裁掉的
//! 预览。放在这里才有整块面板的背板。
//!
//! 关闭方式只做点按（背板 / 关闭按钮）：Escape 要在面板根上抢焦点或往
//! 全局 keymap 里加动作，那是 navop 主应用的键位表职责；为一个预览器
//! 改全局键位不划算。
//!
//! 两个入口共用同一套壳：composer 里的待发附件（[`AgentChatView::render_attachment_preview`]）
//! 与转录里的消息图（[`AgentChatView::render_message_image_preview`]）。
//! 它们的背板、尺寸上限、关闭按钮都该是同一份——差别只有 debug selector
//! 前缀，测试要能分辨自己在点哪一个。

use std::sync::Arc;

use gpui::{
    AnyElement, ClickEvent, Image, InteractiveElement, MouseButton, MouseDownEvent, ParentElement,
    SharedString, Styled, div, img, relative,
};
use one_assets::IconName;
use one_ui::{IconButton, IconButtonRole};

use super::*;

/// 预览图相对面板的尺寸上限。
///
/// 0.9 留出四周呼吸位：贴着边画的预览看起来像「图被裁了」。
const PREVIEW_MAX_FRACTION: f32 = 0.9;

/// 放大预览的通用壳。
///
/// `id_prefix` 同时决定三个 id 与 debug selector：`{prefix}-layer`（背板）、
/// `{prefix}`（内容面板）、`{prefix}-close`（关闭按钮）。
pub(super) fn image_preview_overlay(
    id_prefix: &'static str,
    image: Arc<Image>,
    name: Option<SharedString>,
    theme: &AgentChatTheme,
    close_backdrop: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    close_button: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let layer_id = SharedString::from(format!("{id_prefix}-layer"));
    let panel_id = SharedString::from(id_prefix);
    let close_id = SharedString::from(format!("{id_prefix}-close"));

    div()
        .id(layer_id)
        .debug_selector(move || format!("{id_prefix}-layer"))
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.overlay_strong)
        .on_mouse_down(MouseButton::Left, close_backdrop)
        .child(
            v_flex()
                .id(panel_id)
                .debug_selector(move || id_prefix.to_string())
                // `occlude` 让点图本身不再穿透到背板：看图不该把图关掉。
                .occlude()
                .relative()
                .max_w(relative(PREVIEW_MAX_FRACTION))
                .max_h(relative(PREVIEW_MAX_FRACTION))
                .p_2()
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(theme.border_strong)
                .bg(theme.overlay)
                .child(
                    // `Img` 默认 `ObjectFit::Contain`，配上下面的尺寸上限
                    // 就是「不超出、也不放大」。
                    img(image).max_w_full().max_h_full(),
                )
                .when_some(name.filter(|name| !name.is_empty()), |this, name| {
                    this.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .text_xs()
                            .text_color(theme.text_ghost)
                            .child(name),
                    )
                })
                .child(
                    div().absolute().top_1().right_1().child(
                        IconButton::new(close_id, IconName::Close)
                            .role(IconButtonRole::Compact)
                            .on_click(close_button),
                    ),
                ),
        )
        .into_any_element()
}

impl AgentChatView {
    /// 待发附件预览覆盖层；没有预览对象时返回 `None`。
    pub(super) fn render_attachment_preview(
        &self,
        theme: &AgentChatTheme,
        cx: &App,
    ) -> Option<AnyElement> {
        let composer = self.input.clone();
        let (image, name) = {
            let input = composer.read(cx);
            let attachment = input.previewed_attachment()?;
            (attachment.image.clone(), attachment.name.clone())
        };

        let backdrop = {
            let composer = composer.clone();
            move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                composer.update(cx, |input, cx| input.close_attachment_preview(cx));
            }
        };
        let button = {
            let composer = composer.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                composer.update(cx, |input, cx| input.close_attachment_preview(cx));
            }
        };

        Some(image_preview_overlay(
            "agent-attachment-preview",
            image,
            Some(name.into()),
            theme,
            backdrop,
            button,
        ))
    }
}
