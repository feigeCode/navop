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
        let custom_program = AppSettings::global(cx)
            .local_terminal_profile
            .custom_program
            .clone();
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
                launch_options(cfg!(target_os = "windows"), &custom_program)
                    .into_iter()
                    .fold(menu, |menu, (kind, label)| {
                        let view = menu_view.clone();
                        menu.item(
                            PopupMenuItem::new(label)
                                .checked(kind == default_kind)
                                .on_click(move |_, window, cx| {
                                    view.update(cx, |home, cx| {
                                        home.add_terminal_tab_with_profile(kind, window, cx);
                                    });
                                }),
                        )
                    })
            })
            .into_any_element()
    }
}
