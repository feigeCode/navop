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
        DropdownButton::new("local-terminal-dropdown")
            .button(
                Button::new("local-terminal-button")
                    .icon(IconName::SquareTerminalColor.color())
                    .label(t!("Home.local_terminal").to_string())
                    .tooltip(t!("Home.local_terminal_tooltip").to_string())
                    .on_click(window.listener_for(&view, move |this, _, window, cx| {
                        this.add_terminal_tab_with_profile(default_kind, window, cx);
                    })),
            )
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                launch_options(cfg!(target_os = "windows"), &profile_settings)
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
                    })
            })
            .into_any_element()
    }
}
