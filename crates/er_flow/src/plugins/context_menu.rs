use std::sync::Arc;

use gpui::{
    IntoElement as _, MouseButton, ParentElement as _, Pixels, Point, SharedString, Styled as _,
    div, px, rgb,
};

use crate::{
    NodeId,
    plugin::{
        EventResult, FlowEvent, InputEvent, Plugin, PluginContext, RenderContext, RenderLayer,
    },
};

use super::{
    clipboard::{
        extract_subgraph, get_clipboard_subgraph, has_clipboard_subgraph, paste_subgraph,
        set_clipboard_subgraph,
    },
    delete::delete_selection,
    fit_all::fit_entire_graph,
    focus_selection::focus_viewport_on_selection,
    select_all_viewport::select_all_in_viewport,
};

const MENU_W: f32 = 228.0;
const ROW_H: f32 = 26.0;
const SEP_H: f32 = 9.0;
const MENU_PAD: f32 = 4.0;

/// Callback invoked when the user picks a custom canvas menu row (e.g. open an input dialog in the app).
///
/// The second argument is the **world-space** point under the initial right-click that opened this menu
/// (same as [`PluginContext::screen_to_world`] applied to that click).
#[derive(Clone)]
pub struct ContextMenuCustomAction(
    Arc<dyn for<'a> Fn(&mut PluginContext<'a>, Point<Pixels>) + Send + Sync>,
);

impl ContextMenuCustomAction {
    pub fn new(
        f: impl for<'a> Fn(&mut PluginContext<'a>, Point<Pixels>) + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(f))
    }

    fn call(&self, ctx: &mut PluginContext<'_>, menu_world: Point<Pixels>) {
        (self.0)(ctx, menu_world);
    }
}

/// One extra row on the **canvas background** context menu (after built-in items).
#[derive(Clone)]
pub struct ContextMenuCanvasExtra {
    pub label: SharedString,
    pub shortcut: Option<SharedString>,
    pub on_select: ContextMenuCustomAction,
}

impl ContextMenuCanvasExtra {
    pub fn new(
        label: impl Into<SharedString>,
        on_select: impl for<'a> Fn(&mut PluginContext<'a>, Point<Pixels>) + Send + Sync + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            shortcut: None,
            on_select: ContextMenuCustomAction::new(on_select),
        }
    }

    pub fn with_shortcut(
        label: impl Into<SharedString>,
        shortcut: impl Into<SharedString>,
        on_select: impl for<'a> Fn(&mut PluginContext<'a>, Point<Pixels>) + Send + Sync + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            shortcut: Some(shortcut.into()),
            on_select: ContextMenuCustomAction::new(on_select),
        }
    }
}

/// Right-click menu on the canvas (empty area) or on a node. Optional [`ContextMenuCanvasExtra`] rows
/// are appended after built-in canvas actions.
pub struct ContextMenuPlugin {
    open: Option<OpenMenu>,
    canvas_extras: Vec<ContextMenuCanvasExtra>,
}

#[derive(Clone, Copy)]
enum MenuBuiltin {
    FitAllGraph,
    Paste,
    SelectAllViewport,
    FocusSelection,
    Copy,
    Delete,
    BringToFront(NodeId),
}

#[derive(Clone)]
enum MenuItem {
    Separator,
    Builtin(MenuBuiltin),
    Custom {
        label: SharedString,
        shortcut: Option<SharedString>,
        action: ContextMenuCustomAction,
    },
}

#[derive(Clone)]
struct OpenMenu {
    anchor: Point<Pixels>,
    /// World position of the right-click that opened this menu.
    anchor_world: Point<Pixels>,
    actions: Vec<MenuItem>,
}

impl ContextMenuPlugin {
    pub fn new() -> Self {
        Self {
            open: None,
            canvas_extras: Vec::new(),
        }
    }

    pub fn with_canvas_extras(canvas_extras: Vec<ContextMenuCanvasExtra>) -> Self {
        Self {
            open: None,
            canvas_extras,
        }
    }

