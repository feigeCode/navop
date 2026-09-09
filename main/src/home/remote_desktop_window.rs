use gpui::{AppContext, Focusable};
use one_core::popup_window::{PopupWindowOptions, open_popup_window};
#[cfg(target_os = "windows")]
use one_core::storage::RemoteDesktopParams;
use remote_desktop::RemoteDesktopConnectionOptions;
use remote_desktop_view::{RemoteDesktopView, RemoteDesktopViewConfig};
use rust_i18n::t;

#[cfg(any(target_os = "windows", test))]
mod mstsc_credentials;

#[cfg(any(target_os = "windows", test))]
use mstsc_credentials::{MstscCredentialInput, mstsc_credentials};

const DEFAULT_WINDOW_WIDTH: f32 = 1280.0;
const DEFAULT_WINDOW_HEIGHT: f32 = 800.0;
const MIN_WINDOW_WIDTH: f32 = 640.0;
const MIN_WINDOW_HEIGHT: f32 = 480.0;

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MstscLaunchPlan {
    pub(crate) program: &'static str,
    pub(crate) args: Vec<String>,
}

#[cfg(any(target_os = "windows", test))]
fn format_host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn mstsc_launch_plan(host: &str, port: u16) -> MstscLaunchPlan {
    MstscLaunchPlan {
        program: "mstsc.exe",
        args: vec![
            format!("/v:{}", format_host_port(host, port)),
            "/f".to_string(),
        ],
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn launch_mstsc_fullscreen(params: &RemoteDesktopParams) -> anyhow::Result<()> {
    let plan = mstsc_launch_plan(&params.host, params.port);
    let credentials = mstsc_credentials(MstscCredentialInput {
        host: &params.host,
        port: params.port,
        username: params.username.as_deref(),
        password: params.password.as_deref(),
        domain: params.domain.as_deref(),
    });
    let lease = credentials
        .as_ref()
        .map(mstsc_credentials::store_temporary)
        .transpose()?;
    let child = std::process::Command::new(plan.program)
        .args(&plan.args)
        .spawn()?;
    if let Some(lease) = lease {
        std::thread::spawn(move || lease.restore_after(child));
    }
    Ok(())
}

fn remote_desktop_window_options(title: String) -> PopupWindowOptions {
    PopupWindowOptions::new(title)
        .size(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT)
        .min_width(MIN_WINDOW_WIDTH)
        .min_height(MIN_WINDOW_HEIGHT)
        .fullscreen(true)
        .hide_titlebar_when_fullscreen(true)
        .fullscreen_hint(t!("Connection.fullscreen_exit_hint").to_string())
}

pub(crate) fn open_remote_desktop_fullscreen_window(
    options: RemoteDesktopConnectionOptions,
    title: String,
    cx: &mut gpui::App,
) {
    open_popup_window(
        remote_desktop_window_options(title.clone()),
        move |window, cx| {
            let view = cx.new(|cx| {
                RemoteDesktopView::new(
                    RemoteDesktopViewConfig {
                        options,
                        title,
                        tab_index: None,
                    },
                    window.window_handle(),
                    cx,
                )
            });
            view.read(cx).focus_handle(cx).focus(window, cx);
            view
        },
        None,
        cx,
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn fullscreen_window_options_keep_internal_popup_fallback() {
        let options = super::remote_desktop_window_options("RDP".to_string());

        assert!(options.fullscreen);
        assert!(options.hide_titlebar_when_fullscreen);
        assert!(options.fullscreen_hint.is_some());
    }

    #[test]
    fn mstsc_launch_plan_uses_native_client_in_fullscreen() {
        let plan = super::mstsc_launch_plan("rdp.example.com", 3389);

        assert_eq!(plan.program, "mstsc.exe");
        assert_eq!(plan.args, vec!["/v:rdp.example.com:3389", "/f"]);
    }

    #[test]
    fn mstsc_launch_plan_brackets_ipv6_destinations() {
        let plan = super::mstsc_launch_plan("::1", 3390);

        assert_eq!(plan.args[0], "/v:[::1]:3390");
    }

    #[test]
    fn mstsc_launch_plan_preserves_bracketed_ipv6_destinations() {
        let plan = super::mstsc_launch_plan("[::1]", 3390);

        assert_eq!(plan.args[0], "/v:[::1]:3390");
    }

    #[test]
    fn mstsc_credentials_use_termsrv_target_and_domain_username() {
        let credentials = super::mstsc_credentials(super::MstscCredentialInput {
            host: "rdp.example.com",
            port: 3390,
            username: Some("operator"),
            password: Some("secret"),
            domain: Some("ACME"),
        })
        .expect("complete credentials should be prepared");

        assert_eq!(credentials.target, "TERMSRV/rdp.example.com:3390");
        assert_eq!(credentials.username, "ACME\\operator");
        assert_eq!(credentials.password, "secret");
    }

    #[test]
    fn mstsc_credentials_require_username_and_password() {
        let input = |username, password| super::MstscCredentialInput {
            host: "server",
            port: 3389,
            username,
            password,
            domain: None,
        };
        assert!(super::mstsc_credentials(input(None, Some("secret"))).is_none());
        assert!(super::mstsc_credentials(input(Some("operator"), None)).is_none());
    }

    #[test]
    fn mstsc_launch_plan_does_not_expose_credentials() {
        let plan = super::mstsc_launch_plan("rdp.example.com", 3389);

        assert_eq!(plan.args, vec!["/v:rdp.example.com:3389", "/f"]);
        assert!(!plan.args.iter().any(|arg| {
            arg.contains("cmd")
                || arg.contains("powershell")
                || arg.contains("username")
                || arg.contains("password")
                || arg.contains("ClearTextPassword")
                || arg.contains("cmdkey")
        }));
    }
}
