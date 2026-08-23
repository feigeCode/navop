use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use remote_desktop::{RemoteDesktopFrameRect, RgbaFramebuffer};

use crate::pixels::rgba_to_bgra;

use super::frames::apply_bgra_rects_to_framebuffer;
use super::surface::RemoteDesktopSurface;

const MAX_MERGED_DELTA_RECTS: usize = 4096;
const MAX_MERGED_DELTA_BYTES: usize = 8 * 1024 * 1024;
const MAX_PENDING_DELTA_RECTS: usize = 8192;
const MAX_PENDING_DELTA_BYTES: usize = 16 * 1024 * 1024;
const FRAME_PRESENTATION_INTERVAL: Duration = Duration::from_millis(16);

pub(super) enum PresentationFrame {
    Rgba {
        width: u16,
        height: u16,
        rgba: Vec<u8>,
    },
    Bgra {
        width: u16,
        height: u16,
        bgra: Vec<u8>,
    },
    BgraRects {
        width: u16,
        height: u16,
        rects: Vec<RemoteDesktopFrameRect>,
        bgra: Vec<u8>,
    },
}

impl PresentationFrame {
    fn dimensions(&self) -> (u16, u16) {
        match self {
            Self::Rgba { width, height, .. }
            | Self::Bgra { width, height, .. }
            | Self::BgraRects { width, height, .. } => (*width, *height),
        }
    }

    fn is_delta(&self) -> bool {
        matches!(self, Self::BgraRects { .. })
    }

    fn delta_size(&self) -> Option<(usize, usize)> {
        match self {
            Self::BgraRects { rects, bgra, .. } => Some((rects.len(), bgra.len())),
            _ => None,
        }
    }
}

pub(super) enum PresentationCommand {
    Reset {
        generation: u64,
    },
    Connected {
        generation: u64,
    },
    Frame {
        generation: u64,
        ticket: u64,
        frame: PresentationFrame,
    },
}

impl PresentationCommand {
    fn generation(&self) -> u64 {
        match self {
            Self::Reset { generation }
            | Self::Connected { generation }
            | Self::Frame { generation, .. } => *generation,
        }
    }
}

#[derive(Clone, Copy)]
struct PresentationQueueLimits {
    max_delta_rects: usize,
    max_delta_bytes: usize,
}

impl Default for PresentationQueueLimits {
    fn default() -> Self {
        Self {
            max_delta_rects: MAX_PENDING_DELTA_RECTS,
            max_delta_bytes: MAX_PENDING_DELTA_BYTES,
        }
    }
}

pub(super) enum PresentationQueuePushResult {
    Queued,
    DeltaDropped {
        generation: u64,
        width: u16,
        height: u16,
        recovery_required: bool,
    },
}

#[derive(Default)]
pub(super) struct PresentationQueue {
    pending: VecDeque<PresentationCommand>,
    pending_delta_rects: usize,
    pending_delta_bytes: usize,
    awaiting_base_generation: Option<u64>,
    limits: PresentationQueueLimits,
}

impl PresentationQueue {
    pub(super) fn push(&mut self, command: PresentationCommand) -> PresentationQueuePushResult {
        match command {
            PresentationCommand::Reset { generation } => {
                self.clear();
                self.pending
                    .push_back(PresentationCommand::Reset { generation });
                PresentationQueuePushResult::Queued
            }
            PresentationCommand::Connected { generation } => {
                self.pending.retain(|command| {
                    matches!(
                        command,
                        PresentationCommand::Reset {
                            generation: barrier_generation
                        } if *barrier_generation == generation
                    )
                });
                self.reset_delta_accounting();
                self.awaiting_base_generation = None;
                self.pending
                    .push_back(PresentationCommand::Connected { generation });
                PresentationQueuePushResult::Queued
            }
            PresentationCommand::Frame {
                generation,
                ticket,
                mut frame,
            } => {
                self.retain_generation_commands(generation);
                if frame.is_delta() {
                    let (width, height) = frame.dimensions();
                    if self.awaiting_base_generation == Some(generation) {
                        return PresentationQueuePushResult::DeltaDropped {
                            generation,
                            width,
                            height,
                            recovery_required: false,
                        };
                    }
                    let (delta_rects, delta_bytes) = frame
                        .delta_size()
                        .expect("delta frame must report its payload size");
                    if self.pending_delta_rects.saturating_add(delta_rects)
                        > self.limits.max_delta_rects
                        || self.pending_delta_bytes.saturating_add(delta_bytes)
                            > self.limits.max_delta_bytes
                    {
                        let recovery_required = self.require_base(generation);
                        return PresentationQueuePushResult::DeltaDropped {
                            generation,
                            width,
                            height,
                            recovery_required,
                        };
                    }
                    if let Some(PresentationCommand::Frame {
                        generation: previous_generation,
                        ticket: previous_ticket,
                        frame:
                            PresentationFrame::BgraRects {
                                width: previous_width,
                                height: previous_height,
                                rects: previous_rects,
                                bgra: previous_bgra,
                            },
                    }) = self.pending.back_mut()
                        && *previous_generation == generation
                        && (*previous_width, *previous_height) == frame.dimensions()
                    {
                        let PresentationFrame::BgraRects { rects, bgra, .. } = &mut frame else {
                            unreachable!("delta frame checked above");
                        };
                        if previous_rects.len().saturating_add(rects.len())
                            <= MAX_MERGED_DELTA_RECTS
                            && previous_bgra.len().saturating_add(bgra.len())
                                <= MAX_MERGED_DELTA_BYTES
                        {
                            previous_rects.append(rects);
                            previous_bgra.append(bgra);
                            *previous_ticket = ticket;
                            self.pending_delta_rects =
                                self.pending_delta_rects.saturating_add(delta_rects);
                            self.pending_delta_bytes =
                                self.pending_delta_bytes.saturating_add(delta_bytes);
                            return PresentationQueuePushResult::Queued;
                        }
                    }
                    self.pending_delta_rects = self.pending_delta_rects.saturating_add(delta_rects);
                    self.pending_delta_bytes = self.pending_delta_bytes.saturating_add(delta_bytes);
                } else {
                    self.pending
                        .retain(|command| !matches!(command, PresentationCommand::Frame { .. }));
                    self.reset_delta_accounting();
                    self.awaiting_base_generation = None;
                }

                self.pending.push_back(PresentationCommand::Frame {
                    generation,
                    ticket,
                    frame,
                });
                PresentationQueuePushResult::Queued
            }
        }
    }

