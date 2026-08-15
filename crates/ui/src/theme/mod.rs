use crate::{
    highlighter::HighlightTheme, list::ListSettings, notification::NotificationSettings,
    scroll::ScrollbarShow, sheet::SheetSettings,
};
use gpui::{App, Global, Hsla, Pixels, SharedString, Window, WindowAppearance, px};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    ops::{Deref, DerefMut},
    rc::Rc,
    sync::Arc,
};

mod color;
mod geometry;
mod registry;
mod schema;
mod theme_color;

pub use color::*;
pub use geometry::*;
pub use registry::*;
pub use schema::*;
pub use theme_color::*;

pub fn init(cx: &mut App) {
    registry::init(cx);

    // Ensure theme is loaded directly on startup for WASM compatibility
    Theme::change(ThemeMode::Light, None, cx);
    Theme::sync_scrollbar_appearance(cx);
}

pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    #[inline(always)]
    fn theme(&self) -> &Theme {
        Theme::global(self)
    }
}

/// The global theme configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Theme {
    pub colors: ThemeColor,
    pub highlight_theme: Arc<HighlightTheme>,
    pub light_theme: Rc<ThemeConfig>,
    pub dark_theme: Rc<ThemeConfig>,
    /// Shared geometry tokens for application shell, components, and pages.
    #[serde(default)]
    pub geometry: ThemeGeometry,

    pub mode: ThemeMode,
    /// The font family for the application, default is `.SystemUIFont`.
    pub font_family: SharedString,
    /// The base font size for the application, default is 16px.
    pub font_size: Pixels,
    /// The monospace font family for the application.
    ///
    /// Defaults to:
    ///
    /// - macOS: `Menlo`
    /// - Windows: `Consolas`
    /// - Linux: `DejaVu Sans Mono`
    pub mono_font_family: SharedString,
    /// The monospace font size for the application, default is 13px.
    pub mono_font_size: Pixels,
    /// Radius for the general elements.
    pub radius: Pixels,
    /// Radius for the large elements, e.g.: Dialog, Notification border radius.
    pub radius_lg: Pixels,
    pub shadow: bool,
    #[schemars(schema_with = "gpui::hsla_schemar")]
    pub transparent: Hsla,
    /// Show the scrollbar mode, default: Scrolling
    pub scrollbar_show: ScrollbarShow,
    /// The notification setting.
    #[serde(skip)]
    pub notification: NotificationSettings,
    /// Tile grid size, default is 4px.
    pub tile_grid_size: Pixels,
    /// The shadow of the tile panel.
    pub tile_shadow: bool,
    /// The border radius of the tile panel, default is 0px.
    pub tile_radius: Pixels,
    /// The list settings.
    pub list: ListSettings,
    /// The sheet settings.
    pub sheet: SheetSettings,
}

impl Default for Theme {
    fn default() -> Self {
        Self::from(&ThemeColor::default())
    }
}

impl Deref for Theme {
    type Target = ThemeColor;

    fn deref(&self) -> &Self::Target {
        &self.colors
    }
}

impl DerefMut for Theme {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.colors
    }
}

impl Global for Theme {}

impl Theme {
    /// Returns the global theme reference
    #[inline(always)]
    pub fn global(cx: &App) -> &Theme {
        cx.global::<Theme>()
    }

    /// Returns the global theme mutable reference
    #[inline(always)]
    pub fn global_mut(cx: &mut App) -> &mut Theme {
        cx.global_mut::<Theme>()
    }

    /// Returns true if the theme is dark.
    #[inline(always)]
    pub fn is_dark(&self) -> bool {
        self.mode.is_dark()
    }

    /// Returns the current theme name.
    pub fn theme_name(&self) -> &SharedString {
        if self.is_dark() {
            &self.dark_theme.name
        } else {
            &self.light_theme.name
        }
    }

    /// Sync the theme with the system appearance
    pub fn sync_system_appearance(window: Option<&mut Window>, cx: &mut App) {
        // Better use window.appearance() for avoid error on Linux.
        // https://github.com/longbridge/gpui-component/issues/104
        let appearance = window
            .as_ref()
            .map(|window| window.appearance())
            .unwrap_or_else(|| cx.window_appearance());

        Self::change(appearance, window, cx);
    }

