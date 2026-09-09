use crate::geometry;
use gpui::{
    AnyElement, App, ElementId, Hsla, InteractiveElement, Interactivity, IntoElement,
    ParentElement, Pixels, RenderOnce, SharedString, Stateful, StatefulInteractiveElement as _,
    StyleRefinement, Styled, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::{ActiveTheme as _, StyledExt as _, h_flex, tooltip::Tooltip};

/// Semantic height roles for panel-level headers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PanelHeaderVariant {
    #[default]
    Panel,
    Embedded,
    Dock,
    Sidebar,
    Toolbar,
}

impl PanelHeaderVariant {
    fn height(self) -> Pixels {
        let layout = geometry::layout();
        match self {
            Self::Panel => layout.panel_header,
            Self::Embedded | Self::Sidebar => layout.embedded_panel_header,
            Self::Dock => layout.dock_panel_header,
            Self::Toolbar => layout.command_bar,
        }
    }
}

/// Shared visual shell for panel titles and their leading/trailing actions.
#[derive(IntoElement)]
pub struct PanelHeader {
    id: ElementId,
    base: Stateful<gpui::Div>,
    style: StyleRefinement,
    variant: PanelHeaderVariant,
    height: Option<Pixels>,
    background: Option<Hsla>,
    border_color: Option<Hsla>,
    border_bottom: bool,
    border_left: bool,
    border_right: bool,
    horizontal_padding: Option<Pixels>,
    leading: Option<AnyElement>,
    title: Option<AnyElement>,
    title_text: Option<SharedString>,
    trailing: Option<AnyElement>,
}

impl PanelHeader {
    pub fn new(id: impl Into<ElementId>) -> Self {
        let id = id.into();
        Self {
            id: id.clone(),
            base: div().id(id),
            style: StyleRefinement::default(),
            variant: PanelHeaderVariant::default(),
            height: None,
            background: None,
            border_color: None,
            border_bottom: true,
            border_left: false,
            border_right: false,
            horizontal_padding: None,
            leading: None,
            title: None,
            title_text: None,
            trailing: None,
        }
    }

    pub fn variant(mut self, variant: PanelHeaderVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn height(mut self, height: Pixels) -> Self {
        self.height = Some(height);
        self
    }

    pub fn background(mut self, background: Hsla) -> Self {
        self.background = Some(background);
        self
    }

    pub fn border_color(mut self, border_color: Hsla) -> Self {
        self.border_color = Some(border_color);
        self
    }

    /// Controls whether the header renders its bottom separator.
    pub fn border_bottom(mut self, enabled: bool) -> Self {
        self.border_bottom = enabled;
        self
    }

    /// Controls whether the header renders a left separator.
    pub fn border_left(mut self, enabled: bool) -> Self {
        self.border_left = enabled;
        self
    }

    /// Controls whether the header renders a right separator.
    pub fn border_right(mut self, enabled: bool) -> Self {
        self.border_right = enabled;
        self
    }

    /// Overrides the semantic horizontal padding for this header.
    pub fn horizontal_padding(mut self, padding: Pixels) -> Self {
        self.horizontal_padding = Some(padding);
        self
    }

    pub fn leading(mut self, leading: impl IntoElement) -> Self {
        self.leading = Some(leading.into_any_element());
        self
    }

    pub fn title(mut self, title: impl IntoElement) -> Self {
        self.title = Some(title.into_any_element());
        self.title_text = None;
        self
    }

    /// Sets a single-line panel title with truncation and a full-text tooltip.
    pub fn title_text(mut self, title: impl Into<SharedString>) -> Self {
        self.title_text = Some(title.into());
        self.title = None;
        self
    }

    pub fn trailing(mut self, trailing: impl IntoElement) -> Self {
        self.trailing = Some(trailing.into_any_element());
        self
    }
}

impl Styled for PanelHeader {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl InteractiveElement for PanelHeader {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for PanelHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let height = self.height.unwrap_or_else(|| self.variant.height());
        let background = self.background.unwrap_or(cx.theme().muted);
        let border_color = self.border_color.unwrap_or(cx.theme().border);
        let spacing = geometry::spacing();
        let horizontal_padding = self.horizontal_padding.unwrap_or(spacing.space_3);
        let title = if let Some(title) = self.title_text {
            let tooltip = title.clone();
            h_flex()
                .id((self.id.clone(), "title"))
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                .child(title)
                .into_any_element()
        } else {
            h_flex()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .gap(spacing.space_2)
                .when_some(self.title, |this, title| this.child(title))
                .into_any_element()
        };

        self.base
            .h_flex()
            .w_full()
            .h(height)
            .flex_shrink_0()
            .gap(spacing.space_2)
            .px(horizontal_padding)
            .when(self.border_bottom, |this| this.border_b_1())
            .when(self.border_left, |this| this.border_l_1())
            .when(self.border_right, |this| this.border_r_1())
            .border_color(border_color)
            .bg(background)
            .text_sm()
            .text_color(cx.theme().foreground)
            .refine_style(&self.style)
            .when_some(self.leading, |this, leading| {
                this.child(h_flex().flex_none().child(leading))
            })
            .child(title)
            .when_some(self.trailing, |this, trailing| {
                this.child(h_flex().flex_none().gap(spacing.space_1).child(trailing))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_resolve_to_geometry_roles() {
        let layout = geometry::layout();

        assert_eq!(PanelHeaderVariant::Panel.height(), layout.panel_header);
        assert_eq!(
            PanelHeaderVariant::Embedded.height(),
            layout.embedded_panel_header
        );
        assert_eq!(
            PanelHeaderVariant::Sidebar.height(),
            layout.embedded_panel_header
        );
        assert_eq!(PanelHeaderVariant::Dock.height(), layout.dock_panel_header);
        assert_eq!(PanelHeaderVariant::Toolbar.height(), layout.command_bar);
    }

    #[test]
    fn new_preserves_default_visual_options() {
        let header = PanelHeader::new("test");

        assert_eq!(header.variant, PanelHeaderVariant::Panel);
        assert_eq!(header.height, None);
        assert_eq!(header.background, None);
        assert_eq!(header.border_color, None);
        assert!(header.border_bottom);
        assert!(!header.border_left);
        assert!(!header.border_right);
        assert_eq!(header.horizontal_padding, None);
        assert_eq!(header.title_text, None);
    }

    #[test]
    fn builders_override_border_and_padding_options() {
        let header = PanelHeader::new("test")
            .border_bottom(false)
            .border_left(true)
            .border_right(true)
            .horizontal_padding(gpui::px(8.0));

        assert!(!header.border_bottom);
        assert!(header.border_left);
        assert!(header.border_right);
        assert_eq!(header.horizontal_padding, Some(gpui::px(8.0)));
    }

    #[test]
    fn title_builders_use_last_call_wins_semantics() {
        let text_last = PanelHeader::new("text-last")
            .title(gpui::div().child("Rich title"))
            .title_text("Plain title");
        assert_eq!(text_last.title_text, Some("Plain title".into()));
        assert!(text_last.title.is_none());

        let rich_last = PanelHeader::new("rich-last")
            .title_text("Plain title")
            .title(gpui::div().child("Rich title"));
        assert!(rich_last.title_text.is_none());
        assert!(rich_last.title.is_some());
    }
}
