use std::rc::Rc;

use crate::{
    ActiveTheme, Colorize as _, Disableable, FocusableExt as _, Icon, IconName, IconSize,
    Selectable, Sizable, Size, StyleSized, StyledExt,
    button::ButtonIcon,
    h_flex,
    tooltip::{ManagedTooltipExt as _, Tooltip},
};
use gpui::{
    AnyElement, App, ClickEvent, Corners, Div, Edges, ElementId, Hsla, InteractiveElement,
    Interactivity, IntoElement, MouseButton, ParentElement, Pixels, RenderOnce, SharedString,
    Stateful, StatefulInteractiveElement, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder as _, px, relative, transparent_white,
};

#[derive(Default, Clone, Copy)]
pub enum ButtonRounded {
    None,
    Small,
    #[default]
    Medium,
    Large,
    Size(Pixels),
}

impl From<Pixels> for ButtonRounded {
    fn from(px: Pixels) -> Self {
        ButtonRounded::Size(px)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ButtonCustomVariant {
    color: Hsla,
    foreground: Hsla,
    shadow: bool,
    hover: Hsla,
    active: Hsla,
}

pub trait ButtonVariants: Sized {
    fn with_variant(self, variant: ButtonVariant) -> Self;

    /// With the primary style for the Button.
    fn primary(self) -> Self {
        self.with_variant(ButtonVariant::Primary)
    }

    /// With the secondary style for the Button.
    fn secondary(self) -> Self {
        self.with_variant(ButtonVariant::Secondary)
    }

    /// With the danger style for the Button.
    fn danger(self) -> Self {
        self.with_variant(ButtonVariant::Danger)
    }

    /// With the warning style for the Button.
    fn warning(self) -> Self {
        self.with_variant(ButtonVariant::Warning)
    }

    /// With the success style for the Button.
    fn success(self) -> Self {
        self.with_variant(ButtonVariant::Success)
    }

    /// With the info style for the Button.
    fn info(self) -> Self {
        self.with_variant(ButtonVariant::Info)
    }

    /// With the ghost style for the Button.
    fn ghost(self) -> Self {
        self.with_variant(ButtonVariant::Ghost)
    }

    /// With the link style for the Button.
    fn link(self) -> Self {
        self.with_variant(ButtonVariant::Link)
    }

    /// With the text style for the Button, it will no padding look like a normal text.
    fn text(self) -> Self {
        self.with_variant(ButtonVariant::Text)
    }

    /// With the custom style for the Button.
    fn custom(self, style: ButtonCustomVariant) -> Self {
        self.with_variant(ButtonVariant::Custom(style))
    }
}

impl ButtonCustomVariant {
    pub fn new(cx: &App) -> Self {
        Self {
            color: cx.theme().transparent,
            foreground: cx.theme().foreground,
            hover: cx.theme().transparent,
            active: cx.theme().transparent,
            shadow: false,
        }
    }

    /// Set background color, default is transparent.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = color;
        self
    }

    /// Set foreground color, default is theme foreground.
    pub fn foreground(mut self, color: Hsla) -> Self {
        self.foreground = color;
        self
    }

    /// Set hover background color, default is transparent.
    pub fn hover(mut self, color: Hsla) -> Self {
        self.hover = color;
        self
    }

    /// Set active background color, default is transparent.
    pub fn active(mut self, color: Hsla) -> Self {
        self.active = color;
        self
    }

    /// Set shadow, default is false.
    pub fn shadow(mut self, shadow: bool) -> Self {
        self.shadow = shadow;
        self
    }
}

/// The variant of the Button.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum ButtonVariant {
    #[default]
    Default,
    Primary,
    Secondary,
    Danger,
    Info,
    Success,
    Warning,
    Ghost,
    Link,
    Text,
    Custom(ButtonCustomVariant),
}

impl ButtonVariant {
    #[inline]
    pub fn is_link(&self) -> bool {
        matches!(self, Self::Link)
    }

