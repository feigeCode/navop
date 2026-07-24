use super::*;

const RDP_DISPLAY_MIN_SIZE: f32 = 200.0;
const RDP_DISPLAY_MAX_SIZE: f32 = 8192.0;
const MAX_REMOTE_FRAME_PIXELS: f32 = 3840.0 * 2160.0;

#[derive(Default)]
pub(super) struct InitialSize {
    pending: Option<PendingInitialSize>,
}

struct PendingInitialSize {
    size: (u16, u16),
    scale_factor: u32,
    updated_at: Instant,
}

impl InitialSize {
    pub(super) fn observe(&mut self, size: (u16, u16), scale_factor: u32, observed_at: Instant) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.size == size && pending.scale_factor == scale_factor)
        {
            return;
        }
        self.pending = Some(PendingInitialSize {
            size,
            scale_factor,
            updated_at: observed_at,
        });
    }

    pub(super) fn take_ready(
        &mut self,
        now: Instant,
        debounce: Duration,
    ) -> Option<((u16, u16), u32)> {
        let pending = self.pending.as_ref()?;
        if now.saturating_duration_since(pending.updated_at) < debounce {
            return None;
        }
        let pending = self.pending.take()?;
        Some((pending.size, pending.scale_factor))
    }
}

pub(super) fn resize_dimensions(
    bounds: Bounds<Pixels>,
    display_scale_factor: f32,
) -> Option<(u16, u16)> {
    let display_scale_factor = if display_scale_factor.is_finite() && display_scale_factor > 0.0 {
        display_scale_factor
    } else {
        1.0
    };
    let mut width = pixels_to_f32(bounds.size.width) * display_scale_factor;
    let mut height = pixels_to_f32(bounds.size.height) * display_scale_factor;
    let area = width * height;
    if area.is_finite() && area > MAX_REMOTE_FRAME_PIXELS {
        let scale = (MAX_REMOTE_FRAME_PIXELS / area).sqrt();
        width *= scale;
        height *= scale;
    }
    let mut width = width
        .round()
        .clamp(RDP_DISPLAY_MIN_SIZE, RDP_DISPLAY_MAX_SIZE) as u16;
    if width % 2 != 0 {
        width = width.saturating_sub(1);
    }
    let height = height
        .round()
        .clamp(RDP_DISPLAY_MIN_SIZE, RDP_DISPLAY_MAX_SIZE) as u16;
    Some((width, height))
}

pub(super) fn is_meaningful_delta(previous: Option<(u16, u16)>, next: (u16, u16)) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    previous.0.abs_diff(next.0) >= RESIZE_DELTA_THRESHOLD
        || previous.1.abs_diff(next.1) >= RESIZE_DELTA_THRESHOLD
}

pub(super) fn scale_factor_percent(display_scale_factor: f32) -> u32 {
    if !display_scale_factor.is_finite() || display_scale_factor <= 0.0 {
        return 100;
    }
    (display_scale_factor * 100.0).round().clamp(100.0, 300.0) as u32
}

fn pixels_to_f32(pixels: Pixels) -> f32 {
    pixels.into()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use gpui::{Bounds, point, px, size};

    use super::{InitialSize, is_meaningful_delta, resize_dimensions};

    #[test]
    fn waits_for_initial_size_to_stabilize() {
        let started_at = Instant::now();
        let debounce = Duration::from_millis(800);
        let mut initial_size = InitialSize::default();

        initial_size.observe((1280, 800), 200, started_at);
        assert_eq!(
            None,
            initial_size.take_ready(started_at + Duration::from_millis(799), debounce)
        );

        initial_size.observe((1920, 1080), 200, started_at + Duration::from_millis(500));
        assert_eq!(
            None,
            initial_size.take_ready(started_at + Duration::from_millis(1299), debounce)
        );
        assert_eq!(
            Some(((1920, 1080), 200)),
            initial_size.take_ready(started_at + Duration::from_millis(1300), debounce)
        );
        assert_eq!(
            None,
            initial_size.take_ready(started_at + Duration::from_millis(2000), debounce)
        );
    }

    #[test]
    fn repeated_identical_initial_size_does_not_restart_debounce() {
        let started_at = Instant::now();
        let debounce = Duration::from_millis(800);
        let mut initial_size = InitialSize::default();

        initial_size.observe((1920, 1080), 200, started_at);
        initial_size.observe((1920, 1080), 200, started_at + Duration::from_millis(700));

        assert_eq!(
            Some(((1920, 1080), 200)),
            initial_size.take_ready(started_at + debounce, debounce)
        );
    }

    #[test]
    fn adjusts_to_display_control_limits() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(1281.4), px(720.6)));
        assert_eq!(Some((1280, 721)), resize_dimensions(bounds, 1.0));

        let oversized = Bounds::new(point(px(0.0), px(0.0)), size(px(90000.0), px(0.0)));
        assert_eq!(Some((8192, 200)), resize_dimensions(oversized, 1.0));
    }

    #[test]
    fn preserves_1080p_at_two_x() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(1920.0), px(1080.0)));
        assert_eq!(Some((3840, 2160)), resize_dimensions(bounds, 2.0));
    }

    #[test]
    fn caps_extreme_hidpi_area() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(5120.0), px(2880.0)));
        assert_eq!(Some((3840, 2160)), resize_dimensions(bounds, 2.0));
    }

    #[test]
    fn requires_meaningful_resize_delta() {
        assert!(!is_meaningful_delta(Some((1280, 720)), (1284, 726)));
        assert!(is_meaningful_delta(Some((1280, 720)), (1300, 726)));
        assert!(is_meaningful_delta(None, (1280, 720)));
    }

    #[test]
    fn converts_display_scale_to_rdp_percent() {
        assert_eq!(100, super::scale_factor_percent(0.0));
        assert_eq!(200, super::scale_factor_percent(2.0));
        assert_eq!(300, super::scale_factor_percent(4.0));
    }
}
