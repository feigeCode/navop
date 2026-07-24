use std::{rc::Rc, sync::Arc};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, DefiniteLength, Edges, EdgesRefinement, Entity, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Rems, RenderOnce,
    StyleRefinement, Styled, TextAlign, Window, div, px, relative,
};

use crate::button::{Button, ButtonVariants as _};
use crate::highlighter::HighlightTheme;
use crate::input::clear_button;
use crate::menu::PopupMenu;
use crate::spinner::Spinner;
use crate::{ActiveTheme, Colorize, v_flex};
use crate::{IconName, Size};
use crate::{Selectable, StyledExt, h_flex};
use crate::{Sizable, StyleSized};

use super::{InputState, element::EditorScrollbar};

/// Local colors for input-like controls embedded in a differently themed panel.
#[derive(Clone, Copy)]
pub struct LocalInputStyle {
    pub background: Hsla,
    pub foreground: Hsla,
    pub muted_foreground: Hsla,
    pub border: Hsla,
}

/// Returns `(background, foreground)` colors for input-like components.
pub(crate) fn input_style(disabled: bool, cx: &App) -> (Hsla, Hsla) {
    if disabled {
        (
            cx.theme().input.mix_oklab(cx.theme().transparent, 0.8),
            cx.theme().muted_foreground,
        )
    } else {
        (cx.theme().input_background(), cx.theme().foreground)
    }
}

fn resolved_input_colors(
    local_style: Option<LocalInputStyle>,
    disabled: bool,
    code_editor: bool,
    fallback: (Hsla, Hsla),
    editor_background: Hsla,
) -> (Hsla, Hsla) {
    let (background, foreground) = if disabled && !code_editor {
        fallback
    } else {
        local_style.map_or(fallback, |style| (style.background, style.foreground))
    };
    let background = if code_editor {
        local_style.map_or(editor_background, |style| style.background)
    } else {
        background
    };
    (background, foreground)
}

fn should_dim_input(disabled: bool, code_editor: bool) -> bool {
    disabled && !code_editor
}

fn should_handle_vertical_navigation(is_multi_line: bool, context_menu_open: bool) -> bool {
    is_multi_line || context_menu_open
}

/// A text input element bind to an [`InputState`].
#[derive(IntoElement)]
pub struct Input {
    state: Entity<InputState>,
    style: StyleRefinement,
    size: Size,
    prefix: Option<AnyElement>,
    suffix: Option<AnyElement>,
    height: Option<DefiniteLength>,
    appearance: bool,
    cleanable: bool,
    mask_toggle: bool,
    disabled: bool,
    read_only: bool,
    bordered: bool,
    focus_bordered: bool,
    caret_color: Option<Hsla>,
    local_style: Option<LocalInputStyle>,
    highlight_theme: Option<Arc<HighlightTheme>>,
    indent_guide_color: Option<Hsla>,
    tab_index: isize,
    selected: bool,
    bare: bool,
    editor_scrollbar: bool,
    text_layout_margin: bool,

    /// An optional context menu builder to allow a custom context menu on the input.
    ///
    /// If set, this will override the built-in context menu.
    context_menu_builder:
        Option<Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>>,
}

impl Sizable for Input {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl Selectable for Input {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl Input {
    /// Create a new [`Input`] element bind to the [`InputState`].
    pub fn new(state: &Entity<InputState>) -> Self {
        Self {
            state: state.clone(),
            size: Size::default(),
            style: StyleRefinement::default(),
            prefix: None,
            suffix: None,
            height: None,
            appearance: true,
            cleanable: false,
            mask_toggle: false,
            disabled: false,
            read_only: false,
            bordered: true,
            focus_bordered: true,
            caret_color: None,
            local_style: None,
            highlight_theme: None,
            indent_guide_color: None,
            tab_index: 0,
            selected: false,
            bare: false,
            editor_scrollbar: true,
            text_layout_margin: true,
            context_menu_builder: None,
        }
    }

    pub fn prefix(mut self, prefix: impl IntoElement) -> Self {
        self.prefix = Some(prefix.into_any_element());
        self
    }

    pub fn suffix(mut self, suffix: impl IntoElement) -> Self {
        self.suffix = Some(suffix.into_any_element());
        self
    }

    /// Set full height of the input (Multi-line only).
    pub fn h_full(mut self) -> Self {
        self.height = Some(relative(1.));
        self
    }

    /// Set height of the input (Multi-line only).
    pub fn h(mut self, height: impl Into<DefiniteLength>) -> Self {
        self.height = Some(height.into());
        self
    }

    /// Set the appearance of the input field, if false the input field will no border, background.
    pub fn appearance(mut self, appearance: bool) -> Self {
        self.appearance = appearance;
        self
    }

