use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Element, ElementId, Entity, InteractiveElement as _,
    IntoElement, MouseDownEvent, ParentElement as _, Pixels, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, deferred, div, point,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};
use lsp_types::{ParameterLabel, SignatureInformation};

use super::ExtendedEditorState;

const EDGE_GAP: Pixels = px(8.0);
const MIN_WIDTH: Pixels = px(220.0);
const MAX_WIDTH: Pixels = px(520.0);

pub(super) struct SignatureHelpOverlay {
    state: Entity<ExtendedEditorState>,
}

impl SignatureHelpOverlay {
    pub(super) fn new(state: Entity<ExtendedEditorState>) -> Self {
        Self { state }
    }

    fn trigger_bounds(&self, cx: &App) -> Option<Bounds<Pixels>> {
        let state = self.state.read(cx);
        let anchor = state.anchor()?;
        state.editor().read(cx).range_to_bounds(&(anchor..anchor))
    }

    fn content(&self, cx: &mut App) -> Option<AnyElement> {
        let state = self.state.read(cx);
        let help = state.help()?;
        let active = state
            .active_signature()
            .min(help.signatures.len().saturating_sub(1));
        let signature = help.signatures.get(active)?;
        let parameter = active_parameter(signature, help.active_parameter);
        let parameter = parameter.and_then(|index| parameter_text(signature, index));
        let total = help.signatures.len();
        let owner = self.state.clone();

        Some(
            v_flex()
                .gap_1()
                .min_w(MIN_WIDTH)
                .max_w(MAX_WIDTH)
                .child(render_header(owner, active, total, cx))
                .child(div().text_sm().child(signature.label.clone()))
                .when_some(parameter, |this, (index, label)| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("parameter {}: {label}", index + 1)),
                    )
                })
                .into_any_element(),
        )
    }
}

fn active_parameter(signature: &SignatureInformation, fallback: Option<u32>) -> Option<usize> {
    signature
        .active_parameter
        .or(fallback)
        .map(|value| value as usize)
}

fn parameter_text(signature: &SignatureInformation, index: usize) -> Option<(usize, String)> {
    let parameter = signature.parameters.as_ref()?.get(index)?;
    let text = match &parameter.label {
        ParameterLabel::Simple(label) => label.clone(),
        ParameterLabel::LabelOffsets([start, end]) => signature
            .label
            .get(*start as usize..*end as usize)
            .unwrap_or_default()
            .to_string(),
    };
    Some((index, text))
}

fn render_header(
    state: Entity<ExtendedEditorState>,
    active: usize,
    total: usize,
    cx: &App,
) -> impl IntoElement {
    let title = if total > 1 {
        format!("signature · {}/{}", active + 1, total)
    } else {
        "signature".to_string()
    };
    h_flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(title),
        )
        .when(total > 1, |this| {
            this.child(
                h_flex()
                    .gap_1()
                    .child(overload_button("previous", "‹", -1, state.clone()))
                    .child(overload_button("next", "›", 1, state)),
            )
        })
}

fn overload_button(
    name: &'static str,
    glyph: &'static str,
    delta: isize,
    state: Entity<ExtendedEditorState>,
) -> impl IntoElement {
    div()
        .id(ElementId::Name(SharedString::from(format!(
            "signature-{name}"
        ))))
        .px_1()
        .on_click(move |_, _, cx| {
            state.update(cx, |state, cx| state.cycle_signature(delta, cx));
        })
        .child(glyph)
}

pub(super) struct OverlayLayoutState {
    bounds: Bounds<Pixels>,
    element: Option<AnyElement>,
}

impl IntoElement for SignatureHelpOverlay {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SignatureHelpOverlay {
    type RequestLayoutState = OverlayLayoutState;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some("signature-help-overlay".into())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let Some(trigger) = self.trigger_bounds(cx) else {
            return empty_layout(window, cx);
        };
        let Some(content) = self.content(cx) else {
            return empty_layout(window, cx);
        };
        let max_width = MAX_WIDTH.min(window.bounds().size.width - EDGE_GAP * 2.0);
        let mut element = deferred(
            div()
                .occlude()
                .p_2()
                .max_w(max_width)
                .bg(cx.theme().popover)
                .border_1()
                .border_color(cx.theme().border)
                .rounded(cx.theme().radius)
                .shadow_md()
                .child(content),
        )
        .into_any_element();
        let size = element.layout_as_root(AvailableSpace::min_size(), window, cx);
        let mut origin = point(trigger.left(), trigger.top() - size.height);
        if origin.y < EDGE_GAP {
            origin.y = trigger.bottom();
        }
        if origin.x + size.width > window.bounds().size.width - EDGE_GAP {
            origin.x = (window.bounds().size.width - size.width - EDGE_GAP).max(EDGE_GAP);
        }
        let mut empty = div().into_any_element();
        let layout_id = empty.request_layout(window, cx);
        (
            layout_id,
            OverlayLayoutState {
                bounds: Bounds { origin, size },
                element: Some(element),
            },
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(element) = state.element.as_mut() {
            window.with_absolute_element_offset(state.bounds.origin, |window| {
                element.prepaint(window, cx)
            });
        }
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(element) = state.element.as_mut() else {
            return;
        };
        element.paint(window, cx);
        let bounds = state.bounds;
        let owner = self.state.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, _, _, cx| {
            if !bounds.contains(&event.position) {
                let _ = owner.update(cx, |state, cx| state.close_signature_help(cx));
            }
        });
    }
}

fn empty_layout(window: &mut Window, cx: &mut App) -> (gpui::LayoutId, OverlayLayoutState) {
    let mut empty = div().into_any_element();
    let layout_id = empty.request_layout(window, cx);
    (
        layout_id,
        OverlayLayoutState {
            bounds: Bounds::default(),
            element: None,
        },
    )
}
