use crate::IconSize;
use gpui::{
    App, ClickEvent, ElementId, Hsla, InteractiveElement, Interactivity, IntoElement, RenderOnce,
    SharedString, StatefulInteractiveElement, StyleRefinement, Styled, Window,
};
use gpui_component::{
    Disableable, Icon, Selectable, Sizable, Size,
    button::{Button, ButtonRounded, ButtonVariant, ButtonVariants},
    menu::DropdownMenu,
};
use palette::IntoColor;

/// Semantic sizing presets for icon-only actions.
///
/// The role controls both the interactive hit target and the visual glyph size
/// while keeping those dimensions independent. Builder calls made after
/// [`IconButton::role`] can still override either dimension explicitly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IconButtonRole {
    /// Dense 28px controls used in compact trees and narrow panel headers.
    Compact,
    /// Standard 32px action with a 16px glyph.
    #[default]
    Standard,
    /// Standard command-bar action. Kept separate so toolbar density can evolve
    /// without changing general-purpose icon buttons.
    Toolbar,
    /// Prominent 40px navigation action with a 20px glyph.
    Navigation,
}

impl IconButtonRole {
    pub const fn hit_size(self) -> Size {
        match self {
            Self::Compact => Size::Small,
            Self::Standard | Self::Toolbar => Size::Medium,
            Self::Navigation => Size::Large,
        }
    }

    pub const fn glyph_size(self) -> IconSize {
        match self {
            Self::Compact => IconSize::Small,
            Self::Standard | Self::Toolbar => IconSize::Default,
            Self::Navigation => IconSize::Medium,
        }
    }
}

/// An icon-only button with independent hit-target and glyph sizing.
#[derive(IntoElement)]
pub struct IconButton {
    button: Button,
    hit_size: Size,
    glyph_size: IconSize,
}

impl IconButton {
    /// Creates a neutral icon-only action with a 32px hit target and 16px glyph.
    pub fn new(id: impl Into<ElementId>, icon: impl Into<Icon>) -> Self {
        let role = IconButtonRole::default();
        let hit_size = role.hit_size();
        let glyph_size = role.glyph_size();
        let icon = icon.into();
        let button = Button::new(id)
            .icon(icon)
            .ghost()
            .with_size(hit_size)
            .glyph_size(glyph_size.into());

        Self {
            button,
            hit_size,
            glyph_size,
        }
    }

    /// Applies a semantic hit-target and glyph-size pair.
    ///
    /// As with other builder methods, later calls win. Use `.role(...)` first,
    /// then `.hit_size(...)` or `.glyph_size(...)` for a deliberate exception.
    pub fn role(self, role: IconButtonRole) -> Self {
        self.hit_size(role.hit_size()).glyph_size(role.glyph_size())
    }

    /// Sets the interactive hit-target size without changing the glyph.
    pub fn hit_size(mut self, size: impl Into<Size>) -> Self {
        let size = size.into();
        self.hit_size = size;
        self.button = self.button.with_size(size);
        self
    }

    /// Sets the glyph size without changing the interactive hit target.
    pub fn glyph_size(mut self, size: IconSize) -> Self {
        self.glyph_size = size;
        self.button = self.button.glyph_size(size.into());
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.button = self.button.tooltip(tooltip);
        self
    }

    pub fn accessible_label(mut self, label: impl Into<SharedString>) -> Self {
        self.button = self.button.accessibility_label(label);
        self
    }

    /// Applies a semantic tint to the glyph and button text state.
    pub fn text_color(mut self, color: impl IntoColor<Hsla>) -> Self {
        self.button = self.button.text_color(color);
        self
    }

    pub fn tooltip_with_action(
        mut self,
        tooltip: impl Into<SharedString>,
        action: &dyn gpui::Action,
        context: Option<&str>,
    ) -> Self {
        self.button = self.button.tooltip_with_action(tooltip, action, context);
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.button = self.button.loading(loading);
        self
    }

    pub fn loading_icon(mut self, icon: impl Into<Icon>) -> Self {
        self.button = self.button.loading_icon(icon);
        self
    }

    pub fn compact(mut self) -> Self {
        self.button = self.button.compact();
        self
    }