    #[inline]
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text)
    }

    #[inline]
    pub fn is_ghost(&self) -> bool {
        matches!(self, Self::Ghost)
    }

    #[inline]
    fn no_padding(&self) -> bool {
        self.is_link() || self.is_text()
    }

    #[inline]
    fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }
}

fn resolve_button_icon_size(button_size: Size, glyph_size: Option<IconSize>) -> Size {
    glyph_size
        .map(Size::from)
        .unwrap_or_else(|| match button_size {
            Size::Size(value) => Size::Size(value * 0.75),
            _ => button_size,
        })
}

/// A Button element.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    base: Stateful<Div>,
    style: StyleRefinement,
    icon: Option<ButtonIcon>,
    label: Option<SharedString>,
    children: Vec<AnyElement>,
    disabled: bool,
    pub(crate) selected: bool,
    variant: ButtonVariant,
    rounded: ButtonRounded,
    outline: bool,
    border_corners: Corners<bool>,
    border_edges: Edges<bool>,
    dropdown_caret: bool,
    size: Size,
    glyph_size: Option<IconSize>,
    compact: bool,
    tooltip: Option<(
        SharedString,
        Option<(Rc<Box<dyn gpui::Action>>, Option<SharedString>)>,
    )>,
    accessible_label: Option<SharedString>,
    tooltip_builder: Option<Rc<dyn Fn(&mut Window, &mut App) -> gpui::AnyView>>,
    on_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    on_hover: Option<Rc<dyn Fn(&bool, &mut Window, &mut App)>>,
    loading: bool,
    loading_icon: Option<Icon>,

    tab_index: isize,
    tab_stop: bool,
}

impl From<Button> for AnyElement {
    fn from(button: Button) -> Self {
        button.into_any_element()
    }
}

impl Button {
    pub fn new(id: impl Into<ElementId>) -> Self {
        let id = id.into();

        Self {
            id: id.clone(),
            // ID must be set after div is created;
            // `dropdown_menu` uses this id to create the popup menu.
            base: div().flex_shrink_0().id(id),
            style: StyleRefinement::default(),
            icon: None,
            label: None,
            disabled: false,
            selected: false,
            variant: ButtonVariant::default(),
            rounded: ButtonRounded::Medium,
            border_corners: Corners {
                top_left: true,
                top_right: true,
                bottom_right: true,
                bottom_left: true,
            },
            border_edges: Edges::all(true),
            size: Size::Medium,
            glyph_size: None,
            tooltip: None,
            accessible_label: None,
            tooltip_builder: None,
            on_click: None,
            on_hover: None,
            loading: false,
            compact: false,
            outline: false,
            children: Vec::new(),
            loading_icon: None,
            dropdown_caret: false,
            tab_index: 0,
            tab_stop: true,
        }
    }

    /// Set the outline style of the Button.
    pub fn outline(mut self) -> Self {
        self.outline = true;
        self
    }

    /// Set the border radius of the Button.
    pub fn rounded(mut self, rounded: impl Into<ButtonRounded>) -> Self {
        self.rounded = rounded.into();
        self
    }

    /// Set the border corners side of the Button.
    pub(crate) fn border_corners(mut self, corners: impl Into<Corners<bool>>) -> Self {
        self.border_corners = corners.into();
        self
    }

    /// Set the border edges of the Button.
    pub(crate) fn border_edges(mut self, edges: impl Into<Edges<bool>>) -> Self {
        self.border_edges = edges.into();
        self
    }

    /// Set label to the Button, if no label is set, the button will be in Icon Button mode.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the icon of the button, if the Button have no label, the button well in Icon Button mode.
    pub fn icon(mut self, icon: impl Into<ButtonIcon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the icon glyph size independently from the button hit target.
    pub fn glyph_size(mut self, size: IconSize) -> Self {
        self.glyph_size = Some(size);
        self
    }

    /// Set the tooltip of the button.
    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        let tooltip = tooltip.into();
        if self.accessible_label.is_none() {
            self.accessible_label = Some(tooltip.clone());
        }
        self.tooltip = Some((tooltip, None));
        self
    }

