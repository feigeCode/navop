use super::*;

fn search_toggle_id(toggle: TerminalSearchToggle) -> &'static str {
    match toggle {
        TerminalSearchToggle::MatchCase => "terminal-search-match-case",
        TerminalSearchToggle::WholeWord => "terminal-search-whole-word",
        TerminalSearchToggle::Regex => "terminal-search-regex",
    }
}

fn search_toggle_label(toggle: TerminalSearchToggle) -> &'static str {
    match toggle {
        TerminalSearchToggle::MatchCase => "Cc",
        TerminalSearchToggle::WholeWord => "W",
        TerminalSearchToggle::Regex => ".*",
    }
}

fn search_toggle_tooltip(toggle: TerminalSearchToggle, invalid_regex: bool) -> String {
    match toggle {
        TerminalSearchToggle::MatchCase => t!("Settings.search_match_case").to_string(),
        TerminalSearchToggle::WholeWord => t!("Settings.search_whole_word").to_string(),
        TerminalSearchToggle::Regex if invalid_regex => {
            t!("Settings.search_regex_invalid").to_string()
        }
        TerminalSearchToggle::Regex => t!("Settings.search_regex").to_string(),
    }
}

impl TerminalView {
    fn render_search_toggle(
        &self,
        toggle: TerminalSearchToggle,
        options: TerminalSearchOptions,
        invalid_regex: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let next = options.toggled(toggle);
        let disabled = toggle == TerminalSearchToggle::WholeWord && options.regex;
        Button::new(search_toggle_id(toggle))
            .label(search_toggle_label(toggle))
            .xsmall()
            .compact()
            .ghost()
            .selected(options.is_on(toggle))
            .toggled(options.is_on(toggle))
            .disabled(disabled)
            .tooltip(search_toggle_tooltip(toggle, invalid_regex))
            .on_click(cx.listener(move |this, _, _, cx| {
                let _ = this.set_search_options(next);
                cx.notify();
            }))
    }

    pub(super) fn render_terminal_search_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = self.current_theme.colors();
        let (options, search_active, invalid_regex) = self
            .addon_manager
            .get_as::<SearchAddon>("search")
            .map(|search| {
                (
                    search.options(),
                    search.is_active(),
                    search.has_invalid_regex(),
                )
            })
            .unwrap_or_default();
        h_flex()
            .id("terminal-search-bar")
            .key_context(TERMINAL_SEARCH_CONTEXT)
            .absolute()
            .top(px(20.0))
            .right(px(20.0))
            .occlude()
            .w(px(460.0))
            .max_w(px(560.0))
            .gap_1()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(colors.background)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_action(cx.listener(Self::close_terminal_search))
            .on_action(cx.listener(Self::search_forward))
            .on_action(cx.listener(Self::search_backward))
            .child(
                Input::new(&self.search_input)
                    .small()
                    .w_full()
                    .prefix(Icon::new(IconName::Search).xsmall())
                    .suffix(
                        h_flex()
                            .gap_1()
                            .child(self.render_search_toggle(
                                TerminalSearchToggle::MatchCase,
                                options,
                                invalid_regex,
                                cx,
                            ))
                            .child(self.render_search_toggle(
                                TerminalSearchToggle::WholeWord,
                                options,
                                invalid_regex,
                                cx,
                            ))
                            .child(self.render_search_toggle(
                                TerminalSearchToggle::Regex,
                                options,
                                invalid_regex,
                                cx,
                            )),
                    ),
            )
            .child(
                Button::new("terminal-search-prev")
                    .icon(IconName::ChevronUp)
                    .ghost()
                    .small()
                    .disabled(!search_active)
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.search_backward_internal(cx);
                    })),
            )
            .child(
                Button::new("terminal-search-next")
                    .icon(IconName::ChevronDown)
                    .ghost()
                    .small()
                    .disabled(!search_active)
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.search_forward_internal(cx);
                    })),
            )
            .child(
                Button::new("terminal-search-close")
                    .icon(IconName::Close)
                    .ghost()
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_search(window, cx);
                    })),
            )
            .into_any_element()
    }
}
