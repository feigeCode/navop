use std::{error::Error, fmt};

use gpui::{App, Hsla, IntoElement, RenderOnce, StyleRefinement, Styled, Transformation, Window};

use crate::{Sizable, Size};

use super::{Icon, IconKind, IconName};

/// Error returned when an icon is used with the wrong semantic wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IconKindMismatch {
    expected: &'static str,
    actual: IconKind,
    icon: IconName,
}

impl IconKindMismatch {
    fn new(expected: &'static str, actual: IconKind, icon: IconName) -> Self {
        Self {
            expected,
            actual,
            icon,
        }
    }

    /// The semantic family expected by the wrapper.
    pub const fn expected(&self) -> &'static str {
        self.expected
    }

    /// The icon's actual semantic family.
    pub const fn actual(&self) -> IconKind {
        self.actual
    }

    /// The icon that failed validation.
    pub const fn icon(&self) -> IconName {
        self.icon
    }
}

impl fmt::Display for IconKindMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "icon {:?} has kind {:?}, expected {}",
            self.icon, self.actual, self.expected
        )
    }
}

impl Error for IconKindMismatch {}

/// A monochrome action or control icon.
#[derive(IntoElement, Clone)]
pub struct FunctionalIcon {
    icon: Icon,
}

impl FunctionalIcon {
    /// Creates a functional icon and panics when the supplied icon is not functional.
    pub fn new(name: IconName) -> Self {
        Self::try_new(name).unwrap_or_else(|error| panic!("{error}"))
    }

    /// Creates a functional icon after validating its metadata.
    pub fn try_new(name: IconName) -> Result<Self, IconKindMismatch> {
        let actual = name.kind();
        if !actual.is_functional() {
            return Err(IconKindMismatch::new("a functional icon", actual, name));
        }

        Ok(Self {
            icon: Icon::new(name).mono(),
        })
    }

    /// Applies an explicit monochrome tint.
    pub fn text_color(mut self, color: impl Into<Hsla>) -> Self {
        self.icon = self.icon.text_color(color);
        self
    }

    /// Rotates the icon by the given angle.
    pub fn rotate(mut self, radians: impl Into<gpui::Radians>) -> Self {
        self.icon = self.icon.rotate(radians);
        self
    }

    /// Applies an arbitrary GPUI transformation.
    pub fn transform(mut self, transformation: Transformation) -> Self {
        self.icon = self.icon.transform(transformation);
        self
    }

    /// Converts the semantic wrapper back into the base icon type.
    pub fn into_icon(self) -> Icon {
        self.icon
    }
}

impl Styled for FunctionalIcon {
    fn style(&mut self) -> &mut StyleRefinement {
        self.icon.style()
    }

    fn text_color(self, color: impl Into<Hsla>) -> Self {
        FunctionalIcon::text_color(self, color)
    }
}

impl Sizable for FunctionalIcon {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.icon = self.icon.with_size(size);
        self
    }
}

impl From<FunctionalIcon> for Icon {
    fn from(icon: FunctionalIcon) -> Self {
        icon.into_icon()
    }
}

impl RenderOnce for FunctionalIcon {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.into_icon()
    }
}

/// An original-color icon that identifies a product, platform, or database brand.
#[derive(IntoElement, Clone)]
pub struct BrandIcon {
    icon: Icon,
}

impl BrandIcon {
    /// Creates a brand icon and panics when the supplied icon is not a brand.
    pub fn new(name: IconName) -> Self {
        Self::try_new(name).unwrap_or_else(|error| panic!("{error}"))
    }

    /// Creates a brand icon after validating its metadata.
    pub fn try_new(name: IconName) -> Result<Self, IconKindMismatch> {
        let actual = name.kind();
        if actual != IconKind::BrandColor {
            return Err(IconKindMismatch::new("a brand icon", actual, name));
        }

        Ok(Self {
            icon: Icon::new(name).color(),
        })
    }

