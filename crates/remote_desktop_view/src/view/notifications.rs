use gpui::{Context, Window};
use gpui_component::{WindowExt, notification::Notification};
use remote_desktop::{
    RemoteDesktopFailure, RemoteDesktopProtocol, RemoteDesktopReconnect,
    RemoteDesktopReconnectReason,
};
use rust_i18n::t;

use super::RemoteDesktopView;

pub(super) struct RemoteDesktopReconnectNotification;
pub(super) struct RemoteDesktopClipboardNotification;
pub(super) struct RemoteDesktopSessionNotification;

impl RemoteDesktopView {
    pub(super) fn notify_reconnecting(
        &self,
        reconnect: RemoteDesktopReconnect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = localized_reconnect_notification(self.options.protocol, reconnect);
        let notification_id = ("remote-desktop-reconnect", cx.entity_id());
        window.defer(cx, move |window, cx| {
            window.push_notification(
                Notification::info(message)
                    .id1::<RemoteDesktopReconnectNotification>(notification_id)
                    .autohide(true),
                cx,
            );
        });
    }

    pub(super) fn notify_session_taken_over(&self, window: &mut Window, cx: &mut Context<Self>) {
        let message = localized_session_taken_over();
        let notification_id = ("remote-desktop-session", cx.entity_id());
        window.defer(cx, move |window, cx| {
            window.push_notification(
                Notification::warning(message)
                    .id1::<RemoteDesktopSessionNotification>(notification_id)
                    .autohide(true),
                cx,
            );
        });
    }

    pub(super) fn notify_vnc_clipboard_ascii_warning(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = localized_vnc_clipboard_ascii_warning();
        let notification_id = ("remote-desktop-clipboard", cx.entity_id());
        window.defer(cx, move |window, cx| {
            window.push_notification(
                Notification::warning(message)
                    .id1::<RemoteDesktopClipboardNotification>(notification_id)
                    .autohide(true),
                cx,
            );
        });
    }

    pub(super) fn notify_clipboard_files_received(
        &self,
        count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = localized_clipboard_files_received(count);
        let notification_id = ("remote-desktop-clipboard", cx.entity_id());
        window.defer(cx, move |window, cx| {
            window.push_notification(
                Notification::success(message)
                    .id1::<RemoteDesktopClipboardNotification>(notification_id)
                    .autohide(true),
                cx,
            );
        });
    }

    pub(super) fn notify_clipboard_transfer_failed(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = localized_clipboard_transfer_failed();
        let notification_id = ("remote-desktop-clipboard", cx.entity_id());
        window.defer(cx, move |window, cx| {
            window.push_notification(
                Notification::error(message)
                    .id1::<RemoteDesktopClipboardNotification>(notification_id)
                    .autohide(true),
                cx,
            );
        });
    }

    pub(super) fn notify_clipboard_files_invalid(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = localized_clipboard_files_invalid();
        let notification_id = ("remote-desktop-clipboard", cx.entity_id());
        window.defer(cx, move |window, cx| {
            window.push_notification(
                Notification::error(message)
                    .id1::<RemoteDesktopClipboardNotification>(notification_id)
                    .autohide(true),
                cx,
            );
        });
    }
}

pub(super) fn localized_reconnect_notification(
    protocol: RemoteDesktopProtocol,
    reconnect: RemoteDesktopReconnect,
) -> String {
    let locale = rust_i18n::locale();
    localized_reconnect_notification_for_locale(locale.as_ref(), protocol, reconnect)
}