    pub(super) fn pop_front(&mut self) -> Option<PresentationCommand> {
        let command = self.pending.pop_front()?;
        if let PresentationCommand::Frame { frame, .. } = &command
            && let Some((rects, bytes)) = frame.delta_size()
        {
            self.pending_delta_rects = self.pending_delta_rects.saturating_sub(rects);
            self.pending_delta_bytes = self.pending_delta_bytes.saturating_sub(bytes);
        }
        Some(command)
    }

    pub(super) fn clear(&mut self) {
        self.pending.clear();
        self.reset_delta_accounting();
        self.awaiting_base_generation = None;
    }

    /// Discard every pending frame for this generation and reject later deltas
    /// until a complete frame re-establishes the predecessor chain.
    ///
    /// Returns `true` only for the first transition into this recovery epoch.
    pub(super) fn require_base(&mut self, generation: u64) -> bool {
        self.retain_generation_commands(generation);
        self.pending
            .retain(|command| !matches!(command, PresentationCommand::Frame { .. }));
        self.reset_delta_accounting();
        let recovery_required = self.awaiting_base_generation != Some(generation);
        self.awaiting_base_generation = Some(generation);
        recovery_required
    }

    pub(super) fn front_is_frame(&self) -> bool {
        matches!(
            self.pending.front(),
            Some(PresentationCommand::Frame { .. })
        )
    }

    pub(super) fn has_pending_frame(&self, generation: u64) -> bool {
        self.pending.iter().any(|command| {
            matches!(
                command,
                PresentationCommand::Frame {
                    generation: frame_generation,
                    ..
                } if *frame_generation == generation
            )
        })
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.pending.len()
    }

    #[cfg(test)]
    fn with_delta_limits(max_delta_rects: usize, max_delta_bytes: usize) -> Self {
        Self {
            limits: PresentationQueueLimits {
                max_delta_rects,
                max_delta_bytes,
            },
            ..Self::default()
        }
    }

    fn retain_generation_commands(&mut self, generation: u64) {
        self.pending
            .retain(|command| command.generation() == generation);
        self.recalculate_delta_accounting();
    }

    fn recalculate_delta_accounting(&mut self) {
        let (rects, bytes) = self
            .pending
            .iter()
            .filter_map(|command| match command {
                PresentationCommand::Frame { frame, .. } => frame.delta_size(),
                _ => None,
            })
            .fold(
                (0usize, 0usize),
                |(total_rects, total_bytes), (rects, bytes)| {
                    (
                        total_rects.saturating_add(rects),
                        total_bytes.saturating_add(bytes),
                    )
                },
            );
        self.pending_delta_rects = rects;
        self.pending_delta_bytes = bytes;
    }

    fn reset_delta_accounting(&mut self) {
        self.pending_delta_rects = 0;
        self.pending_delta_bytes = 0;
    }
}

#[derive(Default)]
pub(super) struct PresentationPacer {
    last_presented_at: Option<Instant>,
    timer_generation: u64,
}

impl PresentationPacer {
    pub(super) fn reset(&mut self) {
        self.last_presented_at = None;
        self.invalidate_timer();
    }

    pub(super) fn mark_presented(&mut self, presented_at: Instant) {
        self.last_presented_at = Some(presented_at);
    }

    pub(super) fn begin_timer(&mut self) -> u64 {
        self.invalidate_timer();
        self.timer_generation
    }

    pub(super) fn invalidate_timer(&mut self) {
        self.timer_generation = self.timer_generation.wrapping_add(1);
    }

    pub(super) fn is_current_timer(&self, timer_generation: u64) -> bool {
        self.timer_generation == timer_generation
    }

    pub(super) fn next_frame_delay(&self, now: Instant) -> Duration {
        let Some(last_presented_at) = self.last_presented_at else {
            return Duration::ZERO;
        };
        FRAME_PRESENTATION_INTERVAL.saturating_sub(now.saturating_duration_since(last_presented_at))
    }
}

pub(super) struct PresentationState {
    connected: bool,
    generation: u64,
    framebuffer: Option<RgbaFramebuffer>,
    surface: Option<Arc<RemoteDesktopSurface>>,
}

impl Default for PresentationState {
    fn default() -> Self {
        Self {
            connected: false,
            generation: 0,
            framebuffer: None,
            surface: None,
        }
    }
}

pub(super) struct PreparedFrame {
    pub(super) generation: u64,
    pub(super) ticket: u64,
    pub(super) width: u16,
    pub(super) height: u16,
    pub(super) surface: Option<Arc<RemoteDesktopSurface>>,
    pub(super) kind: PreparedFrameKind,
}

pub(super) enum PreparedFrameKind {
    Base { encoding: &'static str },
    Delta,
}

pub(super) enum PresentationResult {
    Acknowledged,
    Prepared(PreparedFrame),
    RejectedFrame {
        generation: u64,
        width: u16,
        height: u16,
        reason: String,
    },
    RejectedDelta {
        generation: u64,
        width: u16,
        height: u16,
        reason: String,
    },
    Skipped,
}

pub(super) struct ProcessedPresentation {
    pub(super) state: PresentationState,
    pub(super) result: PresentationResult,
}

impl PresentationState {
    pub(super) fn process(
        mut self,
        command: PresentationCommand,
        latest_frame_ticket: &AtomicU64,
    ) -> ProcessedPresentation {
        let result = match command {
            PresentationCommand::Reset { generation } => {
                self.connected = false;
                self.generation = generation;
                self.framebuffer = None;
                self.surface = None;
                PresentationResult::Acknowledged
            }
            PresentationCommand::Connected { generation } => {
                if self.generation == generation {
                    self.connected = true;
                    self.framebuffer = None;
                    self.surface = None;
                    PresentationResult::Acknowledged
                } else {
                    PresentationResult::Skipped
                }
            }
            PresentationCommand::Frame {
                generation,
                ticket,
                frame,
            } => self.process_frame(generation, ticket, frame, latest_frame_ticket),
        };

        ProcessedPresentation {
            state: self,
            result,
        }
    }

