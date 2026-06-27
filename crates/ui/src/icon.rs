use crate::{ActiveTheme, Sizable, Size};
use gpui::{
    AnyElement, App, AppContext, Context, Entity, Hsla, ImageSource, IntoElement, ParentElement,
    Radians, Render, RenderOnce, SharedString, StyleRefinement, Styled, Svg, Transformation,
    Window, div, img, prelude::FluentBuilder as _, svg,
};
use gpui_component_macros::icon_named;
use std::path::PathBuf;

/// Types implementing this trait can automatically be converted to [`Icon`].
///
/// This allows you to implement a custom version of [`IconName`] that functions as a drop-in
/// replacement for other UI components.
pub trait IconNamed {
    /// Returns the embedded path of the icon.
    fn path(self) -> SharedString;
}

impl<T: IconNamed> From<T> for Icon {
    fn from(value: T) -> Self {
        Icon::build(value)
    }
}

icon_named!(IconName, "../assets/assets/icons");

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IconColorMode {
    /// Monochrome mode: uses SVG with text_color tinting (default)
    #[default]
    Mono,
    /// Color mode: renders the original SVG/image colors
    Color,
}

impl IconName {
    /// Return the icon as a Entity<Icon>
    pub fn view(self, cx: &mut App) -> Entity<Icon> {
        Icon::build(self).view(cx)
    }

    /// Return the icon in color mode.
    pub fn color(self) -> Icon {
        Icon::build(self).color()
    }

    /// Return the icon in monochrome mode.
    pub fn mono(self) -> Icon {
        Icon::build(self).mono()
    }
}

#[allow(non_upper_case_globals)]
impl IconName {
    pub const AI: Self = Self::Ai;
    pub const MongoDB: Self = Self::Mongodb;
    pub const MySQLColor: Self = Self::MysqlColor;
    pub const PostgreSQLColor: Self = Self::PostgresqlColor;
    pub const SQLiteColor: Self = Self::SqliteColor;
    pub const DuckDB: Self = Self::Duckdb;
    pub const MSSQLColor: Self = Self::MssqlColor;
    pub const ClickHouseColor: Self = Self::ClickhouseColor;
    pub const MySQLLineColor: Self = Self::MysqlLineColor;
    pub const PostgreSQLLineColor: Self = Self::PostgresqlLineColor;
    pub const SQLiteLineColor: Self = Self::SqliteLineColor;
    pub const MSSQLLineColor: Self = Self::MssqlLineColor;
    pub const ClickHouseLineColor: Self = Self::ClickhouseLineColor;
}

impl From<IconName> for AnyElement {
    fn from(val: IconName) -> Self {
        Icon::build(val).into_any_element()
    }
}

impl RenderOnce for IconName {
    fn render(self, _: &mut Window, _cx: &mut App) -> impl IntoElement {
        Icon::build(self)
    }
}

#[derive(IntoElement)]
pub struct Icon {
    base: Svg,
    style: StyleRefinement,
    path: SharedString,
    image_source: Option<ImageSource>,
    text_color: Option<Hsla>,
    size: Option<Size>,
    color_mode: IconColorMode,
    rotation: Option<Radians>,
}

impl Default for Icon {
    fn default() -> Self {
        Self {
            base: svg().flex_none().size_4(),
            style: StyleRefinement::default(),
            path: "".into(),
            image_source: None,
            text_color: None,
            size: None,
            color_mode: IconColorMode::default(),
            rotation: None,
        }
    }
}

impl Clone for Icon {
    fn clone(&self) -> Self {
        let mut this = Self::default().path(self.path.clone());
        this.style = self.style.clone();
        this.rotation = self.rotation;
        this.size = self.size;
        this.text_color = self.text_color;
        this.color_mode = self.color_mode;
        this.image_source = self.image_source.clone();
        this
    }
}

impl Icon {
    pub fn new(icon: impl Into<Icon>) -> Self {
        icon.into()
    }

    fn build(name: impl IconNamed) -> Self {
        Self::default().path(name.path())
    }

    /// Set the icon path of the Assets bundle
    ///
    /// For example: `icons/foo.svg`
    pub fn path(mut self, path: impl Into<SharedString>) -> Self {
        self.path = path.into();
        self.image_source = None;
        self
    }

    /// Set the icon source to a filesystem path.
    ///
    /// This is used for external assets that are not embedded in the application asset bundle.
    pub fn file_path(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        self.path = path.display().to_string().into();
        self.image_source = Some(path.into());
        self
    }

    /// Create a new view for the icon
    pub fn view(self, cx: &mut App) -> Entity<Icon> {
        cx.new(|_| self)
    }

