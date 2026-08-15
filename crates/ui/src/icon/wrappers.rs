use gpui::{App, Hsla, IntoElement, RenderOnce, StyleRefinement, Styled, Transformation, Window};
use palette::IntoColor;

use crate::{Sizable, Size};

use super::{Icon, IconName};

/// A monochrome action or control icon.
#[derive(IntoElement, Clone)]
pub struct FunctionalIcon {
    icon: Icon,
}

impl FunctionalIcon {
    /// Creates a functional icon without validating its metadata family.
    pub fn new(name: IconName) -> Self {
        Self {
            icon: Icon::new(name).mono(),
        }
    }

    /// Applies an explicit monochrome tint.
    pub fn text_color(mut self, color: impl IntoColor<Hsla>) -> Self {
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

    /// Converts the wrapper back into the base icon type.
    pub fn into_icon(self) -> Icon {
        self.icon
    }
}

impl Styled for FunctionalIcon {
    fn style(&mut self) -> &mut StyleRefinement {
        self.icon.style()
    }

    fn text_color(self, color: impl IntoColor<Hsla>) -> Self {
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

/// An original-color icon used for product, platform, or database identity.
#[derive(IntoElement, Clone)]
pub struct BrandIcon {
    icon: Icon,
}

impl BrandIcon {
    /// Creates a color icon without validating its metadata family.
    pub fn new(name: IconName) -> Self {
        Self {
            icon: Icon::new(name).color(),
        }
    }

    /// Converts the wrapper back into the base icon type.
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
    /// Creates an object icon without validating its metadata family.
    pub fn new(name: IconName) -> Self {
        Self {
            icon: Icon::new(name).mono(),
        }
    }

    /// Applies an explicit monochrome tint.
    pub fn text_color(mut self, color: impl IntoColor<Hsla>) -> Self {
        self.icon = self.icon.text_color(color);
        self
    }

    /// Converts the wrapper back into the base icon type.
    pub fn into_icon(self) -> Icon {
        self.icon
    }
}

impl Styled for ObjectIcon {
    fn style(&mut self) -> &mut StyleRefinement {
        self.icon.style()
    }

    fn text_color(self, color: impl IntoColor<Hsla>) -> Self {
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

    use super::{BrandIcon, FunctionalIcon, ObjectIcon};
    use crate::{
        Icon, IconColorMode, IconName, IconSize, Sizable, Size, button::ButtonIconVariant,
    };

    #[test]
    fn wrappers_apply_the_required_color_mode_without_kind_validation() {
        let functional = FunctionalIcon::new(IconName::Plus).into_icon();
        let brand = BrandIcon::new(IconName::ServerLine).into_icon();
        let object = ObjectIcon::new(IconName::User).into_icon();

        assert_eq!(functional.color_mode, IconColorMode::Mono);
        assert_eq!(brand.color_mode, IconColorMode::Color);
        assert_eq!(object.color_mode, IconColorMode::Mono);
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