    /// Set the accessible label announced for this button.
    ///
    /// Tooltips provide the default label for icon-only actions, while an
    /// explicit accessible label always takes precedence.
    pub fn accessible_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessible_label = Some(label.into());
        self
    }

    /// Set the tooltip of the button with action to show keybinding.
    pub fn tooltip_with_action(
        mut self,
        tooltip: impl Into<SharedString>,
        action: &dyn gpui::Action,
        context: Option<&str>,
    ) -> Self {
        let tooltip = tooltip.into();
        if self.accessible_label.is_none() {
            self.accessible_label = Some(tooltip.clone());
        }
        self.tooltip = Some((
            tooltip,
            Some((
                Rc::new(action.boxed_clone()),
                context.map(|c| c.to_string().into()),
            )),
        ));
        self
    }

    /// Set true to show the loading indicator.
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// Set the button to compact mode, then padding will be reduced.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// Add click handler.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// Add hover handler, the bool parameter indicates whether the mouse is hovering.
    pub fn on_hover(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_hover = Some(Rc::new(handler));
        self
    }

    /// Set the loading icon of the button, it will be used when loading is true.
    ///
    /// Default is a spinner icon.
    pub fn loading_icon(mut self, icon: impl Into<Icon>) -> Self {
        self.loading_icon = Some(icon.into());
        self
    }

    /// Set the tab index of the button, it will be used to focus the button by tab key.
    ///
    /// Default is 0.
    pub fn tab_index(mut self, tab_index: isize) -> Self {
        self.tab_index = tab_index;
        self
    }

    /// Set the tab stop of the button, if true, the button will be focusable by tab key.
    ///
    /// Default is true.
    pub fn tab_stop(mut self, tab_stop: bool) -> Self {
        self.tab_stop = tab_stop;
        self
    }

    /// Set to show a dropdown caret icon at the end of the button.
    pub fn dropdown_caret(mut self, dropdown_caret: bool) -> Self {
        self.dropdown_caret = dropdown_caret;
        self
    }

    #[inline]
    fn clickable(&self) -> bool {
        !self.is_unavailable() && self.on_click.is_some()
    }

    #[inline]
    fn hoverable(&self) -> bool {
        !self.is_unavailable() && self.on_hover.is_some()
    }

    #[inline]
    fn is_unavailable(&self) -> bool {
        self.disabled || self.loading
    }

    #[inline]
    fn effective_tab_stop(&self) -> bool {
        self.tab_stop && !self.disabled
    }
}

impl Disableable for Button {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Selectable for Button {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl Sizable for Button {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl ButtonVariants for Button {
    fn with_variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Button {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements)
    }
}

impl InteractiveElement for Button {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Button {}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let style: ButtonVariant = self.variant;
        let clickable = self.clickable();
        let is_unavailable = self.is_unavailable();
        let hoverable = self.hoverable();
        let effective_tab_stop = self.effective_tab_stop();
        let normal_style = style.normal(self.outline, cx);
        let icon_size = resolve_button_icon_size(self.size, self.glyph_size);
        let control_height = cx.theme().geometry.control.height(self.size);

        let focus_handle = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let is_focused = focus_handle.is_focused(window);

        let rounding = match self.rounded {
            ButtonRounded::Small => cx.theme().radius * 0.5,
            ButtonRounded::Medium => cx.theme().radius,
            ButtonRounded::Large => cx.theme().radius * 2.0,
            ButtonRounded::Size(px) => px,
            ButtonRounded::None => Pixels::ZERO,
        };

