#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
#![cfg_attr(not(test), deny(clippy::panic))]

pub mod builder_state;
mod canvas;
#[cfg(any(feature = "testing", test))]
pub mod command_interop;
mod edge;
mod graph;
mod plugin;
#[cfg(any(feature = "testing", test))]
pub mod plugin_testing;
mod plugins;
mod port_screen;
mod shared_state;
mod theme;
mod viewport;

pub use canvas::{
    Command, CommandContext, CompositeCommand, FlowCanvas, FlowCanvasOutbound, HistoryProvider,
    Interaction, InteractionResult, InteractionState, LocalHistory, NodeRenderer, RendererRegistry,
    default_node_caption,
};
pub use edge::*;
pub use graph::node::*;
pub use graph::*;
pub use plugin::{
    EventResult, FlowEvent, InitPluginContext, InputEvent, NodeCardVariant, Plugin, PluginContext,
    RenderContext, RenderLayer, SyncPlugin, SyncPluginContext, primary_platform_modifier,
};
pub use plugins::*;
pub use port_screen::PortScreenFrame;
pub use shared_state::SharedState;
pub use theme::FlowTheme;
pub use viewport::Viewport;
