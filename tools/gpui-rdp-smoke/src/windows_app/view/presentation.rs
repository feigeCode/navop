use gpui::Window;
use windows_rdp_host::WindowsRdpHostLifecycle;

use super::SmokeView;
use crate::windows_app::{log_host_error, physical_viewport_size};

impl SmokeView {
    pub(super) fn present_connected(&mut self, window: &Window, reason: &'static str) {
        if self.refresh_native_presentation(window, reason) {
            self.focus_native_presentation(reason);
            self.log_composition_diagnostics(reason);
        }
    }

    pub(super) fn present_login_complete(&mut self, window: &Window, reason: &'static str) {
        if !self.refresh_native_presentation(window, reason) {
            return;
        }
        self.synchronize_session_display_settings(window, true, reason);
        self.focus_native_presentation(reason);
        self.log_composition_diagnostics(reason);
    }

    fn refresh_native_presentation(&mut self, window: &Window, reason: &'static str) -> bool {
        let bounds = physical_viewport_size(window);
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        if session.host.lifecycle() != WindowsRdpHostLifecycle::Open {
            return false;
        }
        if let Err(error) = session.overlay.refresh((0, 0, bounds.0, bounds.1)) {
            eprintln!("ERROR: stage=refresh_connected_overlay error={error}");
            return false;
        }
        if let Err(error) = session.host.set_bounds(0, 0, bounds.0, bounds.1) {
            log_host_error("refresh_connected_bounds", error);
            return false;
        }
        if let Err(error) = session.host.set_visible(true) {
            log_host_error("refresh_connected_visible", error);
            return false;
        }
        self.last_bounds = Some(bounds);
        println!("presentation: refreshed reason={reason}");
        true
    }

    fn focus_native_presentation(&mut self, reason: &'static str) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if let Err(error) = session.host.focus() {
            log_host_error("refresh_connected_focus_best_effort", error);
        } else {
            println!("focus: success reason={reason}");
        }
    }

    pub(super) fn log_composition_diagnostics(&self, reason: &'static str) {
        if let Some(session) = self.session.as_ref() {
            session.overlay.log_composition_diagnostics(reason);
        }
    }
}