    /// Set the bordered for the input, default: true
    pub fn bordered(mut self, bordered: bool) -> Self {
        self.bordered = bordered;
        self
    }

    /// Set focus border for the input, default is true.
    pub fn focus_bordered(mut self, bordered: bool) -> Self {
        self.focus_bordered = bordered;
        self
    }

    /// Override the blinking caret color for locally themed embedded inputs.
    pub fn caret_color(mut self, color: Hsla) -> Self {
        self.caret_color = Some(color);
        self
    }

    /// Override input colors for a locally themed embedded panel.
    pub fn local_style(mut self, style: LocalInputStyle) -> Self {
        self.local_style = Some(style);
        self
    }

    /// Override syntax and editor decoration colors for an embedded code editor.
    pub fn highlight_theme(mut self, theme: Arc<HighlightTheme>) -> Self {
        self.highlight_theme = Some(theme);
        self
    }

    /// Override the indent-guide color for an embedded code editor.
    pub fn indent_guide_color(mut self, color: Hsla) -> Self {
        self.indent_guide_color = Some(color);
        self
    }

    /// Set whether to show the scrollbar embedded in a multi-line editor.
    pub fn editor_scrollbar(mut self, visible: bool) -> Self {
        self.editor_scrollbar = visible;
        self
    }

    /// Set whether the text layout reserves the editor's trailing safety margin.
    pub fn text_layout_margin(mut self, enabled: bool) -> Self {
        self.text_layout_margin = enabled;
        self
    }

    /// Set whether to show the clear button when the input field is not empty, default is false.
    pub fn cleanable(mut self, cleanable: bool) -> Self {
        self.cleanable = cleanable;
        self
    }

    /// Set to enable toggle button for password mask state.
    pub fn mask_toggle(mut self) -> Self {
        self.mask_toggle = true;
        self
    }

    /// Set to disable the input field.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set the input to read-only while keeping selection and copy available.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Set the tab index for the input, default is 0.
    pub fn tab_index(mut self, index: isize) -> Self {
        self.tab_index = index;
        self
    }

    /// Pure editor mode: remove Input-owned padding, height and vertical centering.
    pub fn bare(mut self) -> Self {
        self.bare = true;
        self
    }

    /// Sets the context menu for the input.
    pub fn context_menu(
        mut self,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        self.context_menu_builder = Some(Rc::new(f));
        self
    }

    fn render_toggle_mask_button(state: &Entity<InputState>, cx: &App) -> impl IntoElement {
        let masked = state.read(cx).masked;
        Button::new("toggle-mask")
            .icon(if masked {
                IconName::Eye
            } else {
                IconName::EyeOff
            })
            .xsmall()
            .ghost()
            .tab_stop(false)
            .on_click({
                let state = state.clone();
                move |_, window, cx| {
                    state.update(cx, |state, cx| {
                        state.set_masked(!state.masked, window, cx);
                    })
                }
            })
    }

    /// This method must after the refine_style.
    fn render_editor(
        paddings: EdgesRefinement<DefiniteLength>,
        input_state: &Entity<InputState>,
        state: &InputState,
        editor_scrollbar: bool,
        window: &Window,
    ) -> impl IntoElement {
        let base_size = window.text_style().font_size;
        let rem_size = window.rem_size();

        let paddings = Edges {
            left: paddings
                .left
                .map(|v| v.to_pixels(base_size, rem_size))
                .unwrap_or(px(0.)),
            right: paddings
                .right
                .map(|v| v.to_pixels(base_size, rem_size))
                .unwrap_or(px(0.)),
            top: paddings
                .top
                .map(|v| v.to_pixels(base_size, rem_size))
                .unwrap_or(px(0.)),
            bottom: paddings
                .bottom
                .map(|v| v.to_pixels(base_size, rem_size))
                .unwrap_or(px(0.)),
        };

        state.editor_scrollbar_paddings.set(paddings);
        state.editor_scrollbar_snapshot.set(None);

        v_flex()
            .size_full()
            .min_w_0()
            .children(state.search_panel.clone())
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .w_full()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .w_full()
                            .min_w_0()
                            .child(input_state.clone()),
                    )
                    .when(editor_scrollbar, |this| {
                        this.child(EditorScrollbar::new(input_state.clone()))
                    }),
            )
    }
}

impl Styled for Input {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Input {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        const LINE_HEIGHT: Rems = Rems(1.25);
        let text_align = self.style.text.text_align.unwrap_or(TextAlign::Left);

        self.state.update(cx, |state, _| {
            state.context_menu_builder = self.context_menu_builder.clone();
            state.disabled = self.disabled;
            state.read_only = self.read_only;
            state.size = self.size;
            state.caret_color = self.caret_color;
            state.placeholder_color = self.local_style.map(|style| style.muted_foreground);
            state.background_color = self.local_style.map(|style| style.background);
            state.highlight_theme = self.highlight_theme.clone();
            state.indent_guide_color = self.indent_guide_color;
            state.text_layout_margin = self.text_layout_margin;

            state.text_align = text_align;
        });