        self.base
            .role(gpui::Role::Button)
            .when_some(self.accessible_label, |this, label| this.aria_label(label))
            .track_focus(
                &focus_handle
                    .tab_index(self.tab_index)
                    .tab_stop(effective_tab_stop),
            )
            .cursor_default()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .cursor_default()
            .when(self.variant.is_link(), |this| this.cursor_pointer())
            .when(cx.theme().shadow && normal_style.shadow, |this| {
                this.shadow_xs()
            })
            .when(!style.no_padding(), |this| {
                if self.label.is_none() && self.children.is_empty() {
                    // Icon Button
                    this.size(control_height)
                } else {
                    // Normal Button
                    this.h(control_height).map(|this| match self.size {
                        Size::Size(size) => this.px(size * 0.2),
                        Size::XSmall => this.px_1().when(self.compact, |this| this.min_w_5()),
                        Size::Small => this
                            .px_3()
                            .when(self.compact, |this| this.min_w_6().px_1p5()),
                        _ => this.px_4().when(self.compact, |this| this.min_w_8().px_2()),
                    })
                }
            })
            .when(self.border_corners.top_left, |this| {
                this.rounded_tl(rounding)
            })
            .when(self.border_corners.top_right, |this| {
                this.rounded_tr(rounding)
            })
            .when(self.border_corners.bottom_left, |this| {
                this.rounded_bl(rounding)
            })
            .when(self.border_corners.bottom_right, |this| {
                this.rounded_br(rounding)
            })
            .when(self.variant.is_default() || self.outline, |this| {
                this.when(self.border_edges.left, |this| this.border_l_1())
                    .when(self.border_edges.right, |this| this.border_r_1())
                    .when(self.border_edges.top, |this| this.border_t_1())
                    .when(self.border_edges.bottom, |this| this.border_b_1())
            })
            .refine_style(&self.style)
            .text_color(normal_style.fg)
            .when(self.selected, |this| {
                let selected_style = style.selected(self.outline, cx);
                this.bg(selected_style.bg)
                    .border_color(selected_style.border)
                    .text_color(selected_style.fg)
            })
            .when(!is_unavailable && !self.selected, |this| {
                this.border_color(normal_style.border)
                    .bg(normal_style.bg)
                    .when(normal_style.underline, |this| this.text_decoration_1())
                    .hover(|this| {
                        let hover_style = style.hovered(self.outline, cx);
                        this.bg(hover_style.bg)
                            .border_color(hover_style.border)
                            .text_color(hover_style.fg)
                    })
                    .active(|this| {
                        let active_style = style.active(self.outline, cx);
                        this.bg(active_style.bg)
                            .border_color(active_style.border)
                            .text_color(active_style.fg)
                    })
            })
            .when(is_unavailable, |this| {
                let disabled_style = style.disabled(self.outline, cx);
                this.bg(disabled_style.bg)
                    .text_color(disabled_style.fg)
                    .border_color(disabled_style.border)
                    .shadow_none()
            })
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                // Stop all button and dropdown interaction while unavailable.
                if is_unavailable {
                    cx.stop_propagation();
                    return;
                }

                // Avoid focus on mouse down.
                window.prevent_default();
            })
            .when_some(self.on_click, |this, on_click| {
                this.on_click(move |event, window, cx| {
                    // Stop handle any click event when disabled.
                    // To avoid handle dropdown menu open when button is disabled.
                    if !clickable {
                        cx.stop_propagation();
                        return;
                    }

                    on_click(event, window, cx);
                })
            })
            .when_some(self.on_hover.filter(|_| hoverable), |this, on_hover| {
                this.on_hover(move |hovered, window, cx| {
                    on_hover(hovered, window, cx);
                })
            })
            .child({
                h_flex()
                    .id("label")
                    .size_full()
                    .items_center()
                    .justify_center()
                    .button_text_size(self.size)
                    .map(|this| match self.size {
                        Size::XSmall => this.gap_1(),
                        Size::Small => this.gap_1(),
                        _ => this.gap_2(),
                    })
                    .when_some(self.icon, |this, icon| {
                        this.child(
                            icon.loading_icon(self.loading_icon)
                                .loading(self.loading)
                                .with_size(icon_size),
                        )
                    })
                    .when_some(self.label, |this, label| {
                        this.child(div().flex_none().line_height(relative(1.)).child(label))
                    })
                    .children(self.children)
                    .when(self.dropdown_caret, |this| {
                        this.justify_between().child(
                            Icon::new(IconName::ChevronDown).xsmall().text_color(
                                match is_unavailable {
                                    true => normal_style.fg.opacity(0.3),
                                    false => normal_style.fg.opacity(0.5),
                                },
                            ),
                        )
                    })
            })
            .map(|this| {
                if let Some(builder) = self.tooltip_builder {
                    this.managed_tooltip(move |window, cx| builder(window, cx))
                } else if let Some((tooltip, action)) = self.tooltip {
                    this.managed_tooltip(move |window, cx| {
                        Tooltip::new(tooltip.clone())
                            .when_some(action.clone(), |this, (action, context)| {
                                this.action(
                                    action.boxed_clone().as_ref(),
                                    context.as_ref().map(|c| c.as_ref()),
                                )
                            })
                            .build(window, cx)
                    })
                } else {
                    this
                }
            })
            .focus_ring(is_focused, px(0.), window, cx)
    }
}