    fn process_frame(
        &mut self,
        generation: u64,
        ticket: u64,
        frame: PresentationFrame,
        latest_frame_ticket: &AtomicU64,
    ) -> PresentationResult {
        if generation != self.generation || !self.connected {
            return PresentationResult::Skipped;
        }

        match frame {
            PresentationFrame::Rgba {
                width,
                height,
                rgba,
            } => self.process_full_frame(
                generation,
                ticket,
                width,
                height,
                rgba_to_bgra(rgba),
                "rgba",
                latest_frame_ticket,
            ),
            PresentationFrame::Bgra {
                width,
                height,
                bgra,
            } => self.process_full_frame(
                generation,
                ticket,
                width,
                height,
                bgra,
                "bgra",
                latest_frame_ticket,
            ),
            PresentationFrame::BgraRects {
                width,
                height,
                rects,
                bgra,
            } => self.process_delta_frame(
                generation,
                ticket,
                width,
                height,
                rects,
                bgra,
                latest_frame_ticket,
            ),
        }
    }

    fn process_delta_frame(
        &mut self,
        generation: u64,
        ticket: u64,
        width: u16,
        height: u16,
        rects: Vec<RemoteDesktopFrameRect>,
        bgra: Vec<u8>,
        latest_frame_ticket: &AtomicU64,
    ) -> PresentationResult {
        {
            let Some(framebuffer) = self.framebuffer.as_mut() else {
                return rejected_delta(generation, width, height, "missing base framebuffer");
            };
            if let Err(error) =
                apply_bgra_rects_to_framebuffer(framebuffer, width, height, &rects, &bgra)
            {
                return rejected_delta(generation, width, height, error);
            }
        }

        let framebuffer = self
            .framebuffer
            .as_ref()
            .expect("framebuffer must remain available after applying a delta");
        let surface = if let Some(surface) = self.surface.as_ref() {
            surface.with_dirty_rects(framebuffer, &rects)
        } else {
            RemoteDesktopSurface::from_framebuffer(framebuffer)
        };
        let surface = match surface {
            Ok(surface) => Arc::new(surface),
            Err(error) => {
                self.framebuffer = None;
                self.surface = None;
                return rejected_delta(generation, width, height, error);
            }
        };

        self.surface = Some(surface.clone());
        let visible_surface = is_latest_frame(ticket, latest_frame_ticket).then_some(surface);
        prepared_frame(
            generation,
            ticket,
            width,
            height,
            visible_surface,
            PreparedFrameKind::Delta,
        )
    }