    /// Append a canvas-background row with a custom label (e.g. “Add node…” → show input in meili).
    pub fn canvas_row(
        mut self,
        label: impl Into<SharedString>,
        on_select: impl for<'a> Fn(&mut PluginContext<'a>, Point<Pixels>) + Send + Sync + 'static,
    ) -> Self {
        self.canvas_extras
            .push(ContextMenuCanvasExtra::new(label, on_select));
        self
    }

    /// Same as [`Self::canvas_row`] but with a shortcut hint string shown on the right.
    pub fn canvas_row_with_shortcut(
        mut self,
        label: impl Into<SharedString>,
        shortcut: impl Into<SharedString>,
        on_select: impl for<'a> Fn(&mut PluginContext<'a>, Point<Pixels>) + Send + Sync + 'static,
    ) -> Self {
        self.canvas_extras
            .push(ContextMenuCanvasExtra::with_shortcut(
                label, shortcut, on_select,
            ));
        self
    }

    fn row_height(action: &MenuItem) -> f32 {
        match action {
            MenuItem::Separator => SEP_H,
            _ => ROW_H,
        }
    }

    fn content_height(actions: &[MenuItem]) -> f32 {
        actions.iter().map(Self::row_height).sum()
    }

    fn menu_bounds(anchor: Point<Pixels>, actions: &[MenuItem]) -> gpui::Bounds<Pixels> {
        let h = Self::content_height(actions) + MENU_PAD * 2.0;
        gpui::Bounds::new(anchor, gpui::Size::new(px(MENU_W), px(h)))
    }

    fn label_builtin(b: MenuBuiltin) -> &'static str {
        match b {
            MenuBuiltin::FitAllGraph => "Fit entire graph",
            MenuBuiltin::Paste => "Paste",
            MenuBuiltin::SelectAllViewport => "Select all in view",
            MenuBuiltin::FocusSelection => "Focus selection",
            MenuBuiltin::Copy => "Copy",
            MenuBuiltin::Delete => "Delete",
            MenuBuiltin::BringToFront(_) => "Bring to front",
        }
    }

    fn shortcut_hint_builtin(b: MenuBuiltin) -> Option<&'static str> {
        #[cfg(target_os = "macos")]
        {
            match b {
                MenuBuiltin::FitAllGraph => Some("⌘0"),
                MenuBuiltin::Paste => Some("⌘V"),
                MenuBuiltin::SelectAllViewport => Some("⌘A"),
                MenuBuiltin::FocusSelection => Some("⌘⇧F"),
                MenuBuiltin::Copy => Some("⌘C"),
                MenuBuiltin::Delete => Some("⌫"),
                MenuBuiltin::BringToFront(_) => None,
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            match b {
                MenuBuiltin::FitAllGraph => Some("Ctrl+0"),
                MenuBuiltin::Paste => Some("Ctrl+V"),
                MenuBuiltin::SelectAllViewport => Some("Ctrl+A"),
                MenuBuiltin::FocusSelection => Some("Ctrl+Shift+F"),
                MenuBuiltin::Copy => Some("Ctrl+C"),
                MenuBuiltin::Delete => Some("Del"),
                MenuBuiltin::BringToFront(_) => None,
            }
        }
    }

    fn canvas_actions(&self, ctx: &PluginContext) -> Vec<MenuItem> {
        let mut v = Vec::new();
        v.push(MenuItem::Builtin(MenuBuiltin::FitAllGraph));
        v.push(MenuItem::Separator);
        if has_clipboard_subgraph(ctx) {
            v.push(MenuItem::Builtin(MenuBuiltin::Paste));
            v.push(MenuItem::Separator);
        }
        v.push(MenuItem::Builtin(MenuBuiltin::SelectAllViewport));
        v.push(MenuItem::Separator);
        v.push(MenuItem::Builtin(MenuBuiltin::FocusSelection));
        for e in &self.canvas_extras {
            v.push(MenuItem::Separator);
            v.push(MenuItem::Custom {
                label: e.label.clone(),
                shortcut: e.shortcut.clone(),
                action: e.on_select.clone(),
            });
        }
        v
    }

    fn node_actions(nid: NodeId) -> Vec<MenuItem> {
        vec![
            MenuItem::Builtin(MenuBuiltin::Copy),
            MenuItem::Separator,
            MenuItem::Builtin(MenuBuiltin::Delete),
            MenuItem::Builtin(MenuBuiltin::BringToFront(nid)),
            MenuItem::Separator,
            MenuItem::Builtin(MenuBuiltin::FocusSelection),
        ]
    }

    fn run_action(ctx: &mut PluginContext, action: &MenuItem, menu_world: Point<Pixels>) {
        match action {
            MenuItem::Separator => {}
            MenuItem::Builtin(b) => match b {
                MenuBuiltin::FitAllGraph => fit_entire_graph(ctx),
                MenuBuiltin::Paste => {
                    if let Some(sub) = get_clipboard_subgraph(ctx) {
                        paste_subgraph(ctx, &sub);
                    }
                }
                MenuBuiltin::SelectAllViewport => select_all_in_viewport(ctx),
                MenuBuiltin::FocusSelection => focus_viewport_on_selection(ctx),
                MenuBuiltin::Copy => {
                    if let Some(s) = extract_subgraph(ctx.graph) {
                        set_clipboard_subgraph(ctx, s);
                    }
                }
                MenuBuiltin::Delete => delete_selection(ctx),
                MenuBuiltin::BringToFront(id) => ctx.bring_node_to_front(*id),
            },
            MenuItem::Custom { action, .. } => action.call(ctx, menu_world),
        }
        ctx.notify();
    }

    fn row_at_dy(actions: &[MenuItem], dy: f32) -> Option<usize> {
        if dy < 0.0 {
            return None;
        }
        let mut y = 0.0;
        for (i, a) in actions.iter().enumerate() {
            let h = Self::row_height(a);
            if dy < y + h {
                return Some(i);
            }
            y += h;
        }
        None
    }
}

