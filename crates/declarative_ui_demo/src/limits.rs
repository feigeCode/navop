use std::fmt;

pub const DEFAULT_MAX_SOURCE_BYTES: usize = 256 * 1024;
pub const DEFAULT_MAX_NODES: usize = 10_000;
pub const DEFAULT_MAX_DEPTH: usize = 64;
pub const DEFAULT_MAX_ATTRIBUTES: usize = 20_000;
pub const DEFAULT_MAX_CLASSES: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompileLimits {
    pub max_source_bytes: usize,
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_attributes: usize,
    pub max_classes: usize,
}

impl CompileLimits {
    pub const DEFAULT: Self = Self {
        max_source_bytes: DEFAULT_MAX_SOURCE_BYTES,
        max_nodes: DEFAULT_MAX_NODES,
        max_depth: DEFAULT_MAX_DEPTH,
        max_attributes: DEFAULT_MAX_ATTRIBUTES,
        max_classes: DEFAULT_MAX_CLASSES,
    };
}

impl Default for CompileLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseResource {
    SourceBytes,
    Nodes,
    Depth,
    Attributes,
    Classes,
}

impl fmt::Display for ParseResource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::SourceBytes => "source bytes",
            Self::Nodes => "nodes",
            Self::Depth => "element depth",
            Self::Attributes => "attributes",
            Self::Classes => "class tokens",
        };
        formatter.write_str(name)
    }
}
