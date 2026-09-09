use crate::{IconSize, geometry};
use gpui::{
    AnyElement, App, IntoElement, ParentElement, Pixels, RenderOnce, SharedString, StyleRefinement,
    Styled, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, spinner::Spinner, v_flex,
};

/// Generic outer-content states. Business state machines remain owned by pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentStateKind {
    Empty,
    Loading,
    Error,
}

/// Shared empty, loading, and error presentation.
#[derive(IntoElement)]
pub struct ContentState {
    style: StyleRefinement,
    kind: ContentStateKind,
    title: SharedString,
    detail: Option<SharedString>,
    icon: Option<Icon>,
    action: Option<AnyElement>,
    compact: bool,
    fill: bool,
    visual_size: Option<IconSize>,
    detail_max_width: Option<Pixels>,
}

impl ContentState {
    pub fn empty(title: impl Into<SharedString>) -> Self {
        Self::new(ContentStateKind::Empty, title).icon(IconName::Inbox)
    }

    pub fn loading(title: impl Into<SharedString>) -> Self {
        Self::new(ContentStateKind::Loading, title)
    }

    pub fn error(title: impl Into<SharedString>) -> Self {
        Self::new(ContentStateKind::Error, title).icon(IconName::TriangleAlert)
    }

    fn new(kind: ContentStateKind, title: impl Into<SharedString>) -> Self {
        Self {
            style: StyleRefinement::default(),
            kind,
            title: title.into(),
            detail: None,
            icon: None,
            action: None,
            compact: false,
            fill: true,
            visual_size: None,
            detail_max_width: None,
        }
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }

    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// Applies the compact spacing and icon scale used by narrow side panels.
    pub fn narrow(mut self) -> Self {
        self.compact = true;
        self.visual_size = Some(IconSize::Default);
        self
    }

    /// Controls whether the state fills all available width and height.
    pub fn fill(mut self, fill: bool) -> Self {
        self.fill = fill;
        self
    }

    /// Renders the state at its intrinsic size instead of filling its parent.
    pub fn inline(self) -> Self {
        self.fill(false)
    }

    /// Overrides the loading spinner or state icon size.
    pub fn visual_size(mut self, size: IconSize) -> Self {
        self.visual_size = Some(size);
        self
    }

    /// Constrains supporting text without coupling it to workspace dimensions.
    pub fn detail_max_width(mut self, width: Pixels) -> Self {
        self.detail_max_width = Some(width);
        self
    }

    fn render_visual(
        kind: ContentStateKind,
        icon: Option<Icon>,
        visual_size: Option<IconSize>,
        cx: &mut App,
    ) -> AnyElement {
        match kind {
            ContentStateKind::Loading => Spinner::new()
                .with_size(visual_size.unwrap_or(IconSize::Medium))
                .color(cx.theme().muted_foreground)
                .into_any_element(),
            ContentStateKind::Empty => icon
                .unwrap_or_else(|| Icon::new(IconName::Inbox))
                .with_size(visual_size.unwrap_or(IconSize::Large))
                .text_color(cx.theme().muted_foreground)
                .into_any_element(),
            ContentStateKind::Error => icon
                .unwrap_or_else(|| Icon::new(IconName::TriangleAlert))
                .with_size(visual_size.unwrap_or(IconSize::Large))
                .text_color(cx.theme().danger)
                .into_any_element(),
        }
    }
}

impl Styled for ContentState {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for ContentState {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            style,
            kind,
            title,
            detail,
            icon,
            action,
            compact,
            fill,
            visual_size,
            detail_max_width,
        } = self;
        let visual = Self::render_visual(kind, icon, visual_size, cx);
        let spacing = geometry::spacing();
        let title_color = match kind {
            ContentStateKind::Error => cx.theme().danger,
            ContentStateKind::Empty | ContentStateKind::Loading => cx.theme().foreground,
        };

        v_flex()
            .min_w_0()
            .when(fill, |this| this.size_full())
            .items_center()
            .justify_center()
            .text_center()
            .gap(if compact {
                spacing.space_2
            } else {
                spacing.space_3
            })
            .p(if compact {
                spacing.space_2
            } else {
                spacing.space_6
            })
            .refine_style(&style)
            .child(visual)
            .child(
                div()
                    .min_w_0()
                    .max_w_full()
                    .text_sm()
                    .text_color(title_color)
                    .child(title),
            )
            .when_some(detail, |this, detail| {
                this.child(
                    div()
                        .min_w_0()
                        .max_w_full()
                        .when_some(detail_max_width, |this, width| this.max_w(width))
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(detail),
                )
            })
            .when_some(action, |this, action| {
                this.child(
                    div()
                        .min_w_0()
                        .max_w_full()
                        .mt(spacing.space_1)
                        .child(action),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_preserve_state_semantics() {
        assert_eq!(ContentState::empty("No data").kind, ContentStateKind::Empty);
        assert_eq!(
            ContentState::loading("Loading").kind,
            ContentStateKind::Loading
        );
        assert_eq!(ContentState::error("Failed").kind, ContentStateKind::Error);
    }

    #[test]
    fn narrow_and_inline_presets_are_composable() {
        let state = ContentState::empty("No data").narrow().inline();

        assert!(state.compact);
        assert!(!state.fill);
        assert_eq!(state.visual_size, Some(IconSize::Default));
    }

    #[test]
    fn explicit_visual_and_detail_constraints_are_preserved() {
        let state = ContentState::loading("Loading")
            .visual_size(IconSize::Small)
            .detail_max_width(gpui::px(280.));

        assert_eq!(state.visual_size, Some(IconSize::Small));
        assert_eq!(state.detail_max_width, Some(gpui::px(280.)));
    }
}