    fn process_full_frame(
        &mut self,
        generation: u64,
        ticket: u64,
        width: u16,
        height: u16,
        bgra: Vec<u8>,
        encoding: &'static str,
        latest_frame_ticket: &AtomicU64,
    ) -> PresentationResult {
        let framebuffer = match RgbaFramebuffer::from_bgra(width, height, bgra) {
            Ok(framebuffer) => framebuffer,
            Err(error) => {
                return PresentationResult::RejectedFrame {
                    generation,
                    width,
                    height,
                    reason: error.to_string(),
                };
            }
        };
        self.framebuffer = Some(framebuffer);

        let framebuffer = self
            .framebuffer
            .as_ref()
            .expect("framebuffer assigned above");
        let surface = match self.surface.as_ref() {
            Some(surface) if surface.width() == width && surface.height() == height => {
                surface.with_full_framebuffer(framebuffer)
            }
            _ => RemoteDesktopSurface::from_framebuffer(framebuffer),
        };
        let surface = match surface {
            Ok(surface) => Arc::new(surface),
            Err(error) => {
                self.framebuffer = None;
                self.surface = None;
                return PresentationResult::RejectedFrame {
                    generation,
                    width,
                    height,
                    reason: error.to_string(),
                };
            }
        };

        self.surface = Some(surface.clone());
        let visible_surface = is_latest_frame(ticket, latest_frame_ticket).then_some(surface);
        prepared_frame(
            generation,
            ticket,
            width,
            height,
            visible_surface,
            PreparedFrameKind::Base { encoding },
        )
    }
}

fn is_latest_frame(ticket: u64, latest_frame_ticket: &AtomicU64) -> bool {
    latest_frame_ticket.load(Ordering::Acquire) == ticket
}

fn prepared_frame(
    generation: u64,
    ticket: u64,
    width: u16,
    height: u16,
    surface: Option<Arc<RemoteDesktopSurface>>,
    kind: PreparedFrameKind,
) -> PresentationResult {
    PresentationResult::Prepared(PreparedFrame {
        generation,
        ticket,
        width,
        height,
        surface,
        kind,
    })
}

fn rejected_delta(
    generation: u64,
    width: u16,
    height: u16,
    reason: impl ToString,
) -> PresentationResult {
    PresentationResult::RejectedDelta {
        generation,
        width,
        height,
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        FRAME_PRESENTATION_INTERVAL, MAX_MERGED_DELTA_BYTES, MAX_MERGED_DELTA_RECTS, PreparedFrame,
        PreparedFrameKind, PresentationCommand, PresentationFrame, PresentationPacer,
        PresentationQueue, PresentationQueuePushResult, PresentationResult, PresentationState,
    };
    use remote_desktop::RemoteDesktopFrameRect;
    use std::time::{Duration, Instant};

    fn delta(generation: u64, value: u8) -> PresentationCommand {
        delta_with_ticket(generation, u64::from(value), value)
    }

    fn delta_with_ticket(generation: u64, ticket: u64, value: u8) -> PresentationCommand {
        PresentationCommand::Frame {
            generation,
            ticket,
            frame: PresentationFrame::BgraRects {
                width: 2,
                height: 1,
                rects: vec![RemoteDesktopFrameRect {
                    x: value as u16 % 2,
                    y: 0,
                    width: 1,
                    height: 1,
                    byte_len: 4,
                }],
                bgra: vec![value, value, value, 255],
            },
        }
    }

    #[test]
    fn full_frame_replaces_older_pending_frames_but_keeps_session_barriers() {
        let mut queue = PresentationQueue::default();
        queue.push(PresentationCommand::Reset { generation: 7 });
        queue.push(PresentationCommand::Connected { generation: 7 });
        queue.push(delta(7, 1));
        queue.push(PresentationCommand::Frame {
            generation: 7,
            ticket: 3,
            frame: PresentationFrame::Bgra {
                width: 2,
                height: 1,
                bgra: vec![0; 8],
            },
        });

        assert_eq!(3, queue.len());
        assert!(matches!(
            queue.pop_front(),
            Some(PresentationCommand::Reset { generation: 7 })
        ));
        assert!(matches!(
            queue.pop_front(),
            Some(PresentationCommand::Connected { generation: 7 })
        ));
        assert!(matches!(
            queue.pop_front(),
            Some(PresentationCommand::Frame {
                frame: PresentationFrame::Bgra { .. },
                ..
            })
        ));
        assert!(queue.pop_front().is_none());
    }

    #[test]
    fn consecutive_deltas_merge_without_dropping_predecessor_payload() {
        let mut queue = PresentationQueue::default();
        queue.push(PresentationCommand::Reset { generation: 7 });
        queue.push(PresentationCommand::Connected { generation: 7 });
        queue.push(delta(7, 1));
        queue.push(delta(7, 2));

        assert_eq!(3, queue.len());
        let _ = queue.pop_front();
        let _ = queue.pop_front();
        let Some(PresentationCommand::Frame {
            ticket,
            frame: PresentationFrame::BgraRects { rects, bgra, .. },
            ..
        }) = queue.pop_front()
        else {
            panic!("expected merged delta");
        };
        assert_eq!(2, rects.len());
        assert_eq!(8, bgra.len());
        assert_eq!(1, bgra[0]);
        assert_eq!(2, bgra[4]);
        assert_eq!(2, ticket);
    }

    #[test]
    fn has_pending_frame_ignores_session_barriers_and_tracks_frames() {
        let mut queue = PresentationQueue::default();

        assert!(!queue.has_pending_frame(7));
        queue.push(PresentationCommand::Reset { generation: 7 });
        queue.push(PresentationCommand::Connected { generation: 7 });
        assert!(!queue.has_pending_frame(7));

        queue.push(delta(7, 1));
        assert!(queue.has_pending_frame(7));
        assert!(!queue.has_pending_frame(8));

        let _ = queue.pop_front();
        let _ = queue.pop_front();
        assert!(queue.has_pending_frame(7));
        let _ = queue.pop_front();
        assert!(!queue.has_pending_frame(7));
    }

    #[test]
    fn presentation_pacer_allows_the_first_frame_without_delay() {
        let pacer = PresentationPacer::default();

        assert_eq!(Duration::ZERO, pacer.next_frame_delay(Instant::now()));
    }

    #[test]
    fn presentation_pacer_limits_visible_frames_to_the_target_interval() {
        let presented_at = Instant::now();
        let mut pacer = PresentationPacer::default();
        pacer.mark_presented(presented_at);

        assert_eq!(
            FRAME_PRESENTATION_INTERVAL,
            pacer.next_frame_delay(presented_at)
        );
        assert_eq!(
            Duration::from_millis(7),
            pacer.next_frame_delay(presented_at + Duration::from_millis(9))
        );
        assert_eq!(
            Duration::ZERO,
            pacer.next_frame_delay(presented_at + FRAME_PRESENTATION_INTERVAL)
        );
    }

    #[test]
    fn resetting_the_pacer_makes_the_next_session_frame_immediate() {
        let now = Instant::now();
        let mut pacer = PresentationPacer::default();
        pacer.mark_presented(now);
        pacer.reset();

        assert_eq!(Duration::ZERO, pacer.next_frame_delay(now));
    }

    #[test]
    fn invalidating_the_pacer_rejects_an_old_timer_callback() {
        let mut pacer = PresentationPacer::default();
        let old_timer_generation = pacer.begin_timer();

        assert!(pacer.is_current_timer(old_timer_generation));

        pacer.invalidate_timer();

        assert!(!pacer.is_current_timer(old_timer_generation));
    }

    #[test]
    fn starting_a_new_timer_rejects_the_previous_timer_callback() {
        let mut pacer = PresentationPacer::default();
        let previous_timer_generation = pacer.begin_timer();
        let current_timer_generation = pacer.begin_timer();

        assert!(!pacer.is_current_timer(previous_timer_generation));
        assert!(pacer.is_current_timer(current_timer_generation));
    }

    #[test]
    fn resetting_the_pacer_rejects_a_previous_session_timer_callback() {
        let mut pacer = PresentationPacer::default();
        let previous_session_timer_generation = pacer.begin_timer();

        pacer.reset();

        assert!(!pacer.is_current_timer(previous_session_timer_generation));
    }

    #[test]
    fn has_pending_frame_survives_delta_coalescing_and_full_frame_replacement() {
        let mut queue = PresentationQueue::default();
        queue.push(PresentationCommand::Reset { generation: 7 });
        queue.push(PresentationCommand::Connected { generation: 7 });
        queue.push(delta(7, 1));
        queue.push(delta(7, 2));

        assert!(queue.has_pending_frame(7));

        queue.push(PresentationCommand::Frame {
            generation: 7,
            ticket: 3,
            frame: PresentationFrame::Bgra {
                width: 2,
                height: 1,
                bgra: vec![0; 8],
            },
        });
        assert!(queue.has_pending_frame(7));

        queue.clear();
        assert!(!queue.has_pending_frame(7));
    }

    #[test]
    fn merged_delta_stops_before_rect_limit_without_dropping_frames() {
        let mut queue = PresentationQueue::default();
        queue.push(PresentationCommand::Reset { generation: 7 });
        queue.push(PresentationCommand::Connected { generation: 7 });

        for value in 0..MAX_MERGED_DELTA_RECTS {
            queue.push(delta(7, (value % 2) as u8));
        }
        queue.push(delta(7, 9));

        assert_eq!(4, queue.len());
        let _ = queue.pop_front();
        let _ = queue.pop_front();

        let Some(PresentationCommand::Frame {
            frame: PresentationFrame::BgraRects { rects, bgra, .. },
            ..
        }) = queue.pop_front()
        else {
            panic!("expected the first bounded delta batch");
        };
        assert_eq!(MAX_MERGED_DELTA_RECTS, rects.len());
        assert_eq!(MAX_MERGED_DELTA_RECTS * 4, bgra.len());

        let Some(PresentationCommand::Frame {
            frame: PresentationFrame::BgraRects { rects, bgra, .. },
            ..
        }) = queue.pop_front()
        else {
            panic!("expected the delta after the limit");
        };
        assert_eq!(1, rects.len());
        assert_eq!(vec![9, 9, 9, 255], bgra);
    }

    #[test]
    fn merged_delta_stops_before_payload_limit_without_truncating_payload() {
        let mut queue = PresentationQueue::default();
        queue.push(PresentationCommand::Reset { generation: 7 });
        queue.push(PresentationCommand::Connected { generation: 7 });
        queue.push(PresentationCommand::Frame {
            generation: 7,
            ticket: 1,
            frame: PresentationFrame::BgraRects {
                width: 2,
                height: 1,
                rects: vec![RemoteDesktopFrameRect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                    byte_len: MAX_MERGED_DELTA_BYTES,
                }],
                bgra: vec![1; MAX_MERGED_DELTA_BYTES],
            },
        });
        queue.push(delta(7, 2));

        assert_eq!(4, queue.len());
        let _ = queue.pop_front();
        let _ = queue.pop_front();
        let Some(PresentationCommand::Frame {
            frame: PresentationFrame::BgraRects { rects, bgra, .. },
            ..
        }) = queue.pop_front()
        else {
            panic!("expected the first bounded delta batch");
        };
        assert_eq!(MAX_MERGED_DELTA_BYTES, bgra.len());
        assert_eq!(1, rects.len());
        assert_eq!(1, bgra[0]);
        assert_eq!(1, bgra[MAX_MERGED_DELTA_BYTES - 1]);

        let Some(PresentationCommand::Frame {
            frame: PresentationFrame::BgraRects { bgra, .. },
            ..
        }) = queue.pop_front()
        else {
            panic!("expected the delta after the payload limit");
        };
        assert_eq!(vec![2, 2, 2, 255], bgra);
    }

