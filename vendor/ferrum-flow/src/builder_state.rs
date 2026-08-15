//! Typestate markers shared by [`crate::NodeBuilder`], [`crate::EdgeBuilder`], and future builders.

/// Builder field is not set yet.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Unset;

/// Builder field holds `T`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Set<T>(pub T);
