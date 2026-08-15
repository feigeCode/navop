use std::time::Instant;

use gpui::{Bounds, Pixels, point, px};

use super::RemoteDesktopView;
use super::windows_native_display::{WindowsNativeDisplayRequest, WindowsNativeViewportSettings};

impl RemoteDesktopView {
    pub(super) fn observe_windows_native_viewport(
        &mut self,
        bounds: Bounds<Pixels>,
        scale_factor: f32,
    ) {
        if !super::windows_native_policy::uses_dynamic_display_updates(&self.options.rdp) {
            tracing::debug!("display: stage=observe skipped fixed desktop mode");
            return;
        }
        let Some(physical) = super::windows_native::logical_bounds_to_physical(
            bounds,
            point(px(0.0), px(0.0)),
            scale_factor,
        ) else {
            tracing::warn!(
                scale_factor,
                "display: stage=observe invalid native viewport"
            );
            return;
        };
        let settings = physical_viewport_settings(physical, scale_factor);
        let Some(settings) = settings else {
            return;
        };
        self.windows_native_display
            .observe(settings, Instant::now());
    }

    pub(super) fn flush_windows_native_display_settings(&mut self, now: Instant) {
        let Some(request) = self.windows_native_display.take_request(now) else {
            return;
        };
        log_display_request(request);
        if !self.windows_native_display_target_is_open(request) {
            log_display_target_unavailable(request);
            self.windows_native_display.suspend();
            return;
        }
        let settings = windows_rdp_host::WindowsRdpSessionDisplaySettings::viewport(
            request.settings.width,
            request.settings.height,
            request.settings.desktop_scale_factor,
        );
        let result = settings.and_then(|settings| {
            self.windows_native
                .as_mut()
                .expect("validated native display target")
                .update_session_display_settings(settings)
        });
        match result {
            Ok(()) => {
                self.windows_native_display.request_succeeded(request);
                log_display_success(request);
            }
            Err(error) => {
                self.windows_native_display.request_failed(request, now);
                log_display_failure(request, error);
            }
        }
    }

    fn windows_native_display_target_is_open(&self, request: WindowsNativeDisplayRequest) -> bool {
        self.windows_native
            .as_ref()
            .is_some_and(|native| native.generation() == request.generation && native.is_open())
    }
}

fn physical_viewport_settings(
    physical: super::windows_native::Win32ClientPhysicalBounds,
    scale_factor: f32,
) -> Option<WindowsNativeViewportSettings> {
    let (Ok(width), Ok(height)) = (
        u32::try_from(physical.width),
        u32::try_from(physical.height),
    ) else {
        tracing::warn!(
            width = physical.width,
            height = physical.height,
            "display: stage=observe invalid physical dimensions"
        );
        return None;
    };
    if width == 0 || height == 0 {
        return None;
    }
    Some(WindowsNativeViewportSettings {
        width,
        height,
        desktop_scale_factor: super::resize::scale_factor_percent(scale_factor),
    })
}

fn log_display_request(request: WindowsNativeDisplayRequest) {
    tracing::info!(
        reason = ?request.reason,
        generation = request.generation,
        width = request.settings.width,
        height = request.settings.height,
        desktop_scale = request.settings.desktop_scale_factor,
        "display: stage=request"
    );
}

fn log_display_success(request: WindowsNativeDisplayRequest) {
    tracing::info!(
        reason = ?request.reason,
        generation = request.generation,
        width = request.settings.width,
        height = request.settings.height,
        desktop_scale = request.settings.desktop_scale_factor,
        "display: stage=success"
    );
}

fn log_display_failure(
    request: WindowsNativeDisplayRequest,
    error: windows_rdp_host::WindowsRdpHostError,
) {
    tracing::warn!(
        reason = ?request.reason,
        generation = request.generation,
        width = request.settings.width,
        height = request.settings.height,
        desktop_scale = request.settings.desktop_scale_factor,
        ?error,
        "display: stage=failure"
    );
}

fn log_display_target_unavailable(request: WindowsNativeDisplayRequest) {
    tracing::warn!(
        reason = ?request.reason,
        generation = request.generation,
        width = request.settings.width,
        height = request.settings.height,
        desktop_scale = request.settings.desktop_scale_factor,
        error = "native target unavailable",
        "display: stage=failure"
    );
}