    #[test]
    fn total_pending_delta_rect_budget_starts_recovery_and_preserves_barriers() {
        let mut queue = PresentationQueue::with_delta_limits(2, usize::MAX);
        queue.push(PresentationCommand::Reset { generation: 7 });
        queue.push(PresentationCommand::Connected { generation: 7 });
        assert!(matches!(
            queue.push(delta(7, 1)),
            PresentationQueuePushResult::Queued
        ));
        assert!(matches!(
            queue.push(delta(7, 2)),
            PresentationQueuePushResult::Queued
        ));

        assert!(matches!(
            queue.push(delta(7, 3)),
            PresentationQueuePushResult::DeltaDropped {
                generation: 7,
                recovery_required: true,
                ..
            }
        ));
        assert_eq!(2, queue.len());
        assert!(matches!(
            queue.pop_front(),
            Some(PresentationCommand::Reset { generation: 7 })
        ));
        assert!(matches!(
            queue.pop_front(),
            Some(PresentationCommand::Connected { generation: 7 })
        ));
    }

    #[test]
    fn total_pending_delta_byte_budget_drops_following_deltas_until_a_base() {
        let mut queue = PresentationQueue::with_delta_limits(usize::MAX, 8);
        queue.push(PresentationCommand::Reset { generation: 7 });
        queue.push(PresentationCommand::Connected { generation: 7 });
        queue.push(delta(7, 1));
        queue.push(delta(7, 2));

        assert!(matches!(
            queue.push(delta(7, 3)),
            PresentationQueuePushResult::DeltaDropped {
                recovery_required: true,
                ..
            }
        ));
        assert!(matches!(
            queue.push(delta(7, 4)),
            PresentationQueuePushResult::DeltaDropped {
                recovery_required: false,
                ..
            }
        ));
        assert_eq!(2, queue.len());

        assert!(matches!(
            queue.push(PresentationCommand::Frame {
                generation: 7,
                ticket: 5,
                frame: PresentationFrame::Bgra {
                    width: 2,
                    height: 1,
                    bgra: vec![0; 8],
                },
            }),
            PresentationQueuePushResult::Queued
        ));
        assert!(matches!(
            queue.push(delta(7, 6)),
            PresentationQueuePushResult::Queued
        ));
        assert_eq!(4, queue.len());
    }

    #[test]
    fn popping_and_clearing_frames_release_the_pending_delta_budget() {
        let mut queue = PresentationQueue::with_delta_limits(1, 4);
        queue.push(delta(7, 1));
        assert!(matches!(
            queue.pop_front(),
            Some(PresentationCommand::Frame {
                frame: PresentationFrame::BgraRects { .. },
                ..
            })
        ));
        assert!(matches!(
            queue.push(delta(7, 2)),
            PresentationQueuePushResult::Queued
        ));

        queue.clear();

        assert!(matches!(
            queue.push(delta(7, 3)),
            PresentationQueuePushResult::Queued
        ));
    }

