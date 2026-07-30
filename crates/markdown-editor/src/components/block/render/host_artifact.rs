use gpui::*;

use super::super::HostRenderedArtifact;

#[derive(Clone, Copy)]
pub(super) struct HostArtifactSize {
    pub(super) width: f32,
    pub(super) height: f32,
}

pub(super) fn contained_block_size(
    rendered: &HostRenderedArtifact,
    available_width: f32,
    max_height: f32,
    fallback_height: f32,
) -> HostArtifactSize {
    let fallback_width = available_width.min(320.0);
    let size = intrinsic_size(rendered, fallback_width, fallback_height);
    fit_within(size, available_width, max_height, false)
}

pub(super) fn scrollable_block_size(
    rendered: &HostRenderedArtifact,
    available_width: f32,
    max_height: f32,
    fallback_height: f32,
) -> HostArtifactSize {
    let size = intrinsic_size(rendered, available_width, fallback_height);
    let height_scale = (positive(max_height, size.height) / size.height).min(1.0);
    HostArtifactSize {
        width: (size.width * height_scale).max(1.0),
        height: (size.height * height_scale).max(1.0),
    }
}

pub(super) fn inline_size(
    rendered: &HostRenderedArtifact,
    line_height: f32,
    available_width: f32,
) -> HostArtifactSize {
    let line_height = positive(line_height, 24.0);
    let size = intrinsic_size(rendered, line_height * 1.6, line_height);
    fit_within(size, available_width, line_height, true)
}

pub(super) fn render_host_svg(
    rendered: &HostRenderedArtifact,
    size: HostArtifactSize,
) -> AnyElement {
    img(rendered.image.clone())
        .w(px(size.width))
        .h(px(size.height))
        .object_fit(ObjectFit::Contain)
        .into_any_element()
}

fn intrinsic_size(
    rendered: &HostRenderedArtifact,
    fallback_width: f32,
    fallback_height: f32,
) -> HostArtifactSize {
    HostArtifactSize {
        width: rendered
            .artifact
            .intrinsic_width
            .map(|width| positive(width, fallback_width))
            .unwrap_or_else(|| positive(fallback_width, 160.0)),
        height: rendered
            .artifact
            .intrinsic_height
            .map(|height| positive(height, fallback_height))
            .unwrap_or_else(|| positive(fallback_height, 96.0)),
    }
}

fn fit_within(
    size: HostArtifactSize,
    max_width: f32,
    max_height: f32,
    allow_upscale: bool,
) -> HostArtifactSize {
    let max_width = positive(max_width, size.width);
    let max_height = positive(max_height, size.height);
    let mut scale = (max_width / size.width).min(max_height / size.height);
    if !allow_upscale {
        scale = scale.min(1.0);
    }
    HostArtifactSize {
        width: (size.width * scale).max(1.0),
        height: (size.height * scale).max(1.0),
    }
}

fn positive(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback.max(1.0)
    }
}