    /// Sync the Scrollbar showing behavior with the system
    pub fn sync_scrollbar_appearance(cx: &mut App) {
        Theme::global_mut(cx).scrollbar_show = if cx.should_auto_hide_scrollbars() {
            ScrollbarShow::Scrolling
        } else {
            ScrollbarShow::Hover
        };
    }

    /// Change the theme mode.
    pub fn change(mode: impl Into<ThemeMode>, window: Option<&mut Window>, cx: &mut App) {
        let mode = mode.into();
        if !cx.has_global::<Theme>() {
            let mut theme = Theme::default();
            theme.light_theme = ThemeRegistry::global(cx).default_light_theme().clone();
            theme.dark_theme = ThemeRegistry::global(cx).default_dark_theme().clone();
            cx.set_global(theme);
        }

        let theme = cx.global_mut::<Theme>();
        theme.mode = mode;
        if mode.is_dark() {
            theme.apply_config(&theme.dark_theme.clone());
        } else {
            theme.apply_config(&theme.light_theme.clone());
        }

        if let Some(window) = window {
            window.refresh();
        }
    }

    /// Get the input background color.
    ///
    /// For dark, use a transparent color mixed with the input border: `cx.theme().input`,
    /// otherwise use the `cx.theme().background` color.
    #[inline]
    pub fn input_background(&self) -> Hsla {
        if self.is_dark() {
            self.input.mix_oklab(self.transparent, 0.3)
        } else {
            self.background
        }
    }

    /// Get the editor background color, if not set, use the input background color.
    #[inline]
    pub(crate) fn editor_background(&self) -> Hsla {
        self.highlight_theme
            .style
            .editor_background
            .unwrap_or_else(|| self.input_background())
    }
}

impl From<&ThemeColor> for Theme {
    fn from(colors: &ThemeColor) -> Self {
        Theme {
            mode: ThemeMode::default(),
            transparent: gpui::transparent_black(),
            font_family: ".SystemUIFont".into(),
            font_size: px(16.),
            mono_font_family: if cfg!(target_os = "macos") {
                // https://en.wikipedia.org/wiki/Menlo_(typeface)
                "Menlo".into()
            } else if cfg!(target_os = "windows") {
                "Consolas".into()
            } else {
                "DejaVu Sans Mono".into()
            },
            mono_font_size: px(13.),
            radius: px(6.),
            radius_lg: px(8.),
            shadow: true,
            scrollbar_show: ScrollbarShow::default(),
            notification: NotificationSettings::default(),
            tile_grid_size: px(8.),
            tile_shadow: true,
            tile_radius: px(0.),
            list: ListSettings::default(),
            colors: *colors,
            light_theme: Rc::new(ThemeConfig::default()),
            dark_theme: Rc::new(ThemeConfig::default()),
            highlight_theme: HighlightTheme::default_light(),
            geometry: ThemeGeometry::default(),
            sheet: SheetSettings::default(),
        }
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    PartialOrd,
    Eq,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    Light,
    Dark,
}

impl ThemeMode {
    #[inline(always)]
    pub fn is_dark(&self) -> bool {
        matches!(self, Self::Dark)
    }

    /// Return lower_case theme name: `light`, `dark`.
    pub fn name(&self) -> &'static str {
        match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }
}

impl From<WindowAppearance> for ThemeMode {
    fn from(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::Dark,
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::Light,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_uses_default_geometry() {
        assert_eq!(Theme::default().geometry, ThemeGeometry::default());
    }

    #[test]
    fn theme_geometry_survives_serialization_round_trip() {
        let mut theme = Theme::default();
        theme.geometry.layout.global_rail = px(56.);

        let encoded = serde_json::to_value(&theme).expect("theme should serialize");
        let decoded: Theme =
            serde_json::from_value(encoded).expect("serialized theme should deserialize");

        assert_eq!(decoded.geometry.layout.global_rail, px(56.));
    }

    #[test]
    fn historic_theme_without_geometry_uses_default_tokens() {
        let mut encoded = serde_json::to_value(Theme::default()).expect("theme should serialize");
        encoded
            .as_object_mut()
            .expect("theme should serialize as an object")
            .remove("geometry");

        let decoded: Theme =
            serde_json::from_value(encoded).expect("historic theme should deserialize");

        assert_eq!(decoded.geometry, ThemeGeometry::default());
    }
}
