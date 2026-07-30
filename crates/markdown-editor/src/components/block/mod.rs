//! Block runtime and semantic state.
//!
//! This module groups the block entity itself, block-level Markdown parsing,
//! inline text-tree handling, rendering, input bridging, and interaction
//! handlers. A block owns local editing state while the editor owns tree
//! structure and cross-block mutations.

mod element;
mod input;
mod interactions;
mod render;
mod runtime;
mod state;

pub(crate) use crate::components::markdown::code_highlight::*;
pub(crate) use crate::components::markdown::footnote::*;
pub(crate) use crate::components::markdown::image::*;
pub use crate::components::markdown::inline::*;
pub(crate) use crate::components::markdown::link::*;
pub use runtime::*;
pub use state::*;
