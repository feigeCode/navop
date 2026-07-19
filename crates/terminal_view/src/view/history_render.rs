use super::*;

impl TerminalView {
    pub(super) fn render_history_prompt_overlay(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.history_prompt_enabled(cx) || !self.history_prompt.is_active() {
            return None;
        }

        if !self.history_prompt.dropdown_visible() {
            return None;
        }

        let search_mode = self.history_prompt.mode() == HistoryPromptMode::Search;
        let matches = self.history_prompt.matches().to_vec();
        if !search_mode && matches.is_empty() {
            return None;
        }

        let (cursor_line, cursor_col) = {
            let terminal = self.terminal.read(cx);
            let term = terminal.term().lock();
            let cursor = term.grid().cursor.point;
            let display_offset = term.grid().display_offset() as i32;
            (cursor.line.0 + display_offset, cursor.column.0)
        };

        if cursor_line < 0 {
            return None;
        }

        let selected_index = self.history_prompt.selected_index();
        let search_query = self.history_prompt.query_input().to_string();
        let view = cx.entity().clone();
        let overlay_bounds = history_prompt_overlay_bounds(self.terminal_bounds);
        let ghost_left = self.cell_width * cursor_col as f32;
        let ghost_top = self.line_height * cursor_line as f32;
        let dropdown_origin = history_prompt_dropdown_origin(
            overlay_bounds,
            self.cell_width,
            self.line_height,
            cursor_line,
            cursor_col,
            matches.len(),
            search_mode,
        );
        let ghost_suffix = if search_mode {
            None
        } else {
            self.history_prompt.selected_match().map(|selected| {
                selected
                    .strip_prefix(self.history_prompt.query_input())
                    .unwrap_or_default()
                    .to_string()
            })
        };

        Some(
            div()
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .right(px(0.0))
                .bottom(px(0.0))
                .when_some(ghost_suffix, |this, ghost_suffix| {
                    this.child(
                        div()
                            .absolute()
                            .left(ghost_left)
                            .top(ghost_top)
                            .text_color(self.current_theme.foreground.opacity(0.35))
                            .text_size(self.font_size)
                            .child(ghost_suffix),
                    )
                })
                .child(
                    div()
                        .absolute()
                        .left(dropdown_origin.x)
                        .top(dropdown_origin.y)
                        .min_w(px(HISTORY_PROMPT_DROPDOWN_MIN_WIDTH))
                        .max_w(px(HISTORY_PROMPT_DROPDOWN_MAX_WIDTH))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .px_2()
                        .py_2()
                        .rounded_md()
                        .bg(history_prompt_dropdown_background(
                            self.current_theme.background,
                        ))
                        .border_1()
                        .border_color(self.current_theme.foreground.opacity(0.18))
                        .when(search_mode, |this| {
                            this.child(
                                div()
                                    .px_2()
                                    .pb_1()
                                    .text_color(self.current_theme.foreground.opacity(0.7))
                                    .text_size(px(11.0))
                                    .child(format!("history search: {}", search_query)),
                            )
                        })
                        .children(matches.into_iter().enumerate().map(|(index, command)| {
                            let active = selected_index == Some(index);
                            div()
                                .on_mouse_move({
                                    let view = view.clone();
                                    move |_, _, cx| {
                                        cx.stop_propagation();
                                        view.update(cx, |this, cx| {
                                            this.select_history_prompt_match(index, cx);
                                        });
                                    }
                                })
                                .on_mouse_down(MouseButton::Left, {
                                    let view = view.clone();
                                    move |_, _, cx| {
                                        cx.stop_propagation();
                                        view.update(cx, |this, cx| {
                                            this.select_history_prompt_match(index, cx);
                                            let _ = this.try_accept_history_prompt(cx);
                                        });
                                    }
                                })
                                .cursor_pointer()
                                .px_3()
                                .py_1p5()
                                .rounded_sm()
                                .bg(if active {
                                    history_prompt_active_background(self.current_theme.foreground)
                                } else {
                                    transparent_black()
                                })
                                .text_color(if active {
                                    self.current_theme.foreground
                                } else {
                                    self.current_theme.foreground.opacity(0.8)
                                })
                                .text_size(px(12.0))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(Icon::new(IconName::Calendar).xsmall().text_color(
                                            self.current_theme.foreground.opacity(if active {
                                                0.85
                                            } else {
                                                0.55
                                            }),
                                        ))
                                        .child(
                                            div()
                                                .flex_1()
                                                .overflow_x_hidden()
                                                .text_ellipsis()
                                                .whitespace_nowrap()
                                                .child(command),
                                        ),
                                )
                        })),
                )
                .into_any_element(),
        )
    }
}