    pub fn transform(mut self, transformation: gpui::Transformation) -> Self {
        self.base = self.base.with_transformation(transformation);
        self
    }

    pub fn empty() -> Self {
        Self::default()
    }

    /// Set the icon color mode.
    pub fn color_mode(mut self, mode: IconColorMode) -> Self {
        self.color_mode = mode;
        self
    }

    /// Set the icon to color mode.
    pub fn color(mut self) -> Self {
        self.color_mode = IconColorMode::Color;
        self
    }

    /// Set the icon to mono mode.
    pub fn mono(mut self) -> Self {
        self.color_mode = IconColorMode::Mono;
        self
    }

    /// Rotate the icon by the given angle
    pub fn rotate(mut self, radians: impl Into<Radians>) -> Self {
        self.base = self
            .base
            .with_transformation(Transformation::rotate(radians));
        self
    }
}

impl Styled for Icon {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }

    fn text_color(mut self, color: impl Into<Hsla>) -> Self {
        self.text_color = Some(color.into());
        self
    }
}

impl Sizable for Icon {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = Some(size.into());
        self
    }
}

impl RenderOnce for Icon {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let text_size = window.text_style().font_size.to_pixels(window.rem_size());
        let has_base_size = self.style.size.width.is_some() || self.style.size.height.is_some();

        match self.color_mode {
            IconColorMode::Mono => {
                let text_color = self.text_color.unwrap_or_else(|| window.text_style().color);
                let mut base = self.base;
                *base.style() = self.style;

                base.flex_shrink_0()
                    .text_color(text_color)
                    .when(!has_base_size, |this| this.size(text_size))
                    .when_some(self.size, |this, size| match size {
                        Size::Size(px) => this.size(px),
                        Size::XSmall => this.size_3(),
                        Size::Small => this.size_3p5(),
                        Size::Medium => this.size_4(),
                        Size::Large => this.size_6(),
                    })
                    .path(self.path)
                    .into_any_element()
            }
            IconColorMode::Color => {
                let size = self.size.unwrap_or(Size::Medium);
                let (w, h) = match size {
                    Size::Size(px) => (px, px),
                    Size::XSmall => (gpui::px(12.), gpui::px(12.)),
                    Size::Small => (gpui::px(14.), gpui::px(14.)),
                    Size::Medium => (gpui::px(16.), gpui::px(16.)),
                    Size::Large => (gpui::px(24.), gpui::px(24.)),
                };

                div()
                    .flex_shrink_0()
                    .w(w)
                    .h(h)
                    .child(
                        img(self
                            .image_source
                            .unwrap_or_else(|| self.path.clone().into()))
                        .size_full(),
                    )
                    .into_any_element()
            }
        }
    }
}

impl From<Icon> for AnyElement {
    fn from(val: Icon) -> Self {
        val.into_any_element()
    }
}

impl Render for Icon {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text_size = window.text_style().font_size.to_pixels(window.rem_size());
        let has_base_size = self.style.size.width.is_some() || self.style.size.height.is_some();

        match self.color_mode {
            IconColorMode::Mono => {
                let text_color = self.text_color.unwrap_or_else(|| cx.theme().foreground);
                let mut base = svg().flex_none();
                *base.style() = self.style.clone();

                base.flex_shrink_0()
                    .text_color(text_color)
                    .when(!has_base_size, |this| this.size(text_size))
                    .when_some(self.size, |this, size| match size {
                        Size::Size(px) => this.size(px),
                        Size::XSmall => this.size_3(),
                        Size::Small => this.size_3p5(),
                        Size::Medium => this.size_4(),
                        Size::Large => this.size_6(),
                    })
                    .path(self.path.clone())
                    .when_some(self.rotation, |this, rotation| {
                        this.with_transformation(Transformation::rotate(rotation))
                    })
                    .into_any_element()
            }
            IconColorMode::Color => {
                let size = self.size.unwrap_or(Size::Medium);
                let (w, h) = match size {
                    Size::Size(px) => (px, px),
                    Size::XSmall => (gpui::px(12.), gpui::px(12.)),
                    Size::Small => (gpui::px(14.), gpui::px(14.)),
                    Size::Medium => (gpui::px(16.), gpui::px(16.)),
                    Size::Large => (gpui::px(24.), gpui::px(24.)),
                };

                div()
                    .flex_shrink_0()
                    .w(w)
                    .h(h)
                    .child(
                        img(self
                            .image_source
                            .clone()
                            .unwrap_or_else(|| self.path.clone().into()))
                        .size_full(),
                    )
                    .into_any_element()
            }
        }
    }
}
