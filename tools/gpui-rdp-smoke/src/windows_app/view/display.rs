use std::time::{Duration, Instant};

use gpui::Window;
use windows_rdp_host::{WindowsRdpHostLifecycle, WindowsRdpSessionDisplaySettings};

use super::SmokeView;
use crate::windows_app::{log_host_error, physical_viewport_size};

const MIN_DESKTOP_SCALE_PERCENT: f32 = 100.0;
const MAX_DESKTOP_SCALE_PERCENT: f32 = 300.0;
const SCALE_PERCENT_MULTIPLIER: f32 = 100.0;
const DISPLAY_RETRY_DELAY: Duration = Duration::from_millis(500);

#[derive(Clone, Copy)]
struct DisplayRequest {
    reason: &'static str,
    force: bool,
    retry_due: bool,
}

impl SmokeView {
    pub(super) fn synchronize_session_display_settings(
        &mut self,
        window: &Window,
        force: bool,
        reason: &'static str,
    ) -> bool {
        let Some(settings) = session_display_settings(window) else {
            return false;
        };
        let latest_changed = self.latest_session_display_settings != Some(settings);
        self.latest_session_display_settings = Some(settings);
        if !self.login_complete {
            if latest_changed {
                log_display(
                    "cached_before_login",
                    settings,
                    DisplayRequest {
                        reason,
                        force,
                        retry_due: false,
                    },
                );
            }
            return false;
        }
        let Some(request) = self.prepare_display_request(settings, force, reason) else {
            return false;
        };
        self.send_display_request(settings, request)
    }

    fn prepare_display_request(
        &self,
        settings: WindowsRdpSessionDisplaySettings,
        force: bool,
        reason: &'static str,
    ) -> Option<DisplayRequest> {
        let now = Instant::now();
        if self
            .display_retry_after
            .is_some_and(|retry_at| now < retry_at)
        {
            return None;
        }
        let retry_due = self
            .display_retry_after
            .is_some_and(|retry_at| now >= retry_at);
        let request = DisplayRequest {
            reason: if retry_due { "Retry" } else { reason },
            force: force || retry_due,
            retry_due,
        };
        if !request.force && self.last_session_display_settings == Some(settings) {
            return None;
        }
        Some(request)
    }

    fn send_display_request(
        &mut self,
        settings: WindowsRdpSessionDisplaySettings,
        request: DisplayRequest,
    ) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        if session.host.lifecycle() != WindowsRdpHostLifecycle::Open {
            return false;
        }
        if request.retry_due {
            self.display_retry_after = None;
        }

        log_display("request", settings, request);
        match session.host.update_session_display_settings(settings) {
            Ok(()) => {
                self.last_session_display_settings = Some(settings);
                self.display_retry_after = None;
                log_display("success", settings, request);
                true
            }
            Err(error) => {
                self.display_retry_after = Some(Instant::now() + DISPLAY_RETRY_DELAY);
                log_host_error("update_session_display_settings", error);
                log_display("failure", settings, request);
                println!(
                    "display: stage=retry_scheduled reason=Retry epoch={} delay_ms={}",
                    self.display_epoch,
                    DISPLAY_RETRY_DELAY.as_millis()
                );
                false
            }
        }
    }
}

fn session_display_settings(window: &Window) -> Option<WindowsRdpSessionDisplaySettings> {
    let (width, height) = physical_viewport_size(window);
    let desktop_scale_factor = (window.scale_factor() * SCALE_PERCENT_MULTIPLIER)
        .round()
        .clamp(MIN_DESKTOP_SCALE_PERCENT, MAX_DESKTOP_SCALE_PERCENT)
        as u32;
    match WindowsRdpSessionDisplaySettings::viewport(
        width as u32,
        height as u32,
        desktop_scale_factor,
    ) {
        Ok(settings) => Some(settings),
        Err(error) => {
            log_host_error("build_session_display_settings", error);
            None
        }
    }
}

fn log_display(stage: &str, settings: WindowsRdpSessionDisplaySettings, request: DisplayRequest) {
    println!(
        "display: stage={stage} reason={} desktop={}x{} physical={}x{} orientation={} desktop_scale={} device_scale={} force={}",
        request.reason,
        settings.desktop_width(),
        settings.desktop_height(),
        settings.physical_width(),
        settings.physical_height(),
        settings.orientation(),
        settings.desktop_scale_factor(),
        settings.device_scale_factor(),
        request.force,
    );
}