struct ButtonVariantStyle {
    bg: Hsla,
    border: Hsla,
    fg: Hsla,
    underline: bool,
    shadow: bool,
}

impl ButtonVariant {
    fn bg_color(&self, outline: bool, cx: &mut App) -> Hsla {
        if outline {
            return cx.theme().input_background();
        }

        match self {
            Self::Default => cx.theme().input_background(),
            Self::Primary => cx.theme().button_primary,
            Self::Secondary => cx.theme().secondary,
            Self::Danger => cx.theme().danger.mix_oklab(cx.theme().transparent, 0.2),
            Self::Warning => cx.theme().warning.mix_oklab(cx.theme().transparent, 0.2),
            Self::Success => cx.theme().success.mix_oklab(cx.theme().transparent, 0.2),
            Self::Info => cx.theme().info.mix_oklab(cx.theme().transparent, 0.2),
            Self::Ghost | Self::Link | Self::Text => cx.theme().transparent,
            Self::Custom(colors) => colors.color,
        }
    }

    fn text_color(&self, outline: bool, cx: &mut App) -> Hsla {
        match self {
            Self::Default => cx.theme().foreground,
            Self::Primary => {
                if outline {
                    cx.theme().button_primary
                } else {
                    cx.theme().button_primary_foreground
                }
            }
            Self::Secondary | Self::Ghost => cx.theme().secondary_foreground,
            Self::Danger => cx.theme().danger,
            Self::Warning => cx.theme().warning,
            Self::Success => cx.theme().success,
            Self::Info => cx.theme().info,
            Self::Link => cx.theme().link,
            Self::Text => cx.theme().foreground,
            Self::Custom(colors) => colors.foreground,
        }
    }

    fn border_color(&self, _bg: Hsla, outline: bool, cx: &mut App) -> Hsla {
        match self {
            Self::Default => cx.theme().input,
            Self::Secondary => cx.theme().border,
            Self::Primary => cx.theme().button_primary,
            Self::Danger => {
                if outline {
                    cx.theme().danger.mix_oklab(transparent_white(), 0.4)
                } else {
                    cx.theme().danger
                }
            }
            Self::Info => {
                if outline {
                    cx.theme().info.mix_oklab(transparent_white(), 0.4)
                } else {
                    cx.theme().info
                }
            }
            Self::Warning => {
                if outline {
                    cx.theme().warning.mix_oklab(transparent_white(), 0.4)
                } else {
                    cx.theme().warning
                }
            }
            Self::Success => {
                if outline {
                    cx.theme().success.mix_oklab(transparent_white(), 0.4)
                } else {
                    cx.theme().success
                }
            }
            Self::Ghost | Self::Link | Self::Text => cx.theme().transparent,
            Self::Custom(colors) => {
                if outline {
                    colors.color.mix_oklab(transparent_white(), 0.4)
                } else {
                    colors.color
                }
            }
        }
    }

    fn underline(&self, _: &App) -> bool {
        match self {
            Self::Link => true,
            _ => false,
        }
    }

    fn shadow(&self, outline: bool, _: &App) -> bool {
        match self {
            Self::Default => true,
            Self::Primary | Self::Secondary | Self::Danger => outline,
            Self::Custom(c) => c.shadow,
            _ => false,
        }
    }