        let state = self.state.read(cx);
        let focused = state.focus_handle.is_focused(window) && !state.disabled;
        let handle_vertical_navigation = should_handle_vertical_navigation(
            state.mode.is_multi_line(),
            state.is_context_menu_open(cx),
        );
        let gap_x = match self.size {
            Size::Small => px(4.),
            Size::Large => px(8.),
            _ => px(6.),
        };

        let code_editor = state.mode.is_code_editor();
        let (bg, fg) = resolved_input_colors(
            self.local_style,
            state.disabled,
            code_editor,
            input_style(state.disabled, cx),
            cx.theme().editor_background(),
        );
        let border_color = self
            .local_style
            .filter(|_| !state.disabled || code_editor)
            .map_or(cx.theme().input, |style| style.border);

        let prefix = self.prefix;
        let suffix = self.suffix;
        let show_clear_button = self.cleanable
            && !state.disabled
            && !state.loading
            && state.text.len() > 0
            && state.mode.is_single_line();
        let has_suffix = suffix.is_some() || state.loading || self.mask_toggle || show_clear_button;

        div()
            .id(("input", self.state.entity_id()))
            .flex()
            .key_context(crate::input::CONTEXT)
            .track_focus(&state.focus_handle.clone())
            .tab_index(self.tab_index)
            .when(!state.disabled && !state.read_only, |this| {
                this.on_action(window.listener_for(&self.state, InputState::backspace))
                    .on_action(window.listener_for(&self.state, InputState::delete))
                    .on_action(
                        window.listener_for(&self.state, InputState::delete_to_beginning_of_line),
                    )
                    .on_action(window.listener_for(&self.state, InputState::delete_to_end_of_line))
                    .on_action(window.listener_for(&self.state, InputState::delete_previous_word))
                    .on_action(window.listener_for(&self.state, InputState::delete_next_word))
                    .on_action(window.listener_for(&self.state, InputState::enter))
                    .on_action(window.listener_for(&self.state, InputState::escape))
                    .on_action(window.listener_for(&self.state, InputState::paste))
                    .on_action(window.listener_for(&self.state, InputState::cut))
                    .on_action(window.listener_for(&self.state, InputState::undo))
                    .on_action(window.listener_for(&self.state, InputState::redo))
                    .when(state.mode.is_multi_line(), |this| {
                        this.on_action(window.listener_for(&self.state, InputState::indent_inline))
                            .on_action(window.listener_for(&self.state, InputState::outdent_inline))
                            .on_action(window.listener_for(&self.state, InputState::indent_block))
                            .on_action(window.listener_for(&self.state, InputState::outdent_block))
                    })
                    .on_action(
                        window.listener_for(&self.state, InputState::on_action_toggle_code_actions),
                    )
            })
            .on_action(window.listener_for(&self.state, InputState::left))
            .on_action(window.listener_for(&self.state, InputState::right))
            .on_action(window.listener_for(&self.state, InputState::select_left))
            .on_action(window.listener_for(&self.state, InputState::select_right))
            .when(handle_vertical_navigation, |this| {
                this.on_action(window.listener_for(&self.state, InputState::up))
                    .on_action(window.listener_for(&self.state, InputState::down))
            })
            .when(state.mode.is_multi_line(), |this| {
                this.on_action(window.listener_for(&self.state, InputState::select_up))
                    .on_action(window.listener_for(&self.state, InputState::select_down))
                    .on_action(window.listener_for(&self.state, InputState::page_up))
                    .on_action(window.listener_for(&self.state, InputState::page_down))
                    .on_action(
                        window.listener_for(&self.state, InputState::on_action_go_to_definition),
                    )
            })
            .on_action(window.listener_for(&self.state, InputState::select_all))
            .on_action(window.listener_for(&self.state, InputState::select_to_start_of_line))
            .on_action(window.listener_for(&self.state, InputState::select_to_end_of_line))
            .on_action(window.listener_for(&self.state, InputState::select_to_previous_word))
            .on_action(window.listener_for(&self.state, InputState::select_to_next_word))
            .on_action(window.listener_for(&self.state, InputState::home))
            .on_action(window.listener_for(&self.state, InputState::end))
            .on_action(window.listener_for(&self.state, InputState::move_to_start))
            .on_action(window.listener_for(&self.state, InputState::move_to_end))
            .on_action(window.listener_for(&self.state, InputState::move_to_previous_word))
            .on_action(window.listener_for(&self.state, InputState::move_to_next_word))
            .on_action(window.listener_for(&self.state, InputState::select_to_start))
            .on_action(window.listener_for(&self.state, InputState::select_to_end))
            .on_action(window.listener_for(&self.state, InputState::show_character_palette))
            .on_action(window.listener_for(&self.state, InputState::copy))
            .on_action(window.listener_for(&self.state, InputState::on_action_search))
            .on_key_down(window.listener_for(&self.state, InputState::on_key_down))
            .on_mouse_down(
                MouseButton::Left,
                window.listener_for(&self.state, InputState::on_mouse_down),
            )
            .on_mouse_down(
                MouseButton::Right,
                window.listener_for(&self.state, InputState::on_mouse_down),
            )
            .on_mouse_up(
                MouseButton::Left,
                window.listener_for(&self.state, InputState::on_mouse_up),
            )
            .on_mouse_up(
                MouseButton::Right,
                window.listener_for(&self.state, InputState::on_mouse_up),
            )
            .on_mouse_move(window.listener_for(&self.state, InputState::on_mouse_move))
            .on_scroll_wheel(window.listener_for(&self.state, InputState::on_scroll_wheel))
            .size_full()
            .min_w_0()
            .when(!self.bare, |this| this.line_height(LINE_HEIGHT))
            .when(!self.bare, |this| this.input_px(self.size))
            .when(!self.bare, |this| this.input_py(self.size))
            .when(!self.bare, |this| this.input_h(self.size))
            .input_text_size(self.size)
            .when(!self.disabled && !self.read_only, |this| this.cursor_text())
            .when(!self.bare, |this| this.items_center())
            .when(state.mode.is_multi_line() && !self.bare, |this| {
                this.h_auto()
                    .when_some(self.height, |this, height| this.h(height))
            })
            .when(self.appearance, |this| {
                this.bg(bg)
                    .text_color(fg)
                    .when(should_dim_input(self.disabled, code_editor), |this| {
                        this.opacity(0.5)
                    })
                    .rounded(cx.theme().radius)
                    .when(self.bordered, |this| {
                        this.border_color(border_color)
                            .border_1()
                            .when(cx.theme().shadow, |this| this.shadow_xs())
                            .when(focused && self.focus_bordered, |this| {
                                this.focused_border(cx)
                            })
                    })
            })
            .when(!self.bare, |this| this.items_center())
            .gap(gap_x)
            .refine_style(&self.style)
            .children(prefix)
            .when(state.mode.is_multi_line(), |mut this| {
                let paddings = this.style().padding.clone();
                this.child(Self::render_editor(
                    paddings,
                    &self.state,
                    &state,
                    self.editor_scrollbar,
                    window,
                ))
            })
            .when(!state.mode.is_multi_line(), |this| {
                this.child(self.state.clone())
            })
            .when(has_suffix, |this| {
                this.pr(self.size.input_px()).child(
                    h_flex()
                        .id("suffix")
                        .gap(gap_x)
                        .items_center()
                        .when(state.loading, |this| {
                            this.child(Spinner::new().color(cx.theme().muted_foreground))
                        })
                        .when(self.mask_toggle, |this| {
                            this.child(Self::render_toggle_mask_button(&self.state, cx))
                        })
                        .when(show_clear_button, |this| {
                            this.child(clear_button(cx).on_click({
                                let state = self.state.clone();
                                move |_, window, cx| {
                                    state.update(cx, |state, cx| {
                                        state.clean(window, cx);
                                        state.focus(window, cx);
                                    })
                                }
                            }))
                        })
                        .children(suffix),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color(value: u32) -> Hsla {
        gpui::rgb(value).into()
    }

    fn local_style() -> LocalInputStyle {
        LocalInputStyle {
            background: color(0x111111),
            foreground: color(0xeeeeee),
            muted_foreground: color(0x888888),
            border: color(0x333333),
        }
    }

    #[test]
    fn local_code_editor_uses_local_background() {
        let style = local_style();
        let colors = resolved_input_colors(
            Some(style),
            false,
            true,
            (color(0xaaaaaa), color(0xbbbbbb)),
            color(0xffffff),
        );

        assert_eq!(colors, (style.background, style.foreground));
    }

    #[test]
    fn local_code_editor_colors_survive_read_only_state() {
        let style = local_style();
        let colors = resolved_input_colors(
            Some(style),
            true,
            true,
            (color(0xaaaaaa), color(0xbbbbbb)),
            color(0xffffff),
        );

        assert_eq!(colors, (style.background, style.foreground));
        assert!(!should_dim_input(true, true));
    }

    #[test]
    fn disabled_regular_input_keeps_disabled_treatment() {
        let fallback = (color(0xaaaaaa), color(0xbbbbbb));
        let colors =
            resolved_input_colors(Some(local_style()), true, false, fallback, color(0xffffff));

        assert_eq!(colors, fallback);
        assert!(should_dim_input(true, false));
    }
}
