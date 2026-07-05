use serde::{Deserialize, Serialize};

use crate::TOOLBAR_ACTION_IDS;
use crate::normalize_html_document;
use crate::transform::HtmlPreviewTransformOutput;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlPreviewAsset {
    pub path: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlPreviewDocument {
    language: String,
    source_html: String,
    render_html: String,
    assets: Vec<HtmlPreviewAsset>,
}

impl HtmlPreviewDocument {
    pub fn new(language: impl Into<String>, source_html: impl Into<String>) -> Self {
        let source_html = source_html.into();
        let render_html = normalize_html_document(&source_html);
        Self {
            language: language.into(),
            source_html,
            render_html,
            assets: Vec::new(),
        }
    }

    pub fn from_transform(
        language: impl Into<String>,
        transform: HtmlPreviewTransformOutput,
    ) -> Self {
        let mut document = Self::new(language, transform.html);
        document.assets = transform.assets;
        document
    }

    pub fn apply_transform(&mut self, transform: HtmlPreviewTransformOutput) {
        self.render_html = normalize_html_document(&transform.html);
        self.assets = transform.assets;
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn source_html(&self) -> &str {
        &self.source_html
    }

    pub fn render_html(&self) -> &str {
        &self.render_html
    }

    pub fn assets(&self) -> &[HtmlPreviewAsset] {
        &self.assets
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtmlPreviewAction {
    Preview,
    Source,
    Copy,
    Download,
    OpenWindow,
}

impl HtmlPreviewAction {
    pub fn toolbar_ids() -> Vec<&'static str> {
        TOOLBAR_ACTION_IDS.to_vec()
    }
}
