//! Helpers for testing [`Plugin`](crate::Plugin) implementations.
//!
//! Enable the **`testing`** Cargo feature on `ferrum-flow` to use this module:
//!
//! ```toml
//! ferrum-flow = { version = "…", features = ["testing"] }
//! ```
//!
//! This harness is intended for plugin unit/integration tests in downstream crates where
//! [`InitPluginContext`], [`PluginContext`] and [`RenderContext`] constructors are intentionally
//! not public.

use gpui::{AnyElement, Context, Pixels, Size, Window, px};
use std::time::Duration;

use crate::{
    EventResult, FlowCanvas, FlowEvent, FlowTheme, Graph, LocalHistory, Plugin, PluginContext,
    RenderContext, RendererRegistry, SharedState, SyncPlugin, Viewport,
    canvas::{InteractionState, PortLayoutCache},
    plugin::InitPluginContext,
};

/// Test harness that can drive plugin `setup`, `on_event`, and `render` with realistic internal
/// contexts.
pub struct PluginTestHarness {
    pub graph: Graph,
    pub port_offset_cache: PortLayoutCache,
    pub viewport: Viewport,
    pub interaction: InteractionState,
    pub renderers: RendererRegistry,
    pub history: LocalHistory,
    pub theme: FlowTheme,
    pub shared_state: SharedState,
    sync_plugin: Option<Box<dyn SyncPlugin + 'static>>,
    emitted_events: Vec<FlowEvent>,
    notify_count: usize,
}

impl PluginTestHarness {
    pub fn new(graph: Graph) -> Self {
        Self {
            graph,
            port_offset_cache: PortLayoutCache::new(),
            viewport: Viewport::new(),
            interaction: InteractionState::new(),
            renderers: RendererRegistry::new(),
            history: LocalHistory::new(),
            theme: FlowTheme::default(),
            shared_state: SharedState::new(),
            sync_plugin: None,
            emitted_events: Vec::new(),
            notify_count: 0,
        }
    }

    /// Runs `Plugin::setup`.
    ///
    /// Call this only in tests that already have a GPUI context/window.
    pub fn run_setup<'a, 'b>(
        &mut self,
        plugin: &mut dyn Plugin,
        gpui_ctx: &'a Context<'b, FlowCanvas>,
        drawable_size: Size<Pixels>,
    ) {
        let mut ctx = InitPluginContext::new(
            &mut self.graph,
            &mut self.port_offset_cache,
            &mut self.viewport,
            &mut self.renderers,
            gpui_ctx,
            drawable_size,
            &mut self.theme,
            &mut self.shared_state,
        );
        plugin.setup(&mut ctx);
    }

    /// Runs `Plugin::on_event` once and captures emitted events / notify calls.
    pub fn run_event(&mut self, plugin: &mut dyn Plugin, event: FlowEvent) -> EventResult {
        let emitted_events = &mut self.emitted_events;
        let notify_count = &mut self.notify_count;
        let mut emit = |e: FlowEvent| {
            emitted_events.push(e);
        };
        let mut notify = || {
            *notify_count += 1;
        };
        let mut schedule_after = |_delay: Duration| {};
        let mut ctx = PluginContext::new(
            &mut self.graph,
            &mut self.port_offset_cache,
            &mut self.viewport,
            &mut self.interaction,
            &mut self.renderers,
            &mut self.sync_plugin,
            &mut self.history,
            &mut self.theme,
            &mut self.shared_state,
            &mut emit,
            &mut notify,
            &mut schedule_after,
        );
        plugin.on_event(&event, &mut ctx)
    }

    /// Runs `Plugin::render` once.
    ///
    /// Call this only in tests that already have a GPUI window.
    pub fn run_render(&mut self, plugin: &mut dyn Plugin, window: &Window) -> Option<AnyElement> {
        let mut ctx = RenderContext::new(
            &mut self.graph,
            &mut self.port_offset_cache,
            &self.viewport,
            &self.renderers,
            window,
            &self.theme,
            &self.shared_state,
        );
        plugin.render(&mut ctx)
    }

    /// Returns number of times `ctx.notify()` was called during `run_event`.
    pub fn notify_count(&self) -> usize {
        self.notify_count
    }

    /// Drains and returns custom/input events emitted via `ctx.emit(...)`.
    pub fn drain_emitted_events(&mut self) -> Vec<FlowEvent> {
        std::mem::take(&mut self.emitted_events)
    }
}

impl Default for PluginTestHarness {
    fn default() -> Self {
        let mut harness = Self::new(Graph::new());
        harness.viewport.set_window_bounds(Some(gpui::Bounds::new(
            gpui::Point::new(px(0.0), px(0.0)),
            gpui::Size::new(px(800.0), px(600.0)),
        )));
        harness
    }
}
