//! Enlarged rendered-view overlay for Mermaid and math blocks.
//!
//! Clicking a rendered Mermaid diagram or display-math block opens a centered
//! overlay showing the block's rendered preview and its source, with a
//! source/preview toggle in the top-right corner.

use gpui::*;

use super::Editor;
use crate::components::{EnlargedBlockKind, HostRenderedArtifact};
use crate::i18n::I18nManager;
use crate::theme::Theme;

/// State for the enlarged Mermaid/Math view opened from a rendered block.
pub(super) struct EnlargedBlockState {
    pub(super) kind: EnlargedBlockKind,
    /// The diagram/math body source shown in source mode.
    pub(super) source: SharedString,
    /// Host-rendered SVG artifact backing the preview.
    pub(super) artifact: HostRenderedArtifact,
    /// Whether the body shows the source instead of the preview.
    pub(super) show_source: bool,
}

/// Largest the enlarged preview may occupy inside the overlay body.
struct EnlargedPreviewLimit {
    width: f32,
    height: f32,
}

/// Fits the artifact's intrinsic size within the body, upscaling small
/// diagrams (capped at 2x) so a tiny formula does not balloon.
fn enlarged_artifact_size(
    artifact: &HostRenderedArtifact,
    limit: EnlargedPreviewLimit,
) -> (f32, f32) {
    let intrinsic_width = artifact
        .artifact
        .intrinsic_width
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(limit.width.min(480.0));
    let intrinsic_height = artifact
        .artifact
        .intrinsic_height
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(limit.height.min(320.0));
    let scale =
        ((limit.width / intrinsic_width).min(limit.height / intrinsic_height)).clamp(0.1, 2.0);
    (
        (intrinsic_width * scale).max(1.0),
        (intrinsic_height * scale).max(1.0),
    )
}

impl Editor {
    /// Opens the enlarged view for a clicked rendered Mermaid/Math block.
    pub(super) fn open_enlarged_block(
        &mut self,
        kind: EnlargedBlockKind,
        source: String,
        artifact: HostRenderedArtifact,
        cx: &mut Context<Self>,
    ) {
        self.enlarged_block = Some(EnlargedBlockState {
            kind,
            source: source.into(),
            artifact,
            show_source: false,
        });
        cx.notify();
    }

    /// Switches the enlarged view body to the rendered preview.
    pub(super) fn on_enlarged_view_preview(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_enlarged_show_source(false, cx);
    }

    /// Switches the enlarged view body to the block's Markdown source.
    pub(super) fn on_enlarged_view_source(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_enlarged_show_source(true, cx);
    }

    /// Sets whether the enlarged view body shows the source.
    fn set_enlarged_show_source(&mut self, show_source: bool, cx: &mut Context<Self>) {
        if let Some(state) = self.enlarged_block.as_mut() {
            state.show_source = show_source;
            cx.notify();
        }
    }

    /// Renders the enlarged Mermaid/Math overlay, or `None` when closed.
    pub(super) fn render_enlarged_block_overlay(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = self.enlarged_block.as_ref()?;
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        let strings = cx.global::<I18nManager>().strings().clone();

        let viewport = window.viewport_size();
        let panel_width = (f32::from(viewport.width) * 0.9).min(960.0);
        let panel_max_height = (f32::from(viewport.height) * 0.85).max(240.0);
        let body_max_height = panel_max_height - 64.0;
        let title = match state.kind {
            EnlargedBlockKind::Mermaid => "Mermaid".into(),
            EnlargedBlockKind::Math => strings.enlarged_view_math_title.clone(),
        };

        let toggle_button =
            |id: &'static str,
             label: String,
             active: bool,
             handler: fn(&mut Editor, &ClickEvent, &mut Window, &mut Context<Editor>)| {
                let base = div()
                    .id(id)
                    .h(px(d.dialog_button_height))
                    .px(px(d.dialog_button_padding_x))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px((d.dialog_radius - 4.0).max(0.0)))
                    .border(px(d.dialog_border_width))
                    .cursor_pointer()
                    .text_size(px(t.dialog_button_size))
                    .font_weight(t.dialog_button_weight.to_font_weight());
                let base = if active {
                    base.border_color(c.dialog_border)
                        .bg(c.dialog_primary_button_bg)
                        .text_color(c.dialog_primary_button_text)
                } else {
                    base.border_color(c.dialog_border)
                        .bg(c.dialog_secondary_button_bg)
                        .hover(|this| this.bg(c.dialog_secondary_button_hover))
                        .text_color(c.dialog_secondary_button_text)
                        .on_click(cx.listener(handler))
                };
                base.child(label)
            };

