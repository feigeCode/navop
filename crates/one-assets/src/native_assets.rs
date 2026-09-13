use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Embeds the `assets/icons` directory so icon paths like `icons/foo.svg`
/// resolve at runtime.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
pub struct Assets;

impl Assets {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Assets {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }
        Ok(Self::get(path).map(|file| file.data))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter_map(|p| p.starts_with(path).then(|| p.into()))
            .collect())
    }
}
