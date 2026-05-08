use ferrum_flow::{Interaction, InteractionResult, PluginContext, RenderContext};
use gpui::{
    Bounds, IntoElement, ParentElement as _, Pixels, Point, Size, Styled as _, div, hsla, px,
};

const SCROLLBAR_MARGIN: f32 = 8.0;
const SCROLLBAR_THICKNESS: f32 = 8.0;
const SCROLLBAR_MIN_THUMB: f32 = 32.0;
const CONTENT_PADDING: f32 = 80.0;

#[derive(Clone, Copy)]
pub(super) enum ScrollbarAxis {
    Horizontal,
    Vertical,
}

#[derive(Default)]
pub(super) struct ScrollbarState {
    horizontal_thumb: Option<Bounds<Pixels>>,
    vertical_thumb: Option<Bounds<Pixels>>,
}

impl ScrollbarState {
    pub(super) fn axis_at(&self, position: Point<Pixels>) -> Option<ScrollbarAxis> {
        if self
            .horizontal_thumb
            .is_some_and(|bounds| bounds.contains(&position))
        {
            return Some(ScrollbarAxis::Horizontal);
        }
        if self
            .vertical_thumb
            .is_some_and(|bounds| bounds.contains(&position))
        {
            return Some(ScrollbarAxis::Vertical);
        }
        None
    }

    pub(super) fn render(&mut self, ctx: &mut RenderContext) -> Option<gpui::AnyElement> {
        let Some(metrics) = ScrollbarMetrics::from_render_context(ctx) else {
            self.horizontal_thumb = None;
            self.vertical_thumb = None;
            return None;
        };
        self.horizontal_thumb = metrics.horizontal_thumb;
        self.vertical_thumb = metrics.vertical_thumb;

        let track_color = hsla(0.0, 0.0, 0.0, 0.18);
        let thumb_color = hsla(0.0, 0.0, 0.45, 0.55);
        Some(
            div()
                .absolute()
                .size_full()
                .child(render_track(metrics.horizontal_track, track_color))
                .child(render_track(metrics.vertical_track, track_color))
                .child(render_thumb(metrics.horizontal_thumb, thumb_color))
                .child(render_thumb(metrics.vertical_thumb, thumb_color))
                .into_any_element(),
        )
    }
}

fn render_track(track: Option<Bounds<Pixels>>, color: gpui::Hsla) -> impl IntoElement {
    div().children(track.map(|track| render_bar(track, color)))
}

fn render_thumb(thumb: Option<Bounds<Pixels>>, color: gpui::Hsla) -> impl IntoElement {
    div().children(thumb.map(|thumb| render_bar(thumb, color)))
}

fn render_bar(bounds: Bounds<Pixels>, color: gpui::Hsla) -> impl IntoElement {
    div()
        .absolute()
        .left(bounds.origin.x)
        .top(bounds.origin.y)
        .w(bounds.size.width)
        .h(bounds.size.height)
        .rounded(px(SCROLLBAR_THICKNESS / 2.0))
        .bg(color)
}

pub(super) struct ScrollbarDragInteraction {
    axis: ScrollbarAxis,
    start_mouse: Point<Pixels>,
    start_offset: Point<Pixels>,
    world_bounds: Option<WorldBounds>,
    window_bounds: Option<Bounds<Pixels>>,
    zoom: f32,
}

impl ScrollbarDragInteraction {
    pub(super) fn new(
        axis: ScrollbarAxis,
        start_mouse: Point<Pixels>,
        ctx: &PluginContext,
    ) -> Self {
        Self {
            axis,
            start_mouse,
            start_offset: ctx.offset(),
            world_bounds: graph_world_bounds(ctx),
            window_bounds: ctx.window_bounds(),
            zoom: ctx.zoom(),
        }
    }
}

impl Interaction for ScrollbarDragInteraction {
    fn on_mouse_move(
        &mut self,
        ev: &gpui::MouseMoveEvent,
        ctx: &mut PluginContext,
    ) -> InteractionResult {
        let Some(bounds) = self.world_bounds else {
            return InteractionResult::End;
        };
        let Some(window_bounds) = self.window_bounds else {
            return InteractionResult::End;
        };
        let next_offset = match self.axis {
            ScrollbarAxis::Horizontal => {
                let delta = ev.position.x - self.start_mouse.x;
                let content_width = bounds.width * self.zoom;
                let track_width = scrollbar_horizontal_track(window_bounds).size.width;
                let movable = (track_width
                    - thumb_length(track_width, window_bounds.size.width, content_width))
                .max(px(1.0));
                let scrollable = (content_width - window_bounds.size.width).max(px(1.0));
                Point::new(
                    self.start_offset.x - delta * pixel_ratio(scrollable, movable),
                    self.start_offset.y,
                )
            }
            ScrollbarAxis::Vertical => {
                let delta = ev.position.y - self.start_mouse.y;
                let content_height = bounds.height * self.zoom;
                let track_height = scrollbar_vertical_track(window_bounds).size.height;
                let movable = (track_height
                    - thumb_length(track_height, window_bounds.size.height, content_height))
                .max(px(1.0));
                let scrollable = (content_height - window_bounds.size.height).max(px(1.0));
                Point::new(
                    self.start_offset.x,
                    self.start_offset.y - delta * pixel_ratio(scrollable, movable),
                )
            }
        };
        ctx.set_offset(next_offset);
        ctx.notify();
        InteractionResult::Continue
    }

