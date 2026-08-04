use gpui::{Pixels, px};

use crate::Size;

/// Absolute visual sizes for icons.
///
/// Icon dimensions are intentionally independent from the surrounding text size.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IconSize {
    Micro,
    Small,
    #[default]
    Default,
    Medium,
    Large,
    Display,
    Hero,
}

impl IconSize {
    /// Returns the absolute visual dimension for this icon size.
    pub fn pixels(self) -> Pixels {
        match self {
            Self::Micro => px(12.),
            Self::Small => px(14.),
            Self::Default => px(16.),
            Self::Medium => px(20.),
            Self::Large => px(24.),
            Self::Display => px(32.),
            Self::Hero => px(40.),
        }
    }
}

impl From<IconSize> for Size {
    fn from(size: IconSize) -> Self {
        Self::Size(size.pixels())
    }
}

pub(crate) fn resolve_icon_size(size: Option<Size>) -> Pixels {
    match size {
        None | Some(Size::Medium) => IconSize::Default.pixels(),
        Some(Size::XSmall) => IconSize::Micro.pixels(),
        Some(Size::Small) => IconSize::Small.pixels(),
        Some(Size::Large) => IconSize::Large.pixels(),
        Some(Size::Size(size)) => size,
    }
}

pub(crate) fn should_apply_resolved_size(size: Option<Size>, has_style_size: bool) -> bool {
    size.is_some() || !has_style_size
}

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::{IconSize, resolve_icon_size, should_apply_resolved_size};
    use crate::{Icon, IconName, Sizable, Size};

    #[test]
    fn icon_size_tokens_map_to_absolute_pixels() {
        assert_eq!(IconSize::Micro.pixels(), px(12.));
        assert_eq!(IconSize::Small.pixels(), px(14.));
        assert_eq!(IconSize::Default.pixels(), px(16.));
        assert_eq!(IconSize::Medium.pixels(), px(20.));
        assert_eq!(IconSize::Large.pixels(), px(24.));
        assert_eq!(IconSize::Display.pixels(), px(32.));
        assert_eq!(IconSize::Hero.pixels(), px(40.));
    }

    #[test]
    fn legacy_sizes_resolve_to_the_icon_scale() {
        assert_eq!(resolve_icon_size(None), px(16.));
        assert_eq!(resolve_icon_size(Some(Size::XSmall)), px(12.));
        assert_eq!(resolve_icon_size(Some(Size::Small)), px(14.));
        assert_eq!(resolve_icon_size(Some(Size::Medium)), px(16.));
        assert_eq!(resolve_icon_size(Some(Size::Large)), px(24.));
        assert_eq!(resolve_icon_size(Some(Size::Size(px(19.)))), px(19.));
    }

    #[test]
    fn icon_size_converts_without_losing_new_scale_values() {
        assert_eq!(Size::from(IconSize::Medium), Size::Size(px(20.)));
        assert_eq!(Size::from(IconSize::Hero), Size::Size(px(40.)));
    }

    #[test]
    fn explicit_icon_size_overrides_styled_dimensions() {
        assert!(should_apply_resolved_size(Some(Size::Small), true));
        assert!(!should_apply_resolved_size(None, true));
        assert!(should_apply_resolved_size(None, false));
    }

    #[test]
    fn public_icon_builder_accepts_icon_size_tokens() {
        let _icon = Icon::new(IconName::Plus).with_size(IconSize::Medium);
    }
}
