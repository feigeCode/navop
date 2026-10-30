use super::*;
use domain_api::{TerminalProfileDescriptor, TerminalProfileKind};
use one_core::settings::LocalTerminalProfileKind;
use one_ui::IconSize;

impl HomePage {
    pub(super) fn render_local_terminal_button(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let default_kind = effective_kind(
            AppSettings::global(cx).local_terminal_profile.kind,
            cfg!(target_os = "windows"),
        );
        let profile_settings = AppSettings::global(cx).local_terminal_profile.clone();
        let wsl_distributions = app_kit::registry()
            .shell_extras()
            .and_then(|extras| extras.wsl_distributions(cx))
            .unwrap_or_default();
        let view = cx.entity();
        let menu_view = view.clone();
        DropdownButton::new("local-terminal-dropdown")
            .flex_shrink_0()
            .button(
                Button::new("local-terminal-button")
                    .icon(
                        Icon::new(IconName::SquareTerminal)
                            .mono()
                            .with_size(IconSize::Small),
                    )
                    .when(window.bounds().size.width > px(1100.0), |button| {
                        button.label(t!("Home.local_terminal").to_string())
                    })
                    .tooltip(super::home_shortcuts::terminal_tooltip(cx))
                    .on_click(window.listener_for(&view, move |this, _, window, cx| {
                        this.open_local_terminal_tab(
                            Some(TerminalProfileDescriptor {
                                kind: domain_terminal_kind(default_kind),
                                id: None,
                            }),
                            window,
                            cx,
                        );
                    })),
            )
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let menu = launch_options(cfg!(target_os = "windows"), &profile_settings)
                    .into_iter()
                    .fold(menu, |menu, (target, label)| {
                        let view = menu_view.clone();
                        let checked = launch_target_is_default(
                            &target,
                            &profile_settings,
                            cfg!(target_os = "windows"),
                        );
                        let descriptor = launch_target_descriptor(&target);
                        menu.item(PopupMenuItem::new(label).checked(checked).on_click(
                            move |_, window, cx| {
                                let descriptor = descriptor.clone();
                                view.update(cx, |home, cx| {
                                    home.open_local_terminal_tab(Some(descriptor), window, cx)
                                });
                            },
                        ))
                    });
                append_wsl_distributions(menu, wsl_distributions.clone(), menu_view.clone())
            })
            .into_any_element()
    }
}

/// 菜单项到领域意图的映射：内置 profile 只带 kind，自定义 profile 带稳定 id。
fn launch_target_descriptor(target: &LocalTerminalLaunchTarget) -> TerminalProfileDescriptor {
    match target {
        LocalTerminalLaunchTarget::Builtin(kind) => TerminalProfileDescriptor {
            kind: domain_terminal_kind(*kind),
            id: None,
        },
        LocalTerminalLaunchTarget::Custom(profile) => TerminalProfileDescriptor {
            kind: TerminalProfileKind::Custom,
            id: Some(profile.id.clone()),
        },
    }
}

/// 在本地终端菜单末尾追加 WSL 发行版区段（feigeCode/navop#182）。
///
/// 发行版列表由 shell 领域在后台识别并通过 `ShellExtras` 暴露；
/// 识别完成前与识别无结果时整段隐藏，避免占位闪烁。
fn append_wsl_distributions(
    menu: gpui_component::menu::PopupMenu,
    distributions: Vec<domain_api::WslDistributionInfo>,
    home: Entity<HomePage>,
) -> gpui_component::menu::PopupMenu {
    if distributions.is_empty() {
        return menu;
    }
    let menu = menu
        .separator()
        .item(PopupMenuItem::label(t!("Home.wsl_distributions_section")));
    let menu = distributions.iter().fold(menu, |menu, distribution| {
        let home = home.clone();
        let label = wsl_distro_item_label(&distribution.name, distribution.is_default);
        let distro = distribution.name.to_string();
        menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
            let distro = distro.clone();
            home.update(cx, |home, cx| {
                home.open_local_terminal_tab(
                    Some(TerminalProfileDescriptor {
                        kind: TerminalProfileKind::Wsl,
                        id: Some(distro),
                    }),
                    window,
                    cx,
                )
            });
        }))
    });
    let home = home.clone();
    menu.separator().item(
        PopupMenuItem::new(t!("Home.wsl_distributions_refresh")).on_click(move |_, _, cx| {
            if let Some(extras) = app_kit::registry().shell_extras() {
                extras.refresh_wsl_distributions(cx);
            }
            home.update(cx, |_, cx| cx.notify());
        }),
    )
}

/// 发行版条目文案：默认发行版追加「默认」标记，便于对齐 MobaXterm 的辨识体验。
fn wsl_distro_item_label(name: &str, is_default: bool) -> String {
    if is_default {
        format!("{name} · {}", t!("Home.wsl_distributions_default"))
    } else {
        name.to_string()
    }
}

/// 本地终端 profile 的领域无关描述映射。
fn domain_terminal_kind(kind: LocalTerminalProfileKind) -> TerminalProfileKind {
    match kind {
        LocalTerminalProfileKind::System => TerminalProfileKind::System,
        LocalTerminalProfileKind::PowerShell => TerminalProfileKind::PowerShell,
        LocalTerminalProfileKind::Cmd => TerminalProfileKind::Cmd,
        LocalTerminalProfileKind::Wsl => TerminalProfileKind::Wsl,
        LocalTerminalProfileKind::GitBash => TerminalProfileKind::GitBash,
        LocalTerminalProfileKind::Zsh => TerminalProfileKind::Zsh,
        LocalTerminalProfileKind::Bash => TerminalProfileKind::Bash,
        LocalTerminalProfileKind::Fish => TerminalProfileKind::Fish,
        LocalTerminalProfileKind::Nushell => TerminalProfileKind::Nushell,
        LocalTerminalProfileKind::Custom => TerminalProfileKind::Custom,
    }
}
