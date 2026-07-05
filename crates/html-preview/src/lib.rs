//! Shared logic for rendering and transforming chat HTML previews.

use std::collections::BTreeMap;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::{OnceLock, RwLock};

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

const DOCTYPE: &str = "<!doctype html>";
const TOOLBAR_ACTION_IDS: [&str; 5] = [
    "html-preview",
    "html-source",
    "html-copy",
    "html-download",
    "html-open-window",
];
const AUTO_CLOSE_TAGS: [&str; 9] = [
    "main", "section", "article", "div", "span", "p", "ul", "ol", "li",
];
const EXTENSION_ASSET_SCHEME: &str = "onet-extension://";
static EXTENSION_ASSET_ROOTS: OnceLock<RwLock<BTreeMap<String, PathBuf>>> = OnceLock::new();
static HTML_PREVIEW_TRANSFORM_PROVIDER: OnceLock<RwLock<Option<HtmlPreviewTransformProvider>>> =
    OnceLock::new();

pub type HtmlPreviewTransformResult = Result<Option<HtmlPreviewTransformOutput>, String>;
pub type HtmlPreviewTransformProvider =
    Arc<dyn Fn(String, String) -> BoxFuture<'static, HtmlPreviewTransformResult> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlPreviewAsset {
    pub path: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlPreviewTransformOutput {
    pub html: String,
    #[serde(default)]
    pub assets: Vec<HtmlPreviewAsset>,
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
    OpenWindow,
}

impl HtmlPreviewAction {
    pub fn toolbar_ids() -> Vec<&'static str> {
        TOOLBAR_ACTION_IDS.to_vec()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlPreviewAssetResolver {
    extension_id: String,
    extension_root: PathBuf,
}

impl HtmlPreviewAssetResolver {
    pub fn new(extension_id: impl Into<String>, extension_root: impl Into<PathBuf>) -> Self {
        Self {
            extension_id: extension_id.into(),
            extension_root: extension_root.into(),
        }
    }

    pub fn resolve(&self, path: &str) -> Option<String> {
        let path = path.trim();
        if !is_safe_relative_asset_path(path) {
            return None;
        }
        Some(format!("onet-extension://{}/{}", self.extension_id, path))
    }