impl Plugin for ContextMenuPlugin {
    fn name(&self) -> &'static str {
        "context_menu"
    }

    fn setup(&mut self, _ctx: &mut crate::plugin::InitPluginContext) {}

    fn priority(&self) -> i32 {
        132
    }

    fn render_layer(&self) -> RenderLayer {
        RenderLayer::Overlay
    }

    fn render(&mut self, ctx: &mut RenderContext) -> Option<gpui::AnyElement> {
        let open = self.open.as_ref()?;
        let panel_bg = ctx.theme.context_menu_background;
        let panel_border = ctx.theme.context_menu_border;
        let row_text = ctx.theme.context_menu_text;
        let shortcut_text = ctx.theme.context_menu_shortcut_text;
        let separator = ctx.theme.context_menu_separator;

        let rows: Vec<_> = open
            .actions
            .iter()
            .map(|a| match a {
                MenuItem::Separator => div()
                    .w_full()
                    .h(px(SEP_H))
                    .flex()
                    .items_center()
                    .px_2()
                    .child(div().w_full().h(px(1.0)).bg(rgb(separator)))
                    .into_any_element(),
                MenuItem::Builtin(b) => {
                    let label = div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(ContextMenuPlugin::label_builtin(*b));
                    let shortcut = ContextMenuPlugin::shortcut_hint_builtin(*b).map(|h| {
                        div()
                            .flex_shrink_0()
                            .ml_2()
                            .text_xs()
                            .text_color(rgb(shortcut_text))
                            .child(h)
                    });
                    div()
                        .w_full()
                        .h(px(ROW_H))
                        .flex()
                        .flex_row()
                        .items_center()
                        .px_2()
                        .text_sm()
                        .text_color(rgb(row_text))
                        .child(label)
                        .children(shortcut)
                        .into_any_element()
                }
                MenuItem::Custom {
                    label, shortcut, ..
                } => {
                    let label_el = div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(label.clone());
                    let shortcut_el = shortcut.as_ref().map(|h| {
                        div()
                            .flex_shrink_0()
                            .ml_2()
                            .text_xs()
                            .text_color(rgb(shortcut_text))
                            .child(h.clone())
                    });
                    div()
                        .w_full()
                        .h(px(ROW_H))
                        .flex()
                        .flex_row()
                        .items_center()
                        .px_2()
                        .text_sm()
                        .text_color(rgb(row_text))
                        .child(label_el)
                        .children(shortcut_el)
                        .into_any_element()
                }
            })
            .collect();

        Some(
            div()
                .absolute()
                .left(open.anchor.x)
                .top(open.anchor.y)
                .w(px(MENU_W))
                .p_1()
                .bg(rgb(panel_bg))
                .border_1()
                .border_color(rgb(panel_border))
                .rounded(px(6.0))
                .shadow_sm()
                .children(rows)
                .into_any_element(),
        )
    }

    fn on_event(
        &mut self,
        event: &FlowEvent,
        ctx: &mut PluginContext,
    ) -> crate::plugin::EventResult {
        if let FlowEvent::Input(InputEvent::MouseDown(ev)) = event {
            if ev.button == MouseButton::Left {
                if let Some(open) = self.open.take() {
                    let menu_world = open.anchor_world;
                    let b = Self::menu_bounds(open.anchor, &open.actions);
                    if b.contains(&ev.position) {
                        let dy: f32 = (ev.position.y - open.anchor.y).into();
                        let inner_y = dy - MENU_PAD;
                        if let Some(row) = Self::row_at_dy(&open.actions, inner_y) {
                            let a = &open.actions[row];
                            if !matches!(a, MenuItem::Separator) {
                                Self::run_action(ctx, a, menu_world);
                            } else {
                                ctx.notify();
                            }
                        } else {
                            ctx.notify();
                        }
                        return EventResult::Stop;
                    }
                    ctx.notify();
                    return EventResult::Continue;
                }
                return EventResult::Continue;
            }

            if ev.button == MouseButton::Right {
                let world = ctx.screen_to_world(ev.position);
                let actions = if let Some(nid) = ctx.hit_node(world) {
                    if !ctx.graph.selected_node.contains(&nid) {
                        ctx.clear_selected_edge();
                        ctx.clear_selected_node();
                        ctx.add_selected_node(nid, false);
                    }
                    Self::node_actions(nid)
                } else {
                    self.canvas_actions(ctx)
                };
                self.open = Some(OpenMenu {
                    anchor: ev.position,
                    anchor_world: world,
                    actions,
                });
                ctx.notify();
                return EventResult::Stop;
            }
        }
        EventResult::Continue
    }
}