    fn on_mouse_up(
        &mut self,
        _event: &gpui::MouseUpEvent,
        ctx: &mut PluginContext,
    ) -> InteractionResult {
        ctx.cancel_interaction();
        InteractionResult::End
    }
}

#[derive(Clone, Copy)]
struct WorldBounds {
    min_x: f32,
    min_y: f32,
    width: Pixels,
    height: Pixels,
}

struct ScrollbarMetrics {
    horizontal_track: Option<Bounds<Pixels>>,
    horizontal_thumb: Option<Bounds<Pixels>>,
    vertical_track: Option<Bounds<Pixels>>,
    vertical_thumb: Option<Bounds<Pixels>>,
}

impl ScrollbarMetrics {
    fn from_render_context(ctx: &RenderContext) -> Option<Self> {
        let bounds = graph_world_bounds_from_render(ctx)?;
        let window_bounds = ctx.window_bounds()?;
        let zoom = ctx.zoom();
        let content_width = bounds.width * zoom;
        let content_height = bounds.height * zoom;
        let horizontal_track = (content_width > window_bounds.size.width)
            .then(|| scrollbar_horizontal_track(window_bounds));
        let vertical_track = (content_height > window_bounds.size.height)
            .then(|| scrollbar_vertical_track(window_bounds));
        let horizontal_thumb = horizontal_track.map(|track| {
            horizontal_thumb_bounds(
                track,
                window_bounds.size.width,
                content_width,
                bounds,
                zoom,
                ctx.offset().x,
            )
        });
        let vertical_thumb = vertical_track.map(|track| {
            vertical_thumb_bounds(
                track,
                window_bounds.size.height,
                content_height,
                bounds,
                zoom,
                ctx.offset().y,
            )
        });
        Some(Self {
            horizontal_track,
            horizontal_thumb,
            vertical_track,
            vertical_thumb,
        })
    }
}

fn graph_world_bounds(ctx: &PluginContext) -> Option<WorldBounds> {
    ctx.graph.nodes_world_aabb().map(world_bounds_from_aabb)
}

fn graph_world_bounds_from_render(ctx: &RenderContext) -> Option<WorldBounds> {
    ctx.graph.nodes_world_aabb().map(world_bounds_from_aabb)
}

fn world_bounds_from_aabb((min_x, min_y, width, height): (f32, f32, f32, f32)) -> WorldBounds {
    WorldBounds {
        min_x: min_x - CONTENT_PADDING,
        min_y: min_y - CONTENT_PADDING,
        width: px(width + 2.0 * CONTENT_PADDING),
        height: px(height + 2.0 * CONTENT_PADDING),
    }
}

fn scrollbar_horizontal_track(window_bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    Bounds::new(
        Point::new(
            px(SCROLLBAR_MARGIN),
            window_bounds.size.height - px(SCROLLBAR_MARGIN + SCROLLBAR_THICKNESS),
        ),
        Size::new(
            window_bounds.size.width - px(2.0 * SCROLLBAR_MARGIN + SCROLLBAR_THICKNESS),
            px(SCROLLBAR_THICKNESS),
        ),
    )
}

fn scrollbar_vertical_track(window_bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    Bounds::new(
        Point::new(
            window_bounds.size.width - px(SCROLLBAR_MARGIN + SCROLLBAR_THICKNESS),
            px(SCROLLBAR_MARGIN),
        ),
        Size::new(
            px(SCROLLBAR_THICKNESS),
            window_bounds.size.height - px(2.0 * SCROLLBAR_MARGIN + SCROLLBAR_THICKNESS),
        ),
    )
}

fn horizontal_thumb_bounds(
    track: Bounds<Pixels>,
    viewport_width: Pixels,
    content_width: Pixels,
    bounds: WorldBounds,
    zoom: f32,
    offset_x: Pixels,
) -> Bounds<Pixels> {
    let length = thumb_length(track.size.width, viewport_width, content_width);
    let movable = (track.size.width - length).max(px(0.0));
    let scrollable = (content_width - viewport_width).max(px(1.0));
    let content_start = px(bounds.min_x) * zoom + offset_x;
    let ratio = (-content_start / scrollable).clamp(0.0, 1.0);
    Bounds::new(
        Point::new(track.origin.x + movable * ratio, track.origin.y),
        Size::new(length, track.size.height),
    )
}

fn vertical_thumb_bounds(
    track: Bounds<Pixels>,
    viewport_height: Pixels,
    content_height: Pixels,
    bounds: WorldBounds,
    zoom: f32,
    offset_y: Pixels,
) -> Bounds<Pixels> {
    let length = thumb_length(track.size.height, viewport_height, content_height);
    let movable = (track.size.height - length).max(px(0.0));
    let scrollable = (content_height - viewport_height).max(px(1.0));
    let content_start = px(bounds.min_y) * zoom + offset_y;
    let ratio = (-content_start / scrollable).clamp(0.0, 1.0);
    Bounds::new(
        Point::new(track.origin.x, track.origin.y + movable * ratio),
        Size::new(track.size.width, length),
    )
}

pub(super) fn thumb_length(
    track_length: Pixels,
    viewport_length: Pixels,
    content_length: Pixels,
) -> Pixels {
    (track_length * pixel_ratio(viewport_length, content_length))
        .clamp(px(SCROLLBAR_MIN_THUMB), track_length)
}

fn pixel_ratio(numerator: Pixels, denominator: Pixels) -> f32 {
    f32::from(numerator) / f32::from(denominator)
}