    /// Converts the semantic wrapper back into the base icon type.
    pub fn into_icon(self) -> Icon {
        self.icon
    }
}

impl Sizable for BrandIcon {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.icon = self.icon.with_size(size);
        self
    }
}

impl From<BrandIcon> for Icon {
    fn from(icon: BrandIcon) -> Self {
        icon.into_icon()
    }
}

impl RenderOnce for BrandIcon {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.into_icon()
    }
}

/// A monochrome icon representing an application or domain object.
#[derive(IntoElement, Clone)]
pub struct ObjectIcon {
    icon: Icon,
}

impl ObjectIcon {
    /// Creates an object icon and panics when the supplied icon is not an object glyph.
    pub fn new(name: IconName) -> Self {
        Self::try_new(name).unwrap_or_else(|error| panic!("{error}"))
    }

    /// Creates an object icon after validating its metadata.
    pub fn try_new(name: IconName) -> Result<Self, IconKindMismatch> {
        let actual = name.kind();
        if actual != IconKind::ObjectGlyph {
            return Err(IconKindMismatch::new("an object icon", actual, name));
        }

        Ok(Self {
            icon: Icon::new(name).mono(),
        })
    }

    /// Applies an explicit monochrome tint.
    pub fn text_color(mut self, color: impl Into<Hsla>) -> Self {
        self.icon = self.icon.text_color(color);
        self
    }

    /// Converts the semantic wrapper back into the base icon type.
    pub fn into_icon(self) -> Icon {
        self.icon
    }
}

impl Styled for ObjectIcon {
    fn style(&mut self) -> &mut StyleRefinement {
        self.icon.style()
    }

    fn text_color(self, color: impl Into<Hsla>) -> Self {
        ObjectIcon::text_color(self, color)
    }
}

impl Sizable for ObjectIcon {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.icon = self.icon.with_size(size);
        self
    }
}

impl From<ObjectIcon> for Icon {
    fn from(icon: ObjectIcon) -> Self {
        icon.into_icon()
    }
}

impl RenderOnce for ObjectIcon {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.into_icon()
    }
}

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::{BrandIcon, FunctionalIcon, IconKindMismatch, ObjectIcon};
    use crate::{
        Icon, IconColorMode, IconName, IconSize, Sizable, Size, button::ButtonIconVariant,
    };

    #[test]
    fn wrappers_apply_the_required_color_mode() {
        let functional = FunctionalIcon::new(IconName::Plus).into_icon();
        let brand = BrandIcon::new(IconName::PostgreSQLColor).into_icon();
        let object = ObjectIcon::new(IconName::Table).into_icon();

        assert_eq!(functional.color_mode, IconColorMode::Mono);
        assert_eq!(brand.color_mode, IconColorMode::Color);
        assert_eq!(object.color_mode, IconColorMode::Mono);
    }

    #[test]
    fn wrappers_reject_the_wrong_semantic_kind() {
        let error = match BrandIcon::try_new(IconName::Plus) {
            Ok(_) => panic!("functional icon unexpectedly passed brand validation"),
            Err(error) => error,
        };

        assert_eq!(error.icon(), IconName::Plus);
        assert_eq!(error.expected(), "a brand icon");
        assert!(matches!(error, IconKindMismatch { .. }));
    }

    #[test]
    fn wrappers_accept_icon_size_tokens() {
        let functional = FunctionalIcon::new(IconName::Plus)
            .with_size(IconSize::Medium)
            .into_icon();
        let brand = BrandIcon::new(IconName::PostgreSQLColor)
            .with_size(IconSize::Hero)
            .into_icon();

        assert_eq!(functional.size, Some(Size::Size(px(20.))));
        assert_eq!(brand.size, Some(Size::Size(px(40.))));
    }

    #[test]
    fn wrappers_convert_to_icon_and_button_icon_variant() {
        let _: Icon = FunctionalIcon::new(IconName::Plus).into();
        let _: ButtonIconVariant = ObjectIcon::new(IconName::Table).into();
    }
}
