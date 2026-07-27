use super::MarkdownEditor;
use gpui::{
    Anchor, Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, px,
};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    highlighter::LanguageRegistry,
    menu::{DropdownMenu, PopupMenuItem},
};
use markdown_source::{SourceBlock, SourceBlockKind};

pub(super) const CODE_LANGUAGE_HEADER_HEIGHT: f32 = 28.;
const CODE_LANGUAGE_MENU_MAX_HEIGHT: f32 = 240.;

impl MarkdownEditor {
    pub(super) fn render_code_language_header(
        &self,
        block: &SourceBlock,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let current = code_language(block)?;
        let selected = resolved_language(&current);
        let options = language_options(&selected);
        let editor = cx.entity();
        let block_id = block.id;
        let button = Button::new(("markdown-code-language-button", block_id.0))
            .debug_selector(move || format!("markdown-code-language-{}", block_id.0))
            .label(current)
            .dropdown_caret(true)
            .ghost()
            .small()
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                build_language_menu(menu, &options, &selected, block_id, &editor)
            });

        Some(
            h_flex()
                .w_full()
                .h(px(CODE_LANGUAGE_HEADER_HEIGHT))
                .min_h(px(CODE_LANGUAGE_HEADER_HEIGHT))
                .flex_shrink_0()
                .justify_end()
                .child(button)
                .into_any_element(),
        )
    }
}

fn build_language_menu(
    mut menu: gpui_component::menu::PopupMenu,
    options: &[SharedString],
    selected: &str,
    block_id: markdown_source::SourceNodeId,
    editor: &gpui::Entity<MarkdownEditor>,
) -> gpui_component::menu::PopupMenu {
    for language in options {
        let checked = language.eq_ignore_ascii_case(selected);
        let label = language.clone();
        let selector = language_option_selector(language);
        let next_language = language.clone();
        let editor = editor.clone();
        menu = menu.item(
            PopupMenuItem::element(move |_, _| {
                gpui::div()
                    .debug_selector(|| selector.clone())
                    .w_full()
                    .child(label.clone())
            })
            .checked(checked)
            .on_click(move |_, window, cx| {
                if checked {
                    return;
                }
                editor.update(cx, |editor, cx| {
                    let _ = editor.set_code_fence_language(
                        block_id,
                        next_language.as_ref(),
                        window,
                        cx,
                    );
                });
            }),
        );
    }
    menu.max_h(px(CODE_LANGUAGE_MENU_MAX_HEIGHT))
        .scrollable(true)
}

fn language_option_selector(language: &str) -> String {
    let language = language
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("markdown-code-language-option-{language}")
}

fn language_options(selected: &str) -> Vec<SharedString> {
    let mut languages = LanguageRegistry::singleton().languages();
    if !languages
        .iter()
        .any(|language| language.eq_ignore_ascii_case(selected))
    {
        languages.push(SharedString::from(selected.to_owned()));
        languages.sort();
    }
    languages
}

fn resolved_language(language: &str) -> String {
    LanguageRegistry::singleton()
        .resolve_language_name(language)
        .unwrap_or_else(|| language.to_owned())
}

fn code_language(block: &SourceBlock) -> Option<String> {
    let SourceBlockKind::CodeFence { language_range, .. } = &block.kind else {
        return None;
    };
    let language = language_range.as_ref().and_then(|range| {
        let start = range.start.checked_sub(block.source_range.start)?;
        let end = range.end.checked_sub(block.source_range.start)?;
        block.original_source.get(start..end)
    });
    Some(
        language
            .filter(|value| !value.is_empty())
            .unwrap_or("text")
            .to_owned(),
    )
}