    pub fn outline(mut self) -> Self {
        self.button = self.button.outline();
        self
    }

    pub fn rounded(mut self, rounded: impl Into<ButtonRounded>) -> Self {
        self.button = self.button.rounded(rounded);
        self
    }

    pub fn tab_index(mut self, tab_index: isize) -> Self {
        self.button = self.button.tab_index(tab_index);
        self
    }

    pub fn tab_stop(mut self, tab_stop: bool) -> Self {
        self.button = self.button.tab_stop(tab_stop);
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.button = self.button.on_click(handler);
        self
    }

    pub fn on_hover(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.button = self.button.on_hover(handler);
        self
    }
}

impl Disableable for IconButton {
    fn disabled(mut self, disabled: bool) -> Self {
        self.button = self.button.disabled(disabled);
        self
    }
}

impl Selectable for IconButton {
    fn selected(mut self, selected: bool) -> Self {
        self.button = self.button.selected(selected);
        self
    }

    fn is_selected(&self) -> bool {
        self.button.is_selected()
    }
}

impl Sizable for IconButton {
    fn with_size(self, size: impl Into<Size>) -> Self {
        self.hit_size(size)
    }
}

impl ButtonVariants for IconButton {
    fn with_variant(mut self, variant: ButtonVariant) -> Self {
        self.button = self.button.with_variant(variant);
        self
    }
}

impl Styled for IconButton {
    fn style(&mut self) -> &mut StyleRefinement {
        self.button.style()
    }
}

impl InteractiveElement for IconButton {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.button.interactivity()
    }
}

impl StatefulInteractiveElement for IconButton {}

impl DropdownMenu for IconButton {}

impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.button
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;
    use gpui_component::{IconName, Theme, button::ButtonCustomVariant};

    #[test]
    fn icon_button_roles_keep_hit_targets_and_glyphs_independent() {
        assert_eq!(IconButtonRole::Compact.hit_size(), Size::Small);
        assert_eq!(IconButtonRole::Compact.glyph_size(), IconSize::Small);
        assert_eq!(IconButtonRole::Standard.hit_size(), Size::Medium);
        assert_eq!(IconButtonRole::Standard.glyph_size(), IconSize::Default);
        assert_eq!(IconButtonRole::Toolbar.hit_size(), Size::Medium);
        assert_eq!(IconButtonRole::Toolbar.glyph_size(), IconSize::Default);
        assert_eq!(IconButtonRole::Navigation.hit_size(), Size::Large);
        assert_eq!(IconButtonRole::Navigation.glyph_size(), IconSize::Medium);
    }

    #[gpui::test]
    fn icon_button_separates_hit_and_glyph_sizes(_cx: &mut gpui::TestAppContext) {
        let button = IconButton::new("add", IconName::Plus)
            .hit_size(px(36.))
            .glyph_size(IconSize::Medium);

        assert_eq!(button.hit_size, Size::Size(px(36.)));
        assert_eq!(button.glyph_size, IconSize::Medium);
    }

    #[test]
    fn explicit_sizes_can_refine_a_semantic_role() {
        let button = IconButton::new("navigate", IconName::ArrowRight)
            .role(IconButtonRole::Navigation)
            .hit_size(px(44.))
            .glyph_size(IconSize::Large);

        assert_eq!(button.hit_size, Size::Size(px(44.)));
        assert_eq!(button.glyph_size, IconSize::Large);
    }

    #[test]
    fn sizable_only_changes_the_hit_target() {
        let button = IconButton::new("compact", IconName::Minus).with_size(Size::XSmall);

        assert_eq!(button.hit_size, Size::XSmall);
        assert_eq!(button.glyph_size, IconSize::Default);
    }

    #[gpui::test]
    fn icon_button_forwards_button_behaviors(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Theme::default());
            let button = IconButton::new("settings", IconName::Settings)
                .tooltip("Settings")
                .accessible_label("Application settings")
                .compact()
                .selected(true)
                .disabled(false)
                .loading(false)
                .tab_stop(false)
                .custom(ButtonCustomVariant::new(cx))
                .on_click(|_, _, _| {});

            assert!(button.is_selected());
        });
    }
}
