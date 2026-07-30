//! Theme configuration and global theme access.
//!
//! Themes are JSON-serializable so editor colors, spacing, and typography can
//! be swapped without changing the runtime logic.

mod theme;
pub use theme::*;
