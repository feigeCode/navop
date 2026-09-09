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
        let background = if self.muted_background {
            cx.theme().muted
        } else {
            cx.theme().background
        };

        self.base
            .h_flex()
            .w_full()
            .h(geometry::layout().status_bar)
            .flex_shrink_0()
            .gap(spacing.space_2)
            .px(spacing.space_3)
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(background)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
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
                        .text_color(self.presentation.color(cx))
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
}
