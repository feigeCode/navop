use std::sync::Arc;
use std::sync::{OnceLock, RwLock};

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

use crate::{HtmlPreviewAsset, HtmlPreviewDocument};

static HTML_PREVIEW_TRANSFORM_PROVIDER: OnceLock<RwLock<Option<HtmlPreviewTransformProvider>>> =
    OnceLock::new();

pub type HtmlPreviewTransformResult = Result<Option<HtmlPreviewTransformOutput>, String>;
pub type HtmlPreviewTransformProvider =
    Arc<dyn Fn(String, String) -> BoxFuture<'static, HtmlPreviewTransformResult> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlPreviewTransformOutput {
    pub html: String,
    #[serde(default)]
    pub assets: Vec<HtmlPreviewAsset>,
}

pub fn set_html_preview_transform_provider<F>(provider: F)
where
    F: Fn(String, String) -> BoxFuture<'static, HtmlPreviewTransformResult> + Send + Sync + 'static,
{
    let provider = Arc::new(provider);
    let slot = HTML_PREVIEW_TRANSFORM_PROVIDER.get_or_init(|| RwLock::new(None));
    if let Ok(mut slot) = slot.write() {
        *slot = Some(provider);
    }
}

pub fn clear_html_preview_transform_provider() {
    let slot = HTML_PREVIEW_TRANSFORM_PROVIDER.get_or_init(|| RwLock::new(None));
    if let Ok(mut slot) = slot.write() {
        *slot = None;
    }
}

pub async fn transform_html_preview(
    language: impl Into<String>,
    html: impl Into<String>,
) -> HtmlPreviewTransformResult {
    let language = language.into();
    let html = html.into();
    match html_preview_transform_provider() {
        Some(provider) => provider(language, html).await,
        None => Ok(None),
    }
}

pub async fn transform_html_preview_document(
    language: impl Into<String>,
    html: impl Into<String>,
) -> Result<HtmlPreviewDocument, String> {
    let language = language.into();
    let html = html.into();
    let mut document = HtmlPreviewDocument::new(language.clone(), html.clone());
    if let Some(transform) = transform_html_preview(language, html).await? {
        document.apply_transform(transform);
    }
    Ok(document)
}

fn html_preview_transform_provider() -> Option<HtmlPreviewTransformProvider> {
    HTML_PREVIEW_TRANSFORM_PROVIDER
        .get_or_init(|| RwLock::new(None))
        .read()
        .ok()?
        .clone()
}