    #[test]
    fn delta_waits_after_a_pending_full_frame_instead_of_patching_the_old_base() {
        let mut queue = PresentationQueue::default();
        queue.push(PresentationCommand::Reset { generation: 7 });
        queue.push(PresentationCommand::Connected { generation: 7 });
        queue.push(PresentationCommand::Frame {
            generation: 7,
            ticket: 1,
            frame: PresentationFrame::Bgra {
                width: 2,
                height: 1,
                bgra: vec![0; 8],
            },
        });
        queue.push(delta(7, 1));

        assert_eq!(4, queue.len());
        let _ = queue.pop_front();
        let _ = queue.pop_front();
        assert!(matches!(
            queue.pop_front(),
            Some(PresentationCommand::Frame {
                frame: PresentationFrame::Bgra { .. },
                ..
            })
        ));
        assert!(matches!(
            queue.pop_front(),
            Some(PresentationCommand::Frame {
                frame: PresentationFrame::BgraRects { .. },
                ..
            })
        ));
    }

    #[test]
    fn reset_discards_pending_commands_from_the_previous_generation() {
        let mut queue = PresentationQueue::default();
        queue.push(PresentationCommand::Reset { generation: 1 });
        queue.push(PresentationCommand::Connected { generation: 1 });
        queue.push(delta(1, 1));
        queue.push(PresentationCommand::Reset { generation: 2 });
        queue.push(PresentationCommand::Connected { generation: 2 });

        assert_eq!(2, queue.len());
        assert!(matches!(
            queue.pop_front(),
            Some(PresentationCommand::Reset { generation: 2 })
        ));
        assert!(matches!(
            queue.pop_front(),
            Some(PresentationCommand::Connected { generation: 2 })
        ));
    }

    #[test]
    fn connected_barrier_is_not_coalesced_away() {
        let mut queue = PresentationQueue::default();
        queue.push(PresentationCommand::Reset { generation: 3 });
        queue.push(PresentationCommand::Connected { generation: 3 });
        queue.push(PresentationCommand::Frame {
            generation: 3,
            ticket: 1,
            frame: PresentationFrame::Bgra {
                width: 1,
                height: 1,
                bgra: vec![0; 4],
            },
        });

        assert!(matches!(
            queue.pop_front(),
            Some(PresentationCommand::Reset { generation: 3 })
        ));
        assert!(matches!(
            queue.pop_front(),
            Some(PresentationCommand::Connected { generation: 3 })
        ));
        assert!(matches!(
            queue.pop_front(),
            Some(PresentationCommand::Frame { .. })
        ));
    }

    #[test]
    fn state_patches_delta_only_after_a_matching_full_frame() {
        let latest_ticket = AtomicU64::new(1);
        let mut state = PresentationState::default();
        let reset = state.process(PresentationCommand::Reset { generation: 4 }, &latest_ticket);
        state = reset.state;
        state = state
            .process(
                PresentationCommand::Connected { generation: 4 },
                &latest_ticket,
            )
            .state;

        let rejected = state.process(delta_with_ticket(4, 1, 1), &latest_ticket);
        assert!(matches!(
            rejected.result,
            PresentationResult::RejectedDelta { .. }
        ));
        state = rejected.state;

        let base = state.process(
            PresentationCommand::Frame {
                generation: 4,
                ticket: 1,
                frame: PresentationFrame::Bgra {
                    width: 2,
                    height: 1,
                    bgra: vec![0; 8],
                },
            },
            &latest_ticket,
        );
        assert!(matches!(
            base.result,
            PresentationResult::Prepared(PreparedFrame {
                ticket: 1,
                kind: PreparedFrameKind::Base { encoding: "bgra" },
                surface: Some(_),
                ..
            })
        ));
        state = base.state;

        let delta = state.process(delta_with_ticket(4, 1, 1), &latest_ticket);
        assert!(matches!(
            delta.result,
            PresentationResult::Prepared(PreparedFrame {
                kind: PreparedFrameKind::Delta,
                surface: Some(_),
                ..
            })
        ));
    }

    #[test]
    fn state_hides_stale_delta_but_keeps_texture_and_delta_history() {
        let latest_ticket = AtomicU64::new(1);
        let mut state = PresentationState::default()
            .process(PresentationCommand::Reset { generation: 4 }, &latest_ticket)
            .state
            .process(
                PresentationCommand::Connected { generation: 4 },
                &latest_ticket,
            )
            .state;

        state = state
            .process(
                PresentationCommand::Frame {
                    generation: 4,
                    ticket: 1,
                    frame: PresentationFrame::Bgra {
                        width: 2,
                        height: 1,
                        bgra: vec![0; 8],
                    },
                },
                &latest_ticket,
            )
            .state;

        let initial_texture_id = state.surface.as_ref().unwrap().texture().id;
        latest_ticket.store(3, Ordering::Release);
        let stale = state.process(delta_with_ticket(4, 2, 1), &latest_ticket);
        assert!(matches!(
            stale.result,
            PresentationResult::Prepared(PreparedFrame {
                ticket: 2,
                kind: PreparedFrameKind::Delta,
                surface: None,
                ..
            })
        ));
        assert_eq!(
            stale.state.framebuffer.as_ref().unwrap().as_rgba(),
            &[0, 0, 0, 0, 1, 1, 1, 255]
        );
        assert_eq!(
            initial_texture_id,
            stale.state.surface.as_ref().unwrap().texture().id
        );

        let latest = stale
            .state
            .process(delta_with_ticket(4, 3, 0), &latest_ticket);
        let PresentationResult::Prepared(PreparedFrame {
            ticket: 3,
            surface: Some(surface),
            kind: PreparedFrameKind::Delta,
            ..
        }) = latest.result
        else {
            panic!("latest delta should materialize a surface");
        };
        assert_eq!(
            latest.state.framebuffer.as_ref().unwrap().as_rgba(),
            &[0, 0, 0, 255, 1, 1, 1, 255]
        );
        assert_eq!(initial_texture_id, surface.texture().id);
        let uploads = surface.pending_texture_uploads(0);
        assert_eq!(1, uploads.len());
        assert_eq!(uploads[0].bytes.as_slice(), &[0, 0, 0, 255, 1, 1, 1, 255]);
    }

