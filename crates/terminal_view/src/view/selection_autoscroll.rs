use super::*;

impl TerminalView {
    pub(super) fn update_selection_autoscroll(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if terminal_selection_autoscroll_delta_rows(
            position.y,
            self.terminal_bounds.origin.y,
            self.terminal_bounds.bottom(),
            self.line_height,
        ) == 0
        {
            self.clear_selection_autoscroll();
            return;
        }

        if self.selection_autoscroll_position.is_none() {
            self.selection_autoscroll_display_offset =
                Some(self.terminal_frame_snapshot.display_offset);
        }
        self.selection_autoscroll_position = Some(position);
        self.schedule_selection_autoscroll(cx);
    }

    pub(super) fn clear_selection_autoscroll(&mut self) {
        self.selection_autoscroll_position = None;
        self.selection_autoscroll_display_offset = None;
        self.selection_autoscroll_task.take();
    }

    fn schedule_selection_autoscroll(&mut self, cx: &mut Context<Self>) {
        if self.selection_autoscroll_task.is_some() {
            return;
        }

        self.selection_autoscroll_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.selection_autoscroll_task = None;
                this.run_selection_autoscroll_tick(cx);
            });
        }));
    }

    fn run_selection_autoscroll_tick(&mut self, cx: &mut Context<Self>) {
        let Some(position) = self.selection_autoscroll_position else {
            return;
        };
        if !self.mouse_state.selecting {
            self.clear_selection_autoscroll();
            return;
        }

        let delta = terminal_selection_autoscroll_delta_rows(
            position.y,
            self.terminal_bounds.origin.y,
            self.terminal_bounds.bottom(),
            self.line_height,
        );
        let current = self
            .selection_autoscroll_display_offset
            .unwrap_or(self.terminal_frame_snapshot.display_offset);
        let target = (current as i64 + delta as i64)
            .clamp(0, self.terminal_frame_snapshot.history_size as i64)
            as usize;
        if delta == 0 || target == current {
            self.clear_selection_autoscroll();
            return;
        }
        if !self.scrollbar_handle.try_set_display_offset(target) {
            self.schedule_terminal_render_retry(cx);
            self.schedule_selection_autoscroll(cx);
            return;
        }

        self.selection_autoscroll_display_offset = Some(target);
        let point = self.pixel_to_point(position, self.terminal_bounds, cx);
        let side = self.pixel_to_side(position, self.terminal_bounds);
        self.apply_or_queue_terminal_selection_action(
            PendingTerminalSelectionAction::Update { point, side },
            cx,
        );
        cx.notify();
        self.schedule_selection_autoscroll(cx);
    }
}