fn localized_reconnect_notification_for_locale(
    locale: &str,
    protocol: RemoteDesktopProtocol,
    reconnect: RemoteDesktopReconnect,
) -> String {
    let reason = match reconnect.reason {
        RemoteDesktopReconnectReason::DisplayUpdate => t!(
            "RemoteDesktop.reconnect_reason_display_update",
            locale = locale
        ),
        RemoteDesktopReconnectReason::SessionError => t!(
            "RemoteDesktop.reconnect_reason_session_error",
            locale = locale
        ),
        RemoteDesktopReconnectReason::ConnectionLost => t!(
            "RemoteDesktop.reconnect_reason_connection_lost",
            locale = locale
        ),
        RemoteDesktopReconnectReason::Manual => {
            return t!(
                "RemoteDesktop.reconnect_notification_manual",
                locale = locale,
                protocol = protocol.label()
            )
            .to_string();
        }
    };
    let Some(seconds) = reconnect.delay_secs else {
        return t!(
            "RemoteDesktop.reconnect_notification_manual",
            locale = locale,
            protocol = protocol.label()
        )
        .to_string();
    };

    t!(
        "RemoteDesktop.reconnect_notification",
        locale = locale,
        protocol = protocol.label(),
        reason = reason,
        seconds = seconds
    )
    .to_string()
}

pub(super) fn localized_failure_message(failure: &RemoteDesktopFailure) -> String {
    let locale = rust_i18n::locale();
    localized_failure_message_for_locale(locale.as_ref(), failure)
}

pub(super) fn localized_failure_message_for_locale(
    locale: &str,
    failure: &RemoteDesktopFailure,
) -> String {
    match failure {
        RemoteDesktopFailure::AuthenticationFailed => {
            t!("RemoteDesktop.failure_authentication", locale = locale).to_string()
        }
        RemoteDesktopFailure::SessionTakenOver => localized_session_taken_over_for_locale(locale),
        RemoteDesktopFailure::HostUnreachable => {
            t!("RemoteDesktop.failure_host_unreachable", locale = locale).to_string()
        }
        RemoteDesktopFailure::ServerEndedSession => {
            t!("RemoteDesktop.failure_server_ended", locale = locale).to_string()
        }
        RemoteDesktopFailure::ConnectionFailed => {
            t!("RemoteDesktop.failure_generic", locale = locale).to_string()
        }
        RemoteDesktopFailure::ProviderVersion {
            protocol,
            installed,
            required,
            invalid,
        } => localized_provider_version_failure(locale, *protocol, installed, required, *invalid),
    }
}

fn localized_provider_version_failure(
    locale: &str,
    protocol: RemoteDesktopProtocol,
    installed: &str,
    required: &str,
    invalid: bool,
) -> String {
    if invalid {
        t!(
            "RemoteDesktop.provider_version_invalid",
            locale = locale,
            protocol = protocol.label(),
            installed = installed,
            required = required
        )
        .to_string()
    } else {
        t!(
            "RemoteDesktop.provider_version_too_old",
            locale = locale,
            protocol = protocol.label(),
            installed = installed,
            required = required
        )
        .to_string()
    }
}

fn localized_session_taken_over() -> String {
    let locale = rust_i18n::locale();
    localized_session_taken_over_for_locale(locale.as_ref())
}

pub(super) fn localized_session_taken_over_for_locale(locale: &str) -> String {
    t!(
        "RemoteDesktop.session_taken_over_notification",
        locale = locale
    )
    .to_string()
}

fn localized_vnc_clipboard_ascii_warning() -> String {
    let locale = rust_i18n::locale();
    localized_vnc_clipboard_ascii_warning_for_locale(locale.as_ref())
}

fn localized_vnc_clipboard_ascii_warning_for_locale(locale: &str) -> String {
    t!("RemoteDesktop.vnc_clipboard_ascii_only", locale = locale).to_string()
}

fn localized_clipboard_files_received(count: usize) -> String {
    let locale = rust_i18n::locale();
    localized_clipboard_files_received_for_locale(locale.as_ref(), count)
}

fn localized_clipboard_files_received_for_locale(locale: &str, count: usize) -> String {
    t!(
        "RemoteDesktop.clipboard_files_received",
        locale = locale,
        count = count
    )
    .to_string()
}

fn localized_clipboard_transfer_failed() -> String {
    let locale = rust_i18n::locale();
    t!("RemoteDesktop.clipboard_transfer_failed", locale = locale).to_string()
}

fn localized_clipboard_files_invalid() -> String {
    let locale = rust_i18n::locale();
    t!("RemoteDesktop.clipboard_files_invalid", locale = locale).to_string()
}

#[cfg(test)]
#[path = "notifications_tests.rs"]
mod tests;
