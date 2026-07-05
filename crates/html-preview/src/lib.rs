//! Shared logic for rendering and transforming chat HTML previews.

mod assets;
mod browser;
mod document;
mod normalize;
mod transform;

pub use assets::{
    HtmlPreviewAssetResolver, register_extension_asset_root, resolve_extension_asset_url,
};
pub use browser::{
    BrowserOpenCommand, browser_ready_html_preview_document, download_html_preview_document,
    open_html_preview_document_in_browser, system_browser_open_command,
    write_browser_html_preview_document_to_dir, write_html_preview_document_to_dir,
};
pub use document::{HtmlPreviewAction, HtmlPreviewAsset, HtmlPreviewDocument};
pub use normalize::normalize_html_document;
pub use transform::{
    HtmlPreviewTransformOutput, HtmlPreviewTransformProvider, HtmlPreviewTransformResult,
    clear_html_preview_transform_provider, set_html_preview_transform_provider,
    transform_html_preview, transform_html_preview_document,
};

const TOOLBAR_ACTION_IDS: [&str; 5] = [
    "html-preview",
    "html-source",
    "html-copy",
    "html-download",
    "html-open-window",
];