    fn normal(&self, outline: bool, cx: &mut App) -> ButtonVariantStyle {
        let bg = self.bg_color(outline, cx);
        let border = self.border_color(bg, outline, cx);
        let fg = self.text_color(outline, cx);
        let underline = self.underline(cx);
        let shadow = self.shadow(outline, cx);

        ButtonVariantStyle {
            bg,
            border,
            fg,
            underline,
            shadow,
        }
    }

    fn hovered(&self, outline: bool, cx: &mut App) -> ButtonVariantStyle {
        let bg = match self {
            Self::Default => cx.theme().input.mix_oklab(cx.theme().transparent, 0.5),
            Self::Primary => {
                if outline {
                    cx.theme()
                        .button_primary
                        .mix_oklab(cx.theme().transparent, 0.2)
                } else {
                    cx.theme().button_primary_hover
                }
            }
            Self::Secondary => cx.theme().secondary_hover,
            Self::Danger => {
                if outline {
                    cx.theme().danger.mix_oklab(cx.theme().transparent, 0.2)
                } else {
                    cx.theme().danger.mix_oklab(cx.theme().transparent, 0.3)
                }
            }
            Self::Warning => {
                if outline {
                    cx.theme().warning.mix_oklab(cx.theme().transparent, 0.2)
                } else {
                    cx.theme().warning.mix_oklab(cx.theme().transparent, 0.3)
                }
            }
            Self::Success => {
                if outline {
                    cx.theme().success.mix_oklab(cx.theme().transparent, 0.2)
                } else {
                    cx.theme().success.mix_oklab(cx.theme().transparent, 0.3)
                }
            }
            Self::Info => {
                if outline {
                    cx.theme().info.mix_oklab(cx.theme().transparent, 0.2)
                } else {
                    cx.theme().info.mix_oklab(cx.theme().transparent, 0.3)
                }
            }
            Self::Custom(colors) => colors.hover,
            Self::Ghost => {
                if cx.theme().mode.is_dark() {
                    cx.theme().secondary.lighten(0.1).opacity(0.8)
                } else {
                    cx.theme().secondary.darken(0.1).opacity(0.8)
                }
            }
            Self::Link => cx.theme().transparent,
            Self::Text => cx.theme().transparent,
        };

        let border = self.border_color(bg, outline, cx);
        let fg = match self {
            Self::Link => cx.theme().link_hover,
            _ => self.text_color(outline, cx),
        };

        let underline = self.underline(cx);
        let shadow = self.shadow(outline, cx);

        ButtonVariantStyle {
            bg,
            border,
            fg,
            underline,
            shadow,
        }
    }

    fn active(&self, outline: bool, cx: &mut App) -> ButtonVariantStyle {
        let bg = match self {
            Self::Default => cx.theme().input.mix_oklab(cx.theme().transparent, 0.7),
            Self::Primary => {
                if outline {
                    cx.theme()
                        .button_primary
                        .mix_oklab(cx.theme().transparent, 0.4)
                } else {
                    cx.theme().button_primary_active
                }
            }
            Self::Secondary => cx.theme().secondary_active,
            Self::Ghost => {
                if cx.theme().mode.is_dark() {
                    cx.theme().secondary.lighten(0.2).opacity(0.8)
                } else {
                    cx.theme().secondary.darken(0.2).opacity(0.8)
                }
            }
            Self::Danger => cx.theme().danger.mix_oklab(cx.theme().transparent, 0.4),
            Self::Warning => cx.theme().warning.mix_oklab(cx.theme().transparent, 0.4),
            Self::Success => cx.theme().success.mix_oklab(cx.theme().transparent, 0.4),
            Self::Info => cx.theme().info.mix_oklab(cx.theme().transparent, 0.4),
            Self::Custom(colors) => colors.active,
            Self::Link => cx.theme().transparent,
            Self::Text => cx.theme().transparent,
        };
        let border = self.border_color(bg, outline, cx);
        let fg = match self {
            Self::Link => cx.theme().link_active,
            Self::Text => cx.theme().foreground.opacity(0.7),
            _ => self.text_color(outline, cx),
        };
        let underline = self.underline(cx);
        let shadow = self.shadow(outline, cx);

        ButtonVariantStyle {
            bg,
            border,
            fg,
            underline,
            shadow,
        }
    }

