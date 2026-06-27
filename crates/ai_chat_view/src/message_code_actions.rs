use crate::code_block::{CodeBlockActionRegistry, extract_fenced_code_blocks};
use crate::{ChatMessageUIGeneric, MessageExtension};
use gpui::{AnyElement, App, IntoElement, ParentElement, SharedString, Styled, div};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{ActiveTheme, Sizable, h_flex, v_flex};

pub(crate) fn render_code_block_actions<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    registry: &CodeBlockActionRegistry,
    cx: &App,
) -> Option<AnyElement> {
    if registry.is_empty() {
        return None;
    }
    let mut rows = Vec::new();
    for (idx, block) in extract_fenced_code_blocks(&msg.content)
        .into_iter()
        .enumerate()
    {
        let actions = registry.get_actions_for_lang(block.language.as_deref());
        if actions.is_empty() {
            continue;
        }
        let lang_label = block.language.clone().unwrap_or_else(|| "text".to_string());
        let mut row = h_flex()
            .w_full()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(cx.theme().muted.opacity(0.45))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("代码块 {} · {}", idx + 1, lang_label)),
            )
            .child(div().flex_1());
        for action in actions {
            let code = block.code.clone();
            let lang = block.language.clone();
            let callback = action.callback.clone();
            let mut button = Button::new(SharedString::from(format!(
                "code-action-{}-{}",
                idx, action.id
            )))
            .icon(action.icon.clone())
            .ghost()
            .xsmall()
            .on_click(move |_, window, cx| {
                (callback)(code.clone(), lang.clone(), window, cx);
            });
            if let Some(label) = &action.label {
                button = button.label(label.clone());
            }
            row = row.child(button);
        }
        rows.push(row.into_any_element());
    }
    (!rows.is_empty()).then(|| v_flex().w_full().gap_1().children(rows).into_any_element())
}
