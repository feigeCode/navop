use super::*;

impl TerminalView {
    pub(super) fn resize_if_needed(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        let cols = (bounds.size.width / self.cell_width).floor() as usize;
        let rows = (bounds.size.height / self.line_height).floor() as usize;

        let cols = cols.max(1);
        let rows = rows.max(1);

        let new_size = (cols, rows);
        if self.last_size != Some(new_size) {
            tracing::info!(
                target: "terminal_residue",
                old = ?self.last_size,
                new = ?new_size,
                bounds_w = ?bounds.size.width,
                bounds_h = ?bounds.size.height,
                cell_width = ?self.cell_width,
                line_height = ?self.line_height,
                "resize_if_needed -> Terminal::resize"
            );
            self.last_size = Some(new_size);
            self.terminal.update(cx, |terminal, _| {
                terminal.resize(
                    cols,
                    rows,
                    f32::from(bounds.size.width).round() as u16,
                    f32::from(bounds.size.height).round() as u16,
                );
            });
        }
    }

    pub(super) fn get_addon_line_text(
        &self,
        screen_line: usize,
        column: usize,
        cx: &Context<Self>,
    ) -> AddonLineText {
        let term = self.terminal.read(cx).term().lock();
        let grid = term.grid();
        let display_offset = grid.display_offset();
        let grid_line = screen_line as i32 - display_offset as i32;
        let min_line = -(term.history_size() as i32);
        let max_line = term.screen_lines() as i32 - 1;

        if grid_line < min_line || grid_line > max_line {
            return AddonLineText {
                text: String::new(),
                column,
                screen_line,
            };
        }

        let first_line = first_wrapped_grid_line(grid_line, min_line, |line| {
            grid[Line(line)][Column(term.columns() - 1)]
                .flags
                .contains(Flags::WRAPLINE)
        });
        let last_line = last_wrapped_grid_line(grid_line, max_line, |line| {
            grid[Line(line)][Column(term.columns() - 1)]
                .flags
                .contains(Flags::WRAPLINE)
        });
        let line_text = |line| {
            let text: String = grid[Line(line)][..].iter().map(|cell| cell.c).collect();
            text.trim_end_matches(|c: char| c == ' ' || c == '\0')
                .to_string()
        };
        let segments = (first_line..=last_line)
            .map(|line| {
                WrappedLineSegment::new(
                    line_text(line),
                    line < last_line
                        && grid[Line(line)][Column(term.columns() - 1)]
                            .flags
                            .contains(Flags::WRAPLINE),
                )
            })
            .collect::<Vec<_>>();

        wrapped_addon_line_text(
            &segments,
            (grid_line - first_line) as usize,
            column,
            (first_line + display_offset as i32).max(0) as usize,
        )
    }

    pub(super) fn terminal_font_metrics(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TerminalFontMetrics {
        if let Some(metrics) = &self.font_metrics {
            if metrics.matches(&self.font_family, &self.font_fallbacks, self.font_size) {
                return metrics.clone();
            }
        }

        let metrics = self.refresh_terminal_font_metrics(window, cx);
        self.font_metrics = Some(metrics.clone());
        metrics
    }

    pub(super) fn refresh_terminal_font_metrics(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TerminalFontMetrics {
        let installed_font_names = cx.text_system().all_font_names();
        let effective_family: SharedString = resolve_installed_grid_monospace_font_family(
            self.font_family.as_ref(),
            &installed_font_names,
        )
        .into();
        let font = self.terminal_font(effective_family.clone());
        let font_id = window.text_system().resolve_font(&font);
        let measured_widths = "mMW@#0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
            .chars()
            .filter_map(|ch| {
                window
                    .text_system()
                    .advance(font_id, self.font_size, ch)
                    .map(|size| size.width)
                    .ok()
            });
        TerminalFontMetrics {
            requested_family: self.font_family.clone(),
            fallbacks: self.font_fallbacks.clone(),
            font_size: self.font_size,
            effective_family,
            cell_width: terminal_cell_width_from_advances(self.font_size, measured_widths),
        }
    }

    pub(super) fn terminal_font(&self, family: SharedString) -> Font {
        let fallbacks = if self.font_fallbacks.is_empty() {
            None
        } else {
            Some(FontFallbacks::from_fonts(
                self.font_fallbacks
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>(),
            ))
        };
        let features = FontFeatures(Arc::new(vec![("calt".to_string(), 0)]));
        Font {
            family,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
            features,
            fallbacks,
        }
    }
}