    fn selected(&self, outline: bool, cx: &mut App) -> ButtonVariantStyle {
        let bg = match self {
            Self::Default => cx.theme().input.mix_oklab(cx.theme().transparent, 0.7),
            Self::Primary => cx.theme().button_primary_active,
            Self::Secondary | Self::Ghost => cx.theme().secondary_active,
            Self::Danger => cx.theme().danger_active,
            Self::Warning => cx.theme().warning_active,
            Self::Success => cx.theme().success_active,
            Self::Info => cx.theme().info_active,
            Self::Link => cx.theme().transparent,
            Self::Text => cx.theme().transparent,
            Self::Custom(colors) => colors.active,
        };

        let border = self.border_color(bg, outline, cx);
        let fg = match self {
            Self::Link => cx.theme().link_active,
            Self::Text => cx.theme().foreground.opacity(0.7),
            _ => self.text_color(false, cx),
        };
        let underline = self.underline(cx);
        let shadow = self.shadow(outline, cx);

        ButtonVariantStyle {
            bg,
            border,
            fg,
            underline,
            shadow,
        }
    }

    fn disabled(&self, outline: bool, cx: &mut App) -> ButtonVariantStyle {
        let bg = match self {
            Self::Default | Self::Link | Self::Ghost | Self::Text => cx.theme().transparent,
            Self::Primary => cx.theme().button_primary.opacity(0.15),
            Self::Danger => cx.theme().danger.opacity(0.15),
            Self::Warning => cx.theme().warning.opacity(0.15),
            Self::Success => cx.theme().success.opacity(0.15),
            Self::Info => cx.theme().info.opacity(0.15),
            Self::Secondary => cx.theme().secondary.opacity(1.5),
            Self::Custom(style) => style.color.opacity(0.35),
        };
        let fg = match self {
            Self::Custom(style) => style.foreground.opacity(0.45),
            _ => cx.theme().muted_foreground.opacity(0.5),
        };
        let (bg, border) = if outline {
            (
                cx.theme().input_background().opacity(0.5),
                cx.theme().border.opacity(0.5),
            )
        } else if let Self::Default = self {
            (
                cx.theme().input_background().opacity(0.5),
                cx.theme().input.opacity(0.5),
            )
        } else {
            (bg, bg)
        };

        let underline = self.underline(cx);
        let shadow = false;

        ButtonVariantStyle {
            bg,
            border,
            fg,
            underline,
            shadow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    gpui::actions!(button_tests, [CreateQuery]);

    #[gpui::test]
    fn test_button_builder(_cx: &mut gpui::TestAppContext) {
        let button = Button::new("complex-button")
            .label("Save Changes")
            .primary()
            .outline()
            .large()
            .tooltip("Click to save")
            .compact()
            .loading(false)
            .disabled(false)
            .selected(false)
            .tab_index(1)
            .tab_stop(true)
            .dropdown_caret(false)
            .glyph_size(IconSize::Medium)
            .rounded(ButtonRounded::Medium)
            .on_click(|_, _, _| {});

        assert_eq!(button.label, Some("Save Changes".into()));
        assert_eq!(button.variant, ButtonVariant::Primary);
        assert!(button.outline);
        assert_eq!(button.size, Size::Large);
        assert!(button.tooltip.is_some());
        assert!(button.compact);
        assert!(!button.loading);
        assert!(!button.disabled);
        assert!(!button.selected);
        assert_eq!(button.tab_index, 1);
        assert!(button.tab_stop);
        assert!(!button.dropdown_caret);
        assert_eq!(button.glyph_size, Some(IconSize::Medium));
        assert!(matches!(button.rounded, ButtonRounded::Medium));
        assert_eq!(button.accessible_label, Some("Click to save".into()));
    }

    #[test]
    fn tooltip_supplies_accessible_label_without_overriding_an_explicit_label() {
        let tooltip_default = Button::new("tooltip-default").tooltip("Open settings");
        assert_eq!(
            tooltip_default.accessible_label,
            Some("Open settings".into())
        );

        let explicit_after = Button::new("explicit-after")
            .tooltip("Open settings")
            .accessible_label("Settings");
        assert_eq!(explicit_after.accessible_label, Some("Settings".into()));

        let explicit_before = Button::new("explicit-before")
            .accessible_label("Settings")
            .tooltip("Open settings");
        assert_eq!(explicit_before.accessible_label, Some("Settings".into()));
    }

    #[gpui::test]
    fn tooltip_with_action_supplies_accessible_label(cx: &mut gpui::TestAppContext) {
        cx.update(|_| {
            let button = Button::new("tooltip-action").tooltip_with_action(
                "Create query",
                &CreateQuery,
                None,
            );

            assert_eq!(button.accessible_label, Some("Create query".into()));
        });
    }

    #[test]
    fn explicit_glyph_size_is_independent_from_button_hit_size() {
        assert_eq!(
            resolve_button_icon_size(Size::Size(px(36.)), Some(IconSize::Medium)),
            Size::Size(px(20.))
        );
        assert_eq!(
            resolve_button_icon_size(Size::Size(px(36.)), None),
            Size::Size(px(27.))
        );
    }

    #[gpui::test]
    fn test_button_clickable_logic(_cx: &mut gpui::TestAppContext) {
        // Button with click handler should be clickable
        let clickable = Button::new("test").on_click(|_, _, _| {});
        assert!(clickable.clickable());

        // Disabled button should not be clickable
        let disabled = Button::new("test").disabled(true).on_click(|_, _, _| {});
        assert!(!disabled.clickable());

        // Loading button should not be clickable
        let loading = Button::new("test").loading(true).on_click(|_, _, _| {});
        assert!(!loading.clickable());
    }

    #[test]
    fn loading_keeps_tab_focus_while_disabled_leaves_the_tab_order() {
        let loading = Button::new("loading").loading(true).tab_stop(true);
        assert!(loading.effective_tab_stop());

        let disabled = Button::new("disabled").disabled(true).tab_stop(true);
        assert!(!disabled.effective_tab_stop());

        let opt_out = Button::new("opt-out").loading(true).tab_stop(false);
        assert!(!opt_out.effective_tab_stop());
    }

    #[gpui::test]
    fn test_button_variant_methods(_cx: &mut gpui::TestAppContext) {
        // Test variant check methods
        assert!(ButtonVariant::Link.is_link());
        assert!(ButtonVariant::Text.is_text());
        assert!(ButtonVariant::Ghost.is_ghost());

        // Test no_padding logic
        assert!(ButtonVariant::Link.no_padding());
        assert!(ButtonVariant::Text.no_padding());
        assert!(!ButtonVariant::Ghost.no_padding());
    }

    #[gpui::test]
    fn test_custom_variant_uses_configured_colors(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::Theme::default());
            let background: Hsla = gpui::rgb(0x111111).into();
            let foreground: Hsla = gpui::rgb(0xeeeeee).into();
            let hover: Hsla = gpui::rgb(0x333333).into();
            let active: Hsla = gpui::rgb(0x555555).into();
            let variant = ButtonVariant::Custom(
                ButtonCustomVariant::new(cx)
                    .color(background)
                    .foreground(foreground)
                    .hover(hover)
                    .active(active),
            );

            assert_eq!(variant.normal(false, cx).bg, background);
            assert_eq!(variant.normal(false, cx).fg, foreground);
            assert_eq!(variant.hovered(false, cx).bg, hover);
            assert_eq!(variant.active(false, cx).bg, active);
            assert_eq!(variant.disabled(false, cx).fg, foreground.opacity(0.45));
        });
    }
}