        let body = if state.show_source {
            div()
                .id("enlarged-block-source")
                .w_full()
                .max_h(px(body_max_height))
                .overflow_y_scroll()
                .scrollbar_width(px(0.0))
                .rounded_sm()
                .bg(c.source_mode_block_bg)
                .px(px(d.block_padding_x))
                .py(px(d.block_padding_y))
                .text_size(px(t.code_size))
                .line_height(rems(1.5))
                .text_color(c.text_default)
                .child(state.source.clone())
                .into_any_element()
        } else {
            let (width, height) = enlarged_artifact_size(
                &state.artifact,
                EnlargedPreviewLimit {
                    width: panel_width - d.dialog_padding * 2.0,
                    height: body_max_height,
                },
            );
            div()
                .id("enlarged-block-preview")
                .w_full()
                .max_h(px(body_max_height))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(state.artifact.image.clone())
                        .w(px(width))
                        .h(px(height))
                        .object_fit(ObjectFit::Contain),
                )
                .into_any_element()
        };

        Some(
            div()
                .id("enlarged-block-overlay")
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(c.dialog_backdrop)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(Self::on_dismiss_context_menu_overlay),
                )
                .child(
                    div()
                        .id("enlarged-block-dialog")
                        .w(px(panel_width))
                        .max_w(relative(1.0))
                        .p(px(d.dialog_padding))
                        .flex()
                        .flex_col()
                        .gap(px(d.dialog_gap))
                        .bg(c.dialog_surface)
                        .border(px(d.dialog_border_width))
                        .border_color(c.dialog_border)
                        .rounded(px(d.dialog_radius))
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                            cx.stop_propagation()
                        })
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_size(px(t.dialog_title_size))
                                        .font_weight(t.dialog_title_weight.to_font_weight())
                                        .text_color(c.dialog_title)
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(d.dialog_button_gap))
                                        .child(toggle_button(
                                            "enlarged-view-preview",
                                            strings.enlarged_view_preview.clone(),
                                            !state.show_source,
                                            Self::on_enlarged_view_preview,
                                        ))
                                        .child(toggle_button(
                                            "enlarged-view-source",
                                            strings.enlarged_view_source.clone(),
                                            state.show_source,
                                            Self::on_enlarged_view_source,
                                        )),
                                ),
                        )
                        .child(body),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui::{AppContext, Entity, Image, ImageFormat, Modifiers, TestAppContext, rgba};
    use palette::IntoColor as _;

    use super::{Editor, EnlargedPreviewLimit, enlarged_artifact_size};
    use crate::components::{BlockEvent, BlockKind, EnlargedBlockKind, HostRenderedArtifact};
    use crate::{BlockRenderArtifact, EditorHostServices, EditorHostTheme};

    fn init_editor_test_app(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::i18n::I18nManager::init(cx);
            crate::theme::ThemeManager::init(cx);
            crate::components::init(cx);
        });
    }

    fn redraw(cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.background_executor.run_until_parked();
        cx.run_until_parked();
    }

    fn host_theme() -> EditorHostTheme {
        EditorHostTheme {
            background: rgba(0x112233ff).into_color(),
            foreground: rgba(0x223344ff).into_color(),
            border: rgba(0x334455ff).into_color(),
            muted: rgba(0x445566ff).into_color(),
            accent: rgba(0x556677ff).into_color(),
        }
    }

    fn svg_artifact() -> HostRenderedArtifact {
        HostRenderedArtifact {
            artifact: Arc::new(BlockRenderArtifact {
                media_type: "image/svg+xml".into(),
                bytes: vec![b'<'],
                intrinsic_width: Some(120.0),
                intrinsic_height: Some(40.0),
            }),
            image: Arc::new(Image::from_bytes(ImageFormat::Svg, b"<svg/>".to_vec())),
        }
    }

    fn test_editor(cx: &mut TestAppContext) -> Entity<Editor> {
        cx.new(|cx| Editor::from_markdown(cx, "$$x^2$$\n".to_string(), None))
    }

    #[test]
    fn artifact_size_fits_within_limit_and_caps_upscale() {
        let artifact = svg_artifact();

        let (width, height) = enlarged_artifact_size(
            &artifact,
            EnlargedPreviewLimit {
                width: 300.0,
                height: 200.0,
            },
        );
        assert!((width - 240.0).abs() < 0.01, "2x cap applies, got {width}");
        assert!((height - 80.0).abs() < 0.01);

        let (width, height) = enlarged_artifact_size(
            &artifact,
            EnlargedPreviewLimit {
                width: 60.0,
                height: 20.0,
            },
        );
        assert!((width - 60.0).abs() < 0.01);
        assert!((height - 20.0).abs() < 0.01);
    }

    #[gpui::test]
    async fn opening_enlarged_block_sets_state_and_toggle_switches_body(cx: &mut TestAppContext) {
        let editor = test_editor(cx);

        editor.update(cx, |editor, cx| {
            editor.open_enlarged_block(EnlargedBlockKind::Math, "x^2".into(), svg_artifact(), cx);
            let state = editor
                .enlarged_block
                .as_ref()
                .expect("enlarged view opened");
            assert_eq!(state.kind, EnlargedBlockKind::Math);
            assert_eq!(state.source.as_ref(), "x^2");
            assert!(!state.show_source, "starts in preview mode");

            editor.set_enlarged_show_source(true, cx);
            assert!(
                editor.enlarged_block.as_ref().unwrap().show_source,
                "source toggle shows the source"
            );
            editor.set_enlarged_show_source(false, cx);
            assert!(
                !editor.enlarged_block.as_ref().unwrap().show_source,
                "preview toggle shows the preview"
            );
        });
    }

    #[gpui::test]
    async fn block_event_opens_enlarged_view_and_dismiss_closes_it(cx: &mut TestAppContext) {
        let editor = test_editor(cx);

        editor.update(cx, |editor, cx| {
            editor.on_block_event(
                editor
                    .document
                    .first_root()
                    .expect("math root block")
                    .clone(),
                &BlockEvent::RequestEnlargeRenderedBlock {
                    kind: EnlargedBlockKind::Math,
                    source: "x^2".into(),
                    artifact: svg_artifact(),
                },
                cx,
            );
            let state = editor
                .enlarged_block
                .as_ref()
                .expect("event opens the view");
            assert_eq!(state.source.as_ref(), "x^2");

            editor.dismiss_contextual_overlays(cx);
            assert!(
                editor.enlarged_block.is_none(),
                "dismiss closes the enlarged view"
            );
        });
    }

    #[gpui::test]
    async fn clicking_rendered_math_image_opens_preview_without_focusing_block(
        cx: &mut TestAppContext,
    ) {
        init_editor_test_app(cx);
        let (editor, cx) =
            cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "$$x^2$$".into(), None));
        for _ in 0..3 {
            redraw(cx);
        }

        // The editor's first render focuses its first block. Blur the window
        // before installing the host renderer so the math block is drawn in
        // rendered (image) mode rather than focused source-editing mode.
        let math_block = editor.read_with(cx, |editor, cx| {
            editor
                .document
                .visible_blocks()
                .into_iter()
                .find(|visible| visible.entity.read(cx).kind() == BlockKind::MathBlock)
                .expect("a math block exists")
                .entity
                .clone()
        });
        cx.update(|window, _| window.blur());
        redraw(cx);

        math_block.update(cx, |block, cx| {
            block.set_host_services(Arc::new(
                EditorHostServices::new(host_theme()).with_block_renderer(Arc::new(|_request| {
                    Box::pin(async move {
                        Ok(Some(BlockRenderArtifact {
                            media_type: "image/svg+xml".into(),
                            bytes: b"<svg/>".to_vec(),
                            intrinsic_width: Some(120.0),
                            intrinsic_height: Some(40.0),
                        }))
                    })
                })),
            ));
            block.set_host_render_environment(480.0, 2.0);
            cx.notify();
        });
        for _ in 0..3 {
            redraw(cx);
        }

        let image_bounds = cx
            .debug_bounds("enlargable-host-svg")
            .expect("the rendered math image is displayed");
        cx.simulate_click(image_bounds.center(), Modifiers::default());

        editor.read_with(cx, |editor, _cx| {
            let state = editor
                .enlarged_block
                .as_ref()
                .expect("clicking the rendered image opens the enlarged view");
            assert!(
                !state.show_source,
                "the enlarged view opens in preview mode, not source mode"
            );
        });
        let focused = cx.update(|window, cx| math_block.read(cx).focus_handle.is_focused(window));
        assert!(
            !focused,
            "clicking the rendered image must not focus the block into source editing"
        );
    }
}