    pub fn extension_root(&self) -> &PathBuf {
        &self.extension_root
    }
}

pub fn normalize_html_document(source: &str) -> String {
    let trimmed = source.trim();
    if is_complete_document(trimmed) {
        return trimmed.to_string();
    }

    let without_doctype = strip_doctype(trimmed);
    let without_html = strip_html_shell(without_doctype);
    let (head, body) = split_head_and_body(without_html);
    format!("{DOCTYPE}<html><head>{head}</head><body>{body}</body></html>")
}

pub fn register_extension_asset_root(
    extension_id: impl Into<String>,
    assets_root: impl Into<PathBuf>,
) {
    let roots = EXTENSION_ASSET_ROOTS.get_or_init(|| RwLock::new(BTreeMap::new()));
    if let Ok(mut roots) = roots.write() {
        roots.insert(extension_id.into(), assets_root.into());
    }
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
    let provider = html_preview_transform_provider();
    match provider {
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

pub fn download_html_preview_document(document: &HtmlPreviewDocument) -> io::Result<PathBuf> {
    let dir = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or(std::env::current_dir()?);
    write_html_preview_document_to_dir(document, dir)
}

pub fn write_html_preview_document_to_dir(
    document: &HtmlPreviewDocument,
    dir: impl AsRef<Path>,
) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir.as_ref())?;
    let path = available_html_preview_path(dir.as_ref());
    std::fs::write(&path, document.render_html())?;
    Ok(path)
}

pub fn resolve_extension_asset_url(url: &str) -> Option<PathBuf> {
    let (extension_id, path) = parse_extension_asset_url(url)?;
    if !is_safe_relative_asset_path(path) {
        return None;
    }
    let roots = EXTENSION_ASSET_ROOTS.get_or_init(|| RwLock::new(BTreeMap::new()));
    let roots = roots.read().ok()?;
    let root = roots.get(extension_id)?;
    Some(root.join(path).components().collect())
}

fn html_preview_transform_provider() -> Option<HtmlPreviewTransformProvider> {
    HTML_PREVIEW_TRANSFORM_PROVIDER
        .get_or_init(|| RwLock::new(None))
        .read()
        .ok()?
        .clone()
}

fn available_html_preview_path(dir: &Path) -> PathBuf {
    let first = dir.join("onetcli-html-preview.html");
    if !first.exists() {
        return first;
    }
    for index in 1.. {
        let candidate = dir.join(format!("onetcli-html-preview-{index}.html"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn parse_extension_asset_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix(EXTENSION_ASSET_SCHEME)?;
    let (extension_id, path) = rest.split_once('/')?;
    (!extension_id.is_empty() && !path.is_empty()).then_some((extension_id, path))
}

fn is_complete_document(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    lower.starts_with(DOCTYPE) && lower.contains("<html") && lower.ends_with("</html>")
}

fn strip_doctype(source: &str) -> &str {
    if source.to_ascii_lowercase().starts_with(DOCTYPE) {
        source[DOCTYPE.len()..].trim()
    } else {
        source
    }
}

fn strip_html_shell(source: &str) -> &str {
    let lower = source.to_ascii_lowercase();
    if !lower.starts_with("<html") {
        return source;
    }
    let start = source.find('>').map(|ix| ix + 1).unwrap_or(0);
    let end = lower.rfind("</html>").unwrap_or(source.len());
    source[start..end].trim()
}

fn split_head_and_body(source: &str) -> (String, String) {
    if let Some((head, body)) = split_with_body_tag(source) {
        return (clean_head(head), close_body_tags(body));
    }
    if let Some((head, body)) = split_with_head_tag(source) {
        return (clean_head(head), close_body_tags(body));
    }
    (String::new(), close_body_tags(source))
}

fn split_with_body_tag(source: &str) -> Option<(&str, &str)> {
    let lower = source.to_ascii_lowercase();
    let body_start = lower.find("<body")?;
    let body_open_end = source[body_start..].find('>')? + body_start + 1;
    let body_end = lower.rfind("</body>").unwrap_or(source.len());
    Some((
        &source[..body_start],
        source[body_open_end..body_end].trim(),
    ))
}

fn split_with_head_tag(source: &str) -> Option<(&str, &str)> {
    let lower = source.to_ascii_lowercase();
    let head_start = lower.find("<head")?;
    let head_open_end = source[head_start..].find('>')? + head_start + 1;
    let head_end = lower.find("</head>").unwrap_or(head_open_end);
    Some((
        source[head_open_end..head_end].trim(),
        source[(head_end + "</head>".len()).min(source.len())..].trim(),
    ))
}

fn clean_head(source: &str) -> String {
    let lower = source.to_ascii_lowercase();
    let mut head = source.trim();
    if let Some(ix) = lower.find("<head") {
        let open_end = source[ix..].find('>').map(|end| ix + end + 1).unwrap_or(ix);
        head = &source[open_end..];
    }
    head.replace("</head>", "").trim().to_string()
}

fn close_body_tags(source: &str) -> String {
    let mut body = source.trim().replace("</body>", "");
    for tag in AUTO_CLOSE_TAGS {
        let opens = count_open_tags(&body, tag);
        let closes = body
            .to_ascii_lowercase()
            .matches(&format!("</{tag}>"))
            .count();
        for _ in closes..opens {
            body.push_str(&format!("</{tag}>"));
        }
    }
    body
}

fn count_open_tags(source: &str, tag: &str) -> usize {
    let lower = source.to_ascii_lowercase();
    lower
        .match_indices(&format!("<{tag}"))
        .filter(|(ix, _)| {
            lower[*ix + tag.len() + 1..]
                .chars()
                .next()
                .is_some_and(|ch| ch == '>' || ch.is_ascii_whitespace())
        })
        .count()
}

fn is_safe_relative_asset_path(path: &str) -> bool {
    if path.is_empty() || path.contains("://") || path.starts_with('/') {
        return false;
    }
    PathBuf::from(path)
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
}
