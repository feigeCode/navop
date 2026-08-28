//! Signature help popover: renders a function call's signatures with the
//! active parameter highlighted, and lets the user cycle overloads.

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Div, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Window, div, px,
};
use gpui::prelude::FluentBuilder as _;
use gpui::prelude::InteractiveElement as _;
use gpui::prelude::StatefulInteractiveElement as _;
use lsp_types::SignatureHelp;

use crate::{
    ActiveTheme,
    input::{InputState, popovers::Popover, popovers::PopoverDismiss},
    {h_flex, v_flex},
};

pub struct SignatureHelpPopover {
    editor: Entity<InputState>,
    /// Cursor offset the help was requested for (anchor point).
    pub(crate) anchored_at: usize,
    help: Rc<SignatureHelp>,
    /// Highlighted overload index (overload cycling).
    active_signature: usize,
}

impl SignatureHelpPopover {
    pub fn new(
        editor: Entity<InputState>,
        anchored_at: usize,
        help: SignatureHelp,
        cx: &mut App,
    ) -> Entity<Self> {
        let active_signature = cycle_overload(
            help.signatures.len(),
            help.active_signature.unwrap_or(0) as usize,
            0,
        );
        cx.new(|_| Self {
            editor,
            anchored_at,
            help: Rc::new(help),
            active_signature,
        })
    }

    /// Cycle the highlighted overload within `[-1, 1]` deltas.
    pub fn cycle_signature(&mut self, delta: isize) {
        self.active_signature =
            cycle_overload(self.help.signatures.len(), self.active_signature, delta);
    }
}

/// Step the highlighted overload within `[0, len)`, wrapping at both ends.
///
/// A single overload is the identity.
fn cycle_overload(len: usize, current: usize, delta: isize) -> usize {
    if len <= 1 {
        return 0;
    }
    let current = current % len;
    let step = delta.rem_euclid(len as isize) as usize;
    (current + step) % len
}

impl Render for SignatureHelpPopover {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let active = self
            .active_signature
            .min(self.help.signatures.len().saturating_sub(1));
        let has_overloads = self.help.signatures.len() > 1;
        let active_parameter = self.help.active_parameter.unwrap_or(0) as usize;
        let signature = &self.help.signatures[active];

        let anchor = self.anchored_at..self.anchored_at;

        let label = signature.label.clone();
        let parameters = signature
            .parameters
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|parameter| parameter_label(&signature.label, &parameter.label))
            .collect::<Vec<_>>();
        let popover_entity = cx.entity();
        let total = self.help.signatures.len();
        let active_index = self.active_signature.min(total.saturating_sub(1));

        let popover = Popover::new("signature-help-popover", self.editor.clone(), anchor, {
            move |window, cx| {
                render_signature_card(
                    window,
                    cx,
                    &SignatureCardContent {
                        label: label.clone(),
                        parameters: parameters.clone(),
                        active_parameter,
                        has_overloads,
                        popover: popover_entity.clone(),
                        active_index,
                        total,
                    },
                )
            }
        })
        .dismiss(PopoverDismiss::Persistent);
        popover.into_any_element()
    }
}

/// Extract the parameter's text from its label (simple string or offsets into
/// the signature label).
fn parameter_label(label: &str, parameter: &lsp_types::ParameterLabel) -> String {
    match parameter {
        lsp_types::ParameterLabel::Simple(simple) => simple.clone(),
        lsp_types::ParameterLabel::LabelOffsets([start, end]) => label
            .get(*start as usize..*end as usize)
            .unwrap_or_default()
            .to_string(),
    }
}

/// A small clickable arrow that cycles the highlighted overload.
fn overload_button(
    delta: isize,
    entity: Entity<SignatureHelpPopover>,
) -> impl IntoElement {
    let glyph = if delta < 0 { "‹" } else { "›" };
    div()
        .id(gpui::ElementId::Name(SharedString::from(format!(
            "sig-{glyph}"
        ))))
        .cursor_pointer()
        .on_click(move |_, _window, cx| {
            entity.update(cx, |popover, cx| {
                popover.cycle_signature(delta);
                cx.notify();
            });
        })
        .child(glyph)
}

/// Data needed to render the signature card, bundled to keep the render
/// function within the project's positional-argument limit.
struct SignatureCardContent {
    label: String,
    parameters: Vec<String>,
    active_parameter: usize,
    has_overloads: bool,
    popover: Entity<SignatureHelpPopover>,
    active_index: usize,
    total: usize,
}

/// The popover's visual content: a header (with overload cycling) and the
/// active signature with its highlighted parameter.
fn render_signature_card(
    _window: &mut Window,
    cx: &mut gpui::App,
    content: &SignatureCardContent,
) -> Div {
    let theme = cx.theme();
    let header_text = if content.has_overloads {
        format!(
            "signature · {}/{}",
            content.active_index + 1,
            content.total
        )
    } else {
        "signature".to_string()
    };
    let highlighted = content
        .parameters
        .get(content.active_parameter)
        .cloned()
        .unwrap_or_default();

    v_flex()
        .gap_1()
        .min_w(px(220.))
        .max_w(px(520.))
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(header_text),
                )
                .when(content.has_overloads, |this| {
                    this.child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child(overload_button(-1, content.popover.clone()))
                            .child(overload_button(1, content.popover.clone())),
                    )
                }),
        )
        .child(
            div()
                .text_sm()
                .text_color(theme.foreground)
                .child(content.label.clone()),
        )
        .when(
            !content.parameters.is_empty() && !highlighted.is_empty(),
            |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "parameter {}: {}",
                            content.active_parameter + 1,
                            highlighted
                        )),
                )
            },
        )
}

#[cfg(test)]
mod tests {
    use super::cycle_overload;

    #[test]
    fn overload_cycle_wraps_forward_and_back() {
        assert_eq!(cycle_overload(3, 0, 1), 1);
        assert_eq!(cycle_overload(3, 2, 1), 0);
        assert_eq!(cycle_overload(3, 0, -1), 2);
        assert_eq!(cycle_overload(3, 1, 4), 2);
    }

    #[test]
    fn single_signature_is_the_identity() {
        assert_eq!(cycle_overload(1, 0, 1), 0);
        assert_eq!(cycle_overload(0, 0, 1), 0);
    }

    #[test]
    fn invalid_current_is_clamped() {
        assert_eq!(cycle_overload(2, 99, 1), 0);
        assert_eq!(cycle_overload(3, 7, 0), 1);
    }
}