use one_ui::IconSize;
use super::*;

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
        let view = cx.entity();
        let menu_view = view.clone();
        // WSL 发行版在启动时后台识别（仅 Windows）；克隆快照供菜单闭包使用。
        #[cfg(target_os = "windows")]
        let wsl_distributions = self.wsl_distributions.clone();
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
                        this.add_terminal_tab_with_profile(default_kind, window, cx);
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
                        menu.item(PopupMenuItem::new(label).checked(checked).on_click(
                            move |_, window, cx| {
                                view.update(cx, |home, cx| match target.clone() {
                                    LocalTerminalLaunchTarget::Builtin(kind) => {
                                        home.add_terminal_tab_with_profile(kind, window, cx)
                                    }
                                    LocalTerminalLaunchTarget::Custom(profile) => home
                                        .add_terminal_tab_with_custom_profile(profile, window, cx),
                                });
                            },
                        ))
                    });
                #[cfg(target_os = "windows")]
                let menu =
                    append_wsl_distributions(menu, wsl_distributions.clone(), menu_view.clone());
                menu
            })
            .into_any_element()
    }
}

/// 在本地终端菜单末尾追加 WSL 发行版区段（feigeCode/navop#182）。
///
/// 识别完成前与识别无结果时整段隐藏，避免占位闪烁；
/// 区段末尾提供「重新识别」入口，点击后重新后台识别（下次打开菜单生效）。
#[cfg(target_os = "windows")]
fn append_wsl_distributions(
    menu: gpui_component::menu::PopupMenu,
    distributions: Option<Arc<Vec<terminal::WslDistribution>>>,
    home: Entity<HomePage>,
) -> gpui_component::menu::PopupMenu {
    let Some(distributions) = distributions else {
        return menu;
    };
    if distributions.is_empty() {
        return menu;
    }
    let menu = menu
        .separator()
        .item(PopupMenuItem::label(t!("Home.wsl_distributions_section")));
    let menu = distributions.iter().fold(menu, |menu, distribution| {
        let home = home.clone();
        let distro = distribution.name.clone();
        let label = wsl_distro_item_label(distribution);
        menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
            home.update(cx, |home, cx| {
                home.add_terminal_tab_with_wsl_distro(distro.clone(), window, cx)
            });
        }))
    });
    let home = home.clone();
    menu.separator().item(
        PopupMenuItem::new(t!("Home.wsl_distributions_refresh")).on_click(move |_, _, cx| {
            home.update(cx, |home, cx| home.load_wsl_distributions(cx));
        }),
    )
}

/// 发行版条目文案：默认发行版追加「默认」标记，便于对齐 MobaXterm 的辨识体验。
#[cfg(target_os = "windows")]
fn wsl_distro_item_label(distribution: &terminal::WslDistribution) -> String {
    if distribution.is_default {
        format!(
            "{} · {}",
            distribution.name,
            t!("Home.wsl_distributions_default")
        )
    } else {
        distribution.name.clone()
    }
}
