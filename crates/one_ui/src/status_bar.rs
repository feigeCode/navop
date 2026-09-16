use crate::geometry;
use gpui::{
    AnyElement, App, ElementId, Hsla, InteractiveElement, Interactivity, IntoElement,
    ParentElement, RenderOnce, Stateful, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::{ActiveTheme as _, StyledExt as _, h_flex};

/// Semantic status meaning. Colors are reserved for actual state feedback.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StatusPresentation {
    #[default]
    Neutral,
    Progress,
    Success,
    Warning,
    Error,
}

impl StatusPresentation {
    fn color(self, cx: &App) -> Hsla {
        match self {
            Self::Neutral => cx.theme().muted_foreground,
            Self::Progress => cx.theme().info,
            Self::Success => cx.theme().success,
            Self::Warning => cx.theme().warning,
            Self::Error => cx.theme().danger,
        }
    }

    /// The colour for this presentation, with a host palette winning over the
    /// theme's semantic colour wherever it names one.
    fn resolved_color(self, cx: &App, colors: Option<&StatusBarColors>) -> Hsla {
        let from_palette = colors.and_then(|colors| match self {
            Self::Neutral => colors.muted_foreground,
            Self::Progress => colors.info,
            Self::Success => colors.success,
            Self::Warning => colors.warning,
            Self::Error => colors.danger,
        });

        from_palette.unwrap_or_else(|| self.color(cx))
    }
}

/// Colours a host projects onto a status bar.
///
/// Every field is optional and only the ones set take effect; the rest keep
/// coming from the active theme. A workspace whose surfaces follow a palette
/// other than the application theme's — a terminal theme, say — paints its bar
/// with this, so the bar reads as part of the surface above it rather than as
/// a strip of the application theme's chrome.
#[derive(Clone, Debug, Default)]
pub struct StatusBarColors {
    pub background: Option<Hsla>,
    pub border: Option<Hsla>,
    pub muted_foreground: Option<Hsla>,
    pub info: Option<Hsla>,
    pub success: Option<Hsla>,
    pub warning: Option<Hsla>,
    pub danger: Option<Hsla>,
}

/// Shared status-bar shell with leading, center, and trailing slots.
#[derive(IntoElement)]
pub struct StatusBar {
    base: Stateful<gpui::Div>,
    style: StyleRefinement,
    presentation: StatusPresentation,
    leading: Option<AnyElement>,
    center: Option<AnyElement>,
    status: Option<AnyElement>,
    trailing: Option<AnyElement>,
    muted_background: bool,
    colors: Option<StatusBarColors>,
}

impl StatusBar {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id),
            style: StyleRefinement::default(),
            presentation: StatusPresentation::default(),
            leading: None,
            center: None,
            status: None,
            trailing: None,
            muted_background: false,
            colors: None,
        }
    }

    pub fn presentation(mut self, presentation: StatusPresentation) -> Self {
        self.presentation = presentation;
        self
    }

    pub fn leading(mut self, leading: impl IntoElement) -> Self {
        self.leading = Some(leading.into_any_element());
        self
    }

    pub fn center(mut self, center: impl IntoElement) -> Self {
        self.center = Some(center.into_any_element());
        self
    }

    pub fn status(mut self, status: impl IntoElement) -> Self {
        self.status = Some(status.into_any_element());
        self
    }

    pub fn trailing(mut self, trailing: impl IntoElement) -> Self {
        self.trailing = Some(trailing.into_any_element());
        self
    }

    pub fn muted_background(mut self) -> Self {
        self.muted_background = true;
        self
    }

    /// Paint this bar with a palette of the caller's choosing.
    ///
    /// Only the fields set here win over the theme's colours.
    pub fn colors(mut self, colors: StatusBarColors) -> Self {
        self.colors = Some(colors);
        self
    }
}

impl Styled for StatusBar {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl InteractiveElement for StatusBar {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for StatusBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let spacing = geometry::spacing();
        let palette = self.colors.as_ref();
        let theme_background = if self.muted_background {
            cx.theme().muted
        } else {
            cx.theme().background
        };
        let background = palette
            .and_then(|colors| colors.background)
            .unwrap_or(theme_background);
        let border = palette
            .and_then(|colors| colors.border)
            .unwrap_or_else(|| cx.theme().border);
        let foreground = palette
            .and_then(|colors| colors.muted_foreground)
            .unwrap_or_else(|| cx.theme().muted_foreground);

        self.base
            .h_flex()
            .w_full()
            .h(geometry::layout().status_bar)
            .flex_shrink_0()
            .gap(spacing.space_2)
            .px(spacing.space_3)
            .border_t_1()
            .border_color(border)
            .bg(background)
            .text_xs()
            .text_color(foreground)
            .refine_style(&self.style)
            .when_some(self.leading, |this, leading| {
                this.child(h_flex().flex_none().child(leading))
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .justify_center()
                    .when_some(self.center, |this, center| this.child(center)),
            )
            .when_some(self.status, |this, status| {
                this.child(
                    h_flex()
                        .flex_none()
                        .text_color(self.presentation.resolved_color(cx, palette))
                        .child(status),
                )
            })
            .when_some(self.trailing, |this, trailing| {
                this.child(h_flex().flex_none().child(trailing))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn presentations_use_semantic_theme_colors(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(gpui_component::Theme::default());
            assert_eq!(StatusPresentation::Progress.color(cx), cx.theme().info);
            assert_eq!(StatusPresentation::Success.color(cx), cx.theme().success);
            assert_eq!(StatusPresentation::Warning.color(cx), cx.theme().warning);
            assert_eq!(StatusPresentation::Error.color(cx), cx.theme().danger);
        });
    }

    #[gpui::test]
    fn a_host_palette_wins_where_it_is_set(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(gpui_component::Theme::default());
            let colors = StatusBarColors {
                warning: Some(gpui::rgb(0xffaa00).into()),
                muted_foreground: Some(gpui::rgb(0x7788aa).into()),
                ..Default::default()
            };

            assert_eq!(
                StatusPresentation::Warning.resolved_color(cx, Some(&colors)),
                gpui::rgb(0xffaa00).into()
            );
            assert_eq!(
                StatusPresentation::Neutral.resolved_color(cx, Some(&colors)),
                gpui::rgb(0x7788aa).into()
            );
            // A presentation the palette does not name keeps its semantic
            // colour, so naming one state does not mute the others.
            assert_eq!(
                StatusPresentation::Error.resolved_color(cx, Some(&colors)),
                cx.theme().danger
            );
            // Without a palette the theme answers, exactly as before.
            assert_eq!(
                StatusPresentation::Warning.resolved_color(cx, None),
                cx.theme().warning
            );
        });
    }
}
