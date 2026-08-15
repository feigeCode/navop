//! Keyboard entry point for automatic graph layout.
//!
//! Holds at most **one** [`LayoutStrategy`]. By default it is unset; use
//! [`AutoLayoutPlugin::strategy`] when wiring the plugin to supply a built-in or custom
//! implementation, then **⌘⇧G** / **Ctrl⇧G** runs [`LayoutStrategy::compute`] and applies
//! [`LayoutOutput::Delta`] via [`DragNodesCommand`].
//!
//! [`crate::ToastPlugin`] is optional: warnings are emitted on [`LayoutError`] only when toast is
//! registered.

use std::sync::Arc;

use crate::{
    plugin::{FlowEvent, Plugin, PluginContext, primary_platform_modifier},
    plugins::node::DragNodesCommand,
};

use super::{LayoutOptions, LayoutOutput, LayoutStrategy};

/// Runs the configured [`LayoutStrategy`] when the user hits the layout shortcut.
pub struct AutoLayoutPlugin {
    strategy: Option<Arc<dyn LayoutStrategy>>,
    options: LayoutOptions,
}

impl AutoLayoutPlugin {
    /// No strategy: shortcut is a no-op until you call [`Self::strategy`].
    pub fn new() -> Self {
        Self {
            strategy: None,
            options: LayoutOptions::default(),
        }
    }

    /// Use a concrete algorithm (built-in struct or your own `impl LayoutStrategy`).
    pub fn strategy(mut self, strategy: impl LayoutStrategy + 'static) -> Self {
        self.strategy = Some(Arc::new(strategy));
        self
    }

    pub fn options(mut self, options: LayoutOptions) -> Self {
        self.options = options;
        self
    }
}

impl Default for AutoLayoutPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for AutoLayoutPlugin {
    fn name(&self) -> &'static str {
        "auto_layout"
    }

    fn priority(&self) -> i32 {
        89
    }

    fn on_event(
        &mut self,
        event: &FlowEvent,
        ctx: &mut PluginContext,
    ) -> crate::plugin::EventResult {
        if let FlowEvent::Input(crate::plugin::InputEvent::KeyDown(ev)) = event
            && primary_platform_modifier(ev)
            && ev.keystroke.modifiers.shift
            && ev.keystroke.key == "g"
        {
            let Some(strategy) = self.strategy.as_ref() else {
                return crate::plugin::EventResult::Stop;
            };

            match strategy.compute(ctx.graph, &self.options, None) {
                Ok(LayoutOutput::Unchanged) => {}
                Ok(LayoutOutput::Delta(delta)) => {
                    if delta.has_changes() {
                        ctx.execute_command(DragNodesCommand::from_positions(delta.from, delta.to));
                        ctx.cache_all_node_port_offset();
                    }
                }
                Err(e) => {
                    ctx.emit(FlowEvent::warning(format!(
                        "Layout ({}): {e}",
                        strategy.id()
                    )));
                }
            }
            return crate::plugin::EventResult::Stop;
        }
        crate::plugin::EventResult::Continue
    }
}
