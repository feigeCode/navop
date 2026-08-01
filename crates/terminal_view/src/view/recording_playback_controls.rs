use super::recording_playback_footer::{
    playback_progress, playback_seek_target, try_for_each_playback_advance_step,
};
use super::*;
use std::time::{Duration, Instant};
use terminal::recording::{RecordingPlaybackError, RecordingPlaybackState};

const PLAYBACK_TICK_INTERVAL: Duration = Duration::from_millis(33);
const PLAYBACK_SLIDER_SYNC_EPSILON: f32 = 0.000_5;

impl TerminalView {
    pub(super) fn handle_recording_playback_slider_event(
        &mut self,
        _slider: &Entity<SliderState>,
        event: &SliderEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SliderEvent::Change(SliderValue::Single(_)) => {
                self.recording_playback_slider_dragging = true;
                cx.notify();
            }
            SliderEvent::Release(SliderValue::Single(progress)) => {
                self.recording_playback_slider_dragging = false;
                self.request_recording_playback_seek(*progress, cx);
            }
            SliderEvent::Change(SliderValue::Range(_, _))
            | SliderEvent::Release(SliderValue::Range(_, _)) => {
                self.recording_playback_slider_dragging = false;
            }
        }
    }

    pub(super) fn request_recording_playback_seek(
        &mut self,
        progress: f32,
        cx: &mut Context<Self>,
    ) {
        let duration = self
            .terminal
            .read(cx)
            .recording_playback_duration()
            .unwrap_or_default();
        let target = playback_seek_target(progress, duration);
        let result = self
            .terminal
            .update(cx, |terminal, _| terminal.seek_recording_playback(target));
        self.apply_recording_playback_control_result(result, cx);
    }

    pub(super) fn request_recording_playback_pause(&mut self, cx: &mut Context<Self>) {
        let result = self.terminal.update(cx, |terminal, _| {
            terminal.pause_recording_playback().map(|_| ())
        });
        self.apply_recording_playback_control_result(result, cx);
    }

    pub(super) fn request_recording_playback_resume(&mut self, cx: &mut Context<Self>) {
        let result = self.terminal.update(cx, |terminal, _| {
            terminal.resume_recording_playback().map(|_| ())
        });
        self.apply_recording_playback_control_result(result, cx);
    }

    pub(super) fn request_recording_playback_replay(&mut self, cx: &mut Context<Self>) {
        let result = self.terminal.update(cx, |terminal, _| {
            terminal.seek_recording_playback(Duration::ZERO)?;
            terminal.resume_recording_playback()?;
            Ok(())
        });
        self.apply_recording_playback_control_result(result, cx);
    }

    pub(super) fn request_recording_playback_speed(&mut self, speed: f64, cx: &mut Context<Self>) {
        let result = self.terminal.update(cx, |terminal, _| {
            terminal.set_recording_playback_speed(speed).map(|_| ())
        });
        self.apply_recording_playback_control_result(result, cx);
    }

    fn apply_recording_playback_control_result(
        &mut self,
        result: Result<(), RecordingPlaybackError>,
        cx: &mut Context<Self>,
    ) {
        self.recording_playback_ticker.take();
        match result {
            Ok(()) => self.recording_playback_control_error = None,
            Err(error) => self.recording_playback_control_error = Some(error.to_string()),
        }
        self.sync_recording_playback_ticker(cx);
        cx.notify();
    }

    fn recording_playback_is_playing(&self, cx: &App) -> bool {
        self.terminal.read(cx).recording_playback_state() == Some(RecordingPlaybackState::Playing)
    }

    pub(super) fn sync_recording_playback_ticker(&mut self, cx: &mut Context<Self>) {
        if self
            .recording_playback_ticker
            .as_ref()
            .is_some_and(Task::is_ready)
        {
            self.recording_playback_ticker.take();
        }
        if self.recording_playback_control_error.is_some()
            || !self.recording_playback_is_playing(cx)
        {
            self.recording_playback_ticker.take();
            return;
        }
        if self.recording_playback_ticker.is_some() {
            return;
        }

        self.recording_playback_ticker = Some(cx.spawn(async move |this, cx| {
            let mut last_tick = Instant::now();
            loop {
                cx.background_executor().timer(PLAYBACK_TICK_INTERVAL).await;
                let now = Instant::now();
                let elapsed = now.saturating_duration_since(last_tick);
                last_tick = now;
                let should_continue = this
                    .update(cx, |this, cx| {
                        this.advance_recording_playback_clock(elapsed, cx)
                    })
                    .unwrap_or(false);
                if !should_continue {
                    break;
                }
            }
        }));
    }

    fn advance_recording_playback_clock(
        &mut self,
        elapsed: Duration,
        cx: &mut Context<Self>,
    ) -> bool {
        let result = self.terminal.update(cx, |terminal, _| {
            try_for_each_playback_advance_step(elapsed, |step| {
                terminal.advance_recording_playback(step)
            })
        });
        match result {
            Ok(()) => {
                let should_continue = self.recording_playback_is_playing(cx);
                if !should_continue {
                    self.recording_playback_ticker = None;
                }
                cx.notify();
                should_continue
            }
            Err(error) => {
                self.recording_playback_control_error = Some(error.to_string());
                self.recording_playback_ticker = None;
                cx.notify();
                false
            }
        }
    }

    pub(super) fn sync_recording_playback_slider(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.recording_playback_slider_dragging {
            return;
        }
        let progress = {
            let terminal = self.terminal.read(cx);
            if !terminal.is_recording_playback() {
                return;
            }
            playback_progress(
                terminal.recording_playback_elapsed().unwrap_or_default(),
                terminal.recording_playback_duration().unwrap_or_default(),
            )
        };
        let current = self.recording_playback_slider.read(cx).value().start();
        if (current - progress).abs() <= PLAYBACK_SLIDER_SYNC_EPSILON {
            return;
        }
        self.recording_playback_slider
            .update(cx, |slider, cx| slider.set_value(progress, window, cx));
    }
}
