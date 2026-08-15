//! Automatic graph layout.
//!
//! - [`strategy`] — [`LayoutStrategy`], [`LayoutPhase`], [`PositionHint`], [`LayoutOptions`], …
//! - [`layered_dag`] — [`LayeredDagLayout`] (longest-path layers; cycles → grid fallback).
//! - [`force_directed`] — [`ForceDirectedLayout`] (FR-style; cycles OK).
//! - [`pack_isolated`] — [`PackIsolatedNodesLayout`] (optional strip for isolated nodes).
//! - [`pipeline`] — [`LayoutPipeline`] (ordered stages + one combined delta).
//! - [`AutoLayoutPlugin`] — optional single strategy + shortcut; see that type’s docs.

mod auto_layout;
pub mod force_directed;
pub mod layered_dag;
mod pack_isolated;
pub mod pipeline;
mod strategy;

pub use auto_layout::AutoLayoutPlugin;
pub use force_directed::ForceDirectedLayout;
pub use layered_dag::LayeredDagLayout;
pub use pack_isolated::PackIsolatedNodesLayout;
pub use pipeline::LayoutPipeline;
pub use strategy::{
    LayoutDirection, LayoutError, LayoutOptions, LayoutOutput, LayoutPhase, LayoutStrategy,
    NodePositionDelta, PositionHint,
};