    #[test]
    fn state_hides_stale_full_surface_and_reuses_it_for_the_following_delta() {
        let latest_ticket = AtomicU64::new(2);
        let state = PresentationState::default()
            .process(PresentationCommand::Reset { generation: 4 }, &latest_ticket)
            .state
            .process(
                PresentationCommand::Connected { generation: 4 },
                &latest_ticket,
            )
            .state;

        let stale_base = state.process(
            PresentationCommand::Frame {
                generation: 4,
                ticket: 1,
                frame: PresentationFrame::Bgra {
                    width: 2,
                    height: 1,
                    bgra: vec![5, 5, 5, 255, 6, 6, 6, 255],
                },
            },
            &latest_ticket,
        );
        assert!(matches!(
            stale_base.result,
            PresentationResult::Prepared(PreparedFrame {
                ticket: 1,
                kind: PreparedFrameKind::Base { encoding: "bgra" },
                surface: None,
                ..
            })
        ));
        let texture_id = stale_base.state.surface.as_ref().unwrap().texture().id;

        let latest = stale_base
            .state
            .process(delta_with_ticket(4, 2, 1), &latest_ticket);
        let PresentationResult::Prepared(PreparedFrame {
            surface: Some(surface),
            kind: PreparedFrameKind::Delta,
            ..
        }) = latest.result
        else {
            panic!("delta after stale base should materialize the full framebuffer");
        };
        assert_eq!(texture_id, surface.texture().id);
        let uploads = surface.pending_texture_uploads(0);
        assert_eq!(1, uploads.len());
        assert_eq!(uploads[0].bytes.as_slice(), &[5, 5, 5, 255, 1, 1, 1, 255]);
    }

    #[test]
    fn same_size_full_frame_reuses_texture_but_resize_replaces_it() {
        let latest_ticket = AtomicU64::new(1);
        let state = PresentationState::default()
            .process(PresentationCommand::Reset { generation: 4 }, &latest_ticket)
            .state
            .process(
                PresentationCommand::Connected { generation: 4 },
                &latest_ticket,
            )
            .state;

        let first = state.process(
            PresentationCommand::Frame {
                generation: 4,
                ticket: 1,
                frame: PresentationFrame::Bgra {
                    width: 2,
                    height: 1,
                    bgra: vec![0; 8],
                },
            },
            &latest_ticket,
        );
        let first_texture_id = first.state.surface.as_ref().unwrap().texture().id;

        let second = first.state.process(
            PresentationCommand::Frame {
                generation: 4,
                ticket: 1,
                frame: PresentationFrame::Bgra {
                    width: 2,
                    height: 1,
                    bgra: vec![1; 8],
                },
            },
            &latest_ticket,
        );
        let second_texture_id = second.state.surface.as_ref().unwrap().texture().id;
        assert_eq!(first_texture_id, second_texture_id);

        let resized = second.state.process(
            PresentationCommand::Frame {
                generation: 4,
                ticket: 1,
                frame: PresentationFrame::Bgra {
                    width: 3,
                    height: 1,
                    bgra: vec![2; 12],
                },
            },
            &latest_ticket,
        );
        let resized_texture_id = resized.state.surface.as_ref().unwrap().texture().id;
        assert_ne!(second_texture_id, resized_texture_id);
    }
}

use one_core::storage::RemoteDesktopBackendPreference;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemoteDesktopPresentation {
    Canvas,
    NativeWindows,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemoteDesktopPlatform {
    Windows,
    Other,
}

/// A pre-connect reason that makes the native backend predictably unavailable.
///
/// Only this classification may trigger the `Auto` preference's Canvas
/// fallback. Runtime authentication, certificate, Gateway, server-policy, and
/// network failures never enter this model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowsNativeRdpUnavailableReason {
    FeatureDisabled,
    UnsupportedPlatform,
    ProbeReportedUnavailable,
    ClassNotRegistered,
    RequiredInterfaceMissing,
    SharedFoldersUnsupported,
}

