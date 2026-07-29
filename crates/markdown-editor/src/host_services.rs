//! Host-provided rendering services for the embeddable editor.
//!
//! These types deliberately depend only on GPUI and `futures`. Hosts can adapt
//! their syntax highlighters and extension runtimes without coupling the
//! Velotype editor core to either implementation.

use std::ops::Range;
use std::sync::Arc;

use futures::future::BoxFuture;
use gpui::{FontStyle, FontWeight, Hsla};

/// Colors supplied by the embedding application.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditorHostTheme {
    pub background: Hsla,
    pub foreground: Hsla,
    pub border: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
}

/// An asynchronously rendered Markdown construct.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockRenderKind {
    Math,
    Mermaid,
    InlineMath,
}

/// Input passed to a host block renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockRenderRequest {
    pub kind: BlockRenderKind,
    pub source: String,
    pub background: Hsla,
    pub foreground: Hsla,
    pub border: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub available_width: f32,
    pub scale_factor: f32,
}

/// Host-rendered media returned to the editor.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockRenderArtifact {
    pub media_type: String,
    pub bytes: Vec<u8>,
    pub intrinsic_width: Option<f32>,
    pub intrinsic_height: Option<f32>,
}

/// Asynchronous renderer installed by the embedding application.
pub type BlockRenderProvider = Arc<
    dyn Fn(BlockRenderRequest) -> BoxFuture<'static, Result<Option<BlockRenderArtifact>, String>>
        + Send
        + Sync,
>;

/// Input passed to the host syntax highlighter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeHighlightRequest {
    pub language: Option<String>,
    pub source: String,
}

/// Concrete style for one highlighted code range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CodeHighlightStyle {
    pub color: Option<Hsla>,
    pub font_weight: Option<FontWeight>,
    pub font_style: Option<FontStyle>,
}

/// A UTF-8 byte range and its concrete host-provided style.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeHighlightSpan {
    pub range: Range<usize>,
    pub style: CodeHighlightStyle,
}

/// Complete host syntax-highlight result.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CodeHighlightResult {
    pub spans: Vec<CodeHighlightSpan>,
}

/// Synchronous syntax-highlighting callback.
pub type CodeHighlightProvider =
    Arc<dyn Fn(CodeHighlightRequest) -> Result<CodeHighlightResult, String> + Send + Sync>;

/// Syntax highlighter plus a revision source for dynamically registered
/// grammars.
#[derive(Clone)]
pub struct CodeHighlightService {
    provider: CodeHighlightProvider,
    revision_provider: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl CodeHighlightService {
    pub fn new(
        provider: CodeHighlightProvider,
        revision_provider: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Self {
        Self {
            provider,
            revision_provider,
        }
    }

    pub fn highlight(&self, request: CodeHighlightRequest) -> Result<CodeHighlightResult, String> {
        (self.provider)(request)
    }

    pub fn revision(&self) -> u64 {
        (self.revision_provider)()
    }
}

/// Complete host integration installed on an editor.
#[derive(Clone)]
pub struct EditorHostServices {
    theme: EditorHostTheme,
    has_theme_override: bool,
    code_highlighter: Option<CodeHighlightService>,
    block_renderer: Option<BlockRenderProvider>,
}

impl Default for EditorHostServices {
    fn default() -> Self {
        Self {
            theme: EditorHostTheme::default(),
            has_theme_override: false,
            code_highlighter: None,
            block_renderer: None,
        }
    }
}

impl EditorHostServices {
    pub fn new(theme: EditorHostTheme) -> Self {
        Self {
            theme,
            has_theme_override: true,
            ..Self::default()
        }
    }

    pub fn with_code_highlighter(mut self, service: CodeHighlightService) -> Self {
        self.code_highlighter = Some(service);
        self
    }

    pub fn with_block_renderer(mut self, provider: BlockRenderProvider) -> Self {
        self.block_renderer = Some(provider);
        self
    }

    pub fn theme(&self) -> &EditorHostTheme {
        &self.theme
    }

    /// Returns the host palette only when the embedding application supplied
    /// one explicitly. Standalone editors keep using the active Velotype theme
    /// instead of treating `Hsla::default()` as an intentional all-black
    /// palette.
    pub fn theme_override(&self) -> Option<&EditorHostTheme> {
        self.has_theme_override.then_some(&self.theme)
    }

    pub fn code_highlighter(&self) -> Option<&CodeHighlightService> {
        self.code_highlighter.as_ref()
    }

    pub fn block_renderer(&self) -> Option<&BlockRenderProvider> {
        self.block_renderer.as_ref()
    }
}
