//! Enlarged rendered-view overlay for Mermaid and math blocks.
//!
//! Clicking a rendered Mermaid diagram or display-math block opens a centered
//! overlay showing the rendered preview. Source editing stays on the original
//! block through its top-right Source button.

use gpui::*;
use rust_i18n::t;

use super::Editor;
use crate::components::{EnlargedBlockKind, HostRenderedArtifact};
use crate::theme::Theme;
use gpui_component::{IconName, Size};
use one_ui::icon_button::IconButton;

/// State for the enlarged Mermaid/Math view opened from a rendered block.
pub(super) struct EnlargedBlockState {
    pub(super) kind: EnlargedBlockKind,
    /// Host-rendered SVG artifact backing the preview.
    pub(super) artifact: HostRenderedArtifact,
    /// User-controlled scale relative to the fitted preview size.
    pub(super) zoom: f32,
}

/// Largest the enlarged preview may occupy inside the overlay body.
struct EnlargedPreviewLimit {
    width: f32,
    height: f32,
}

const ENLARGED_ZOOM_DEFAULT: f32 = 1.0;
const ENLARGED_ZOOM_MIN: f32 = 0.25;
const ENLARGED_ZOOM_MAX: f32 = 4.0;
const ENLARGED_ZOOM_STEP: f32 = 0.25;

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
        _source: String,
        artifact: HostRenderedArtifact,
        cx: &mut Context<Self>,
    ) {
        self.enlarged_block = Some(EnlargedBlockState {
            kind,
            artifact,
            zoom: ENLARGED_ZOOM_DEFAULT,
        });
        cx.notify();
    }

    fn zoom_in_enlarged_block(
        &mut self,
        _event: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(state) = self.enlarged_block.as_mut() {
            state.zoom = (state.zoom + ENLARGED_ZOOM_STEP).min(ENLARGED_ZOOM_MAX);
            cx.notify();
        }
    }

    fn zoom_out_enlarged_block(
        &mut self,
        _event: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(state) = self.enlarged_block.as_mut() {
            state.zoom = (state.zoom - ENLARGED_ZOOM_STEP).max(ENLARGED_ZOOM_MIN);
            cx.notify();
        }
    }

    fn close_enlarged_block(
        &mut self,
        _event: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.enlarged_block = None;
        cx.notify();
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
        let viewport = window.viewport_size();
        let panel_width = (f32::from(viewport.width) * 0.9).min(960.0);
        let panel_max_height = (f32::from(viewport.height) * 0.85).max(240.0);
        let controls_height = d.dialog_button_height;
        let body_max_height = (panel_max_height - controls_height - d.dialog_gap).max(1.0);
        let title = match state.kind {
            EnlargedBlockKind::Mermaid => {
                t!("MarkdownEditor.enlarged_view_mermaid_title").to_string()
            }
            EnlargedBlockKind::Math => t!("MarkdownEditor.enlarged_view_math_title").to_string(),
        };

        let (width, height) = enlarged_artifact_size(
            &state.artifact,
            EnlargedPreviewLimit {
                width: panel_width,
                height: body_max_height,
            },
        );
        let width = width * state.zoom;
        let height = height * state.zoom;
        let content_width = width.max(panel_width);
        let content_height = height.max(body_max_height);
        let body = div()
            .id("enlarged-block-preview")
            .debug_selector(|| "enlarged-block-preview".to_string())
            .w_full()
            .h(px(body_max_height))
            .overflow_scroll()
            .child(
                div()
                    .w(px(content_width))
                    .h(px(content_height))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        img(state.artifact.image.clone())
                            .w(px(width))
                            .h(px(height))
                            .object_fit(ObjectFit::Contain),
                    ),
            )
            .into_any_element();

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
                    // No card/background around the enlarged content: clicking
                    // a rendered block opens the preview directly.
                    div()
                        .id("enlarged-block-content")
                        .w(px(panel_width))
                        .max_w(relative(1.0))
                        .relative()
                        .flex()
                        .flex_col()
                        .gap(px(d.dialog_gap))
                        .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                            cx.stop_propagation()
                        })
                        .child(
                            div()
                                .w_full()
                                .h(px(controls_height))
                                .flex()
                                .items_center()
                                .justify_start()
                                .child(
                                    div()
                                        .text_size(px(t.dialog_title_size))
                                        .font_weight(t.dialog_title_weight.to_font_weight())
                                        .text_color(c.dialog_title)
                                        .child(title),
                                ),
                        )
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .right_0()
                                .flex()
                                .items_center()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .id("enlarged-view-zoom-in")
                                        .debug_selector(|| "enlarged-view-zoom-in".to_string())
                                        .child(
                                            IconButton::new(
                                                "enlarged-view-zoom-in-button",
                                                IconName::Plus,
                                            )
                                            .hit_size(Size::XSmall)
                                            .tooltip(
                                                t!("MarkdownEditor.enlarged_view_zoom_in")
                                                    .to_string(),
                                            )
                                            .accessible_label(
                                                t!("MarkdownEditor.enlarged_view_zoom_in")
                                                    .to_string(),
                                            )
                                            .on_click(cx.listener(Self::zoom_in_enlarged_block)),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("enlarged-view-zoom-out")
                                        .debug_selector(|| "enlarged-view-zoom-out".to_string())
                                        .child(
                                            IconButton::new(
                                                "enlarged-view-zoom-out-button",
                                                IconName::Minus,
                                            )
                                            .hit_size(Size::XSmall)
                                            .tooltip(
                                                t!("MarkdownEditor.enlarged_view_zoom_out")
                                                    .to_string(),
                                            )
                                            .accessible_label(
                                                t!("MarkdownEditor.enlarged_view_zoom_out")
                                                    .to_string(),
                                            )
                                            .on_click(cx.listener(Self::zoom_out_enlarged_block)),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("enlarged-view-close")
                                        .debug_selector(|| "enlarged-view-close".to_string())
                                        .child(
                                            IconButton::new(
                                                "enlarged-view-close-button",
                                                IconName::Close,
                                            )
                                            .hit_size(Size::XSmall)
                                            .tooltip(
                                                t!("MarkdownEditor.enlarged_view_close")
                                                    .to_string(),
                                            )
                                            .accessible_label(
                                                t!("MarkdownEditor.enlarged_view_close")
                                                    .to_string(),
                                            )
                                            .on_click(cx.listener(Self::close_enlarged_block)),
                                        ),
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

    use super::{
        ENLARGED_ZOOM_DEFAULT, ENLARGED_ZOOM_STEP, Editor, EnlargedPreviewLimit,
        enlarged_artifact_size,
    };
    use crate::components::{BlockEvent, BlockKind, EnlargedBlockKind, HostRenderedArtifact};
    use crate::{BlockRenderArtifact, EditorHostServices, EditorHostTheme};

    fn init_editor_test_app(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
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
    async fn opening_enlarged_block_sets_preview_state(cx: &mut TestAppContext) {
        let editor = test_editor(cx);

        editor.update(cx, |editor, cx| {
            editor.open_enlarged_block(EnlargedBlockKind::Math, "x^2".into(), svg_artifact(), cx);
            let state = editor
                .enlarged_block
                .as_ref()
                .expect("enlarged view opened");
            assert_eq!(state.kind, EnlargedBlockKind::Math);
            assert_eq!(state.zoom, ENLARGED_ZOOM_DEFAULT);
        });
    }

    #[gpui::test]
    async fn enlarged_preview_controls_zoom_and_close(cx: &mut TestAppContext) {
        init_editor_test_app(cx);
        let (editor, cx) =
            cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "$$x^2$$".into(), None));
        editor.update(cx, |editor, cx| {
            editor.open_enlarged_block(EnlargedBlockKind::Math, "x^2".into(), svg_artifact(), cx);
        });
        redraw(cx);

        assert!(cx.debug_bounds("enlarged-view-zoom-in").is_some());
        assert!(cx.debug_bounds("enlarged-view-zoom-out").is_some());
        assert!(cx.debug_bounds("enlarged-view-close").is_some());

        let zoom_in = cx
            .debug_bounds("enlarged-view-zoom-in")
            .expect("zoom-in button");
        cx.simulate_click(zoom_in.center(), Modifiers::default());
        redraw(cx);
        editor.read_with(cx, |editor, _cx| {
            assert_eq!(
                editor
                    .enlarged_block
                    .as_ref()
                    .expect("preview stays open")
                    .zoom,
                ENLARGED_ZOOM_DEFAULT + ENLARGED_ZOOM_STEP
            );
        });

        let zoom_out = cx
            .debug_bounds("enlarged-view-zoom-out")
            .expect("zoom-out button");
        cx.simulate_click(zoom_out.center(), Modifiers::default());
        redraw(cx);
        editor.read_with(cx, |editor, _cx| {
            assert_eq!(
                editor
                    .enlarged_block
                    .as_ref()
                    .expect("preview stays open")
                    .zoom,
                ENLARGED_ZOOM_DEFAULT
            );
        });

        let close = cx
            .debug_bounds("enlarged-view-close")
            .expect("close button");
        cx.simulate_click(close.center(), Modifiers::default());
        redraw(cx);
        editor.read_with(cx, |editor, _cx| {
            assert!(editor.enlarged_block.is_none());
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
            assert_eq!(state.kind, EnlargedBlockKind::Math);

            editor.dismiss_contextual_overlays(cx);
            assert!(
                editor.enlarged_block.is_none(),
                "dismiss closes the enlarged view"
            );
        });
    }

    #[gpui::test]
    async fn source_button_is_on_the_rendered_block_and_focuses_source(cx: &mut TestAppContext) {
        init_editor_test_app(cx);
        let (editor, cx) =
            cx.add_window_view(|_window, cx| Editor::from_markdown(cx, "$$x^2$$".into(), None));
        for _ in 0..3 {
            redraw(cx);
        }

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

        let source_button = cx
            .debug_bounds("rendered-block-source")
            .expect("rendered math block exposes its Source button");
        cx.simulate_click(source_button.center(), Modifiers::default());
        redraw(cx);

        let focused = cx.update(|window, cx| math_block.read(cx).focus_handle.is_focused(window));
        assert!(focused, "Source focuses the original block for editing");
        editor.read_with(cx, |editor, _cx| {
            assert!(
                editor.enlarged_block.is_none(),
                "Source must not open the enlarged preview"
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
        redraw(cx);

        editor.read_with(cx, |editor, _cx| {
            editor
                .enlarged_block
                .as_ref()
                .expect("clicking the rendered image opens the enlarged view");
        });

        assert!(
            cx.debug_bounds("enlarged-block-preview").is_some(),
            "opening the enlarged view renders the image preview"
        );
        assert!(
            cx.debug_bounds("enlarged-view-source").is_none(),
            "the enlarged preview does not add a second Source button"
        );

        let focused = cx.update(|window, cx| math_block.read(cx).focus_handle.is_focused(window));
        assert!(
            !focused,
            "clicking the rendered image must not focus the block into source editing"
        );
    }
}