/// A stable, non-fallback classification for failures while probing the native
/// boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowsNativeRdpProbeFailure {
    InvalidArgument,
    AbiMismatch,
    AllocationFailed,
    Internal,
    WrongThread,
    CallbackInFlight,
    InvalidState,
    NativeReturnedNullHandle,
    NativeDidNotClearHandle,
    InvalidNativeResponse,
    UnexpectedNativeResult(i32),
    NativeHresult { result: i32, hresult: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowsNativeRdpCapability {
    Available,
    Unavailable(WindowsNativeRdpUnavailableReason),
    ProbeFailed(WindowsNativeRdpProbeFailure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RemoteDesktopPresentationSelection {
    pub(crate) presentation: RemoteDesktopPresentation,
    pub(crate) fallback_reason: Option<WindowsNativeRdpUnavailableReason>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemoteDesktopPresentationError {
    NativeUnavailable(WindowsNativeRdpUnavailableReason),
    NativeProbeFailed(WindowsNativeRdpProbeFailure),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RemoteDesktopPresentationCreation<T> {
    Canvas {
        fallback_reason: Option<WindowsNativeRdpUnavailableReason>,
    },
    Native(T),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RemoteDesktopPresentationCreateError<E> {
    Selection(RemoteDesktopPresentationError),
    NativeCreate(E),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemoteDesktopPresentationInitialization {
    Pending,
    Canvas {
        fallback_reason: Option<WindowsNativeRdpUnavailableReason>,
    },
    Native,
    Failed {
        attempted_presentation: RemoteDesktopPresentation,
        canvas_retry_available: bool,
    },
}

impl RemoteDesktopPresentationInitialization {
    pub(crate) const fn presentation(self) -> Option<RemoteDesktopPresentation> {
        match self {
            Self::Pending => None,
            Self::Canvas { .. } => Some(RemoteDesktopPresentation::Canvas),
            Self::Native => Some(RemoteDesktopPresentation::NativeWindows),
            Self::Failed {
                attempted_presentation,
                ..
            } => Some(attempted_presentation),
        }
    }

    pub(crate) const fn fallback_reason(self) -> Option<WindowsNativeRdpUnavailableReason> {
        match self {
            Self::Canvas { fallback_reason } => fallback_reason,
            Self::Pending | Self::Native | Self::Failed { .. } => None,
        }
    }

    pub(crate) const fn allows_canvas_runtime(self) -> bool {
        matches!(self, Self::Canvas { .. })
    }

    pub(crate) const fn allows_explicit_canvas_retry(self) -> bool {
        matches!(
            self,
            Self::Failed {
                canvas_retry_available: true,
                ..
            }
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemoteDesktopPresentationState {
    Created,
    NativeChildCreated,
    Connecting,
    Active,
    Inactive,
    Reconnecting,
    Closing,
    NativeChildDestroyed,
    Released,
}

#[cfg(feature = "windows-native-rdp")]
pub(crate) const fn classify_windows_native_create_error(
    error: windows_rdp_host::WindowsRdpHostError,
) -> Option<WindowsNativeRdpUnavailableReason> {
    use windows_rdp_host::WindowsRdpHresultKind;

    match error.hresult() {
        Some(hresult) => match hresult.kind() {
            WindowsRdpHresultKind::ClassNotRegistered => {
                Some(WindowsNativeRdpUnavailableReason::ClassNotRegistered)
            }
            WindowsRdpHresultKind::NoInterface => {
                Some(WindowsNativeRdpUnavailableReason::RequiredInterfaceMissing)
            }
            _ => None,
        },
        None => None,
    }
}

pub(crate) const fn select_remote_desktop_presentation(
    platform: RemoteDesktopPlatform,
    preference: RemoteDesktopBackendPreference,
    native_capability: WindowsNativeRdpCapability,
) -> Result<RemoteDesktopPresentationSelection, RemoteDesktopPresentationError> {
    if matches!(platform, RemoteDesktopPlatform::Other) {
        return Ok(canvas_selection(None));
    }

    classify_windows_presentation(preference, native_capability)
}

pub(crate) fn create_remote_desktop_presentation_with<T, E>(
    platform: RemoteDesktopPlatform,
    preference: RemoteDesktopBackendPreference,
    probe_native: impl FnOnce() -> WindowsNativeRdpCapability,
    create_native: impl FnOnce() -> Result<T, E>,
    classify_create_error: impl FnOnce(&E) -> Option<WindowsNativeRdpUnavailableReason>,
) -> Result<RemoteDesktopPresentationCreation<T>, RemoteDesktopPresentationCreateError<E>> {
    if matches!(platform, RemoteDesktopPlatform::Other)
        || matches!(preference, RemoteDesktopBackendPreference::Canvas)
    {
        return Ok(RemoteDesktopPresentationCreation::Canvas {
            fallback_reason: None,
        });
    }

    let selection = select_remote_desktop_presentation(platform, preference, probe_native())
        .map_err(RemoteDesktopPresentationCreateError::Selection)?;
    if matches!(selection.presentation, RemoteDesktopPresentation::Canvas) {
        return Ok(RemoteDesktopPresentationCreation::Canvas {
            fallback_reason: selection.fallback_reason,
        });
    }

    match create_native() {
        Ok(native) => Ok(RemoteDesktopPresentationCreation::Native(native)),
        Err(error) => {
            if matches!(preference, RemoteDesktopBackendPreference::Auto)
                && let Some(reason) = classify_create_error(&error)
            {
                return Ok(RemoteDesktopPresentationCreation::Canvas {
                    fallback_reason: Some(reason),
                });
            }
            Err(RemoteDesktopPresentationCreateError::NativeCreate(error))
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) const fn current_remote_desktop_platform() -> RemoteDesktopPlatform {
    RemoteDesktopPlatform::Windows
}

#[cfg(not(target_os = "windows"))]
pub(crate) const fn current_remote_desktop_platform() -> RemoteDesktopPlatform {
    RemoteDesktopPlatform::Other
}

const fn classify_windows_presentation(
    preference: RemoteDesktopBackendPreference,
    native_capability: WindowsNativeRdpCapability,
) -> Result<RemoteDesktopPresentationSelection, RemoteDesktopPresentationError> {
    match (preference, native_capability) {
        (RemoteDesktopBackendPreference::Canvas, _) => Ok(canvas_selection(None)),
        (
            RemoteDesktopBackendPreference::Auto | RemoteDesktopBackendPreference::WindowsNative,
            WindowsNativeRdpCapability::Available,
        ) => Ok(RemoteDesktopPresentationSelection {
            presentation: RemoteDesktopPresentation::NativeWindows,
            fallback_reason: None,
        }),
        (RemoteDesktopBackendPreference::Auto, WindowsNativeRdpCapability::Unavailable(reason)) => {
            Ok(canvas_selection(Some(reason)))
        }
        (
            RemoteDesktopBackendPreference::WindowsNative,
            WindowsNativeRdpCapability::Unavailable(reason),
        ) => Err(RemoteDesktopPresentationError::NativeUnavailable(reason)),
        (
            RemoteDesktopBackendPreference::Auto | RemoteDesktopBackendPreference::WindowsNative,
            WindowsNativeRdpCapability::ProbeFailed(failure),
        ) => Err(RemoteDesktopPresentationError::NativeProbeFailed(failure)),
    }
}

const fn canvas_selection(
    fallback_reason: Option<WindowsNativeRdpUnavailableReason>,
) -> RemoteDesktopPresentationSelection {
    RemoteDesktopPresentationSelection {
        presentation: RemoteDesktopPresentation::Canvas,
        fallback_reason,
    }
}
