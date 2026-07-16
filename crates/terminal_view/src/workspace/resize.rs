use std::ops::Deref;

use gpui::{
    App, AppContext as _, Bounds, Context, Element, ElementId, Entity, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, MouseMoveEvent, MouseUpEvent, Pixels, Point, Style,
    Window,
};
use one_core::layout::{SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH};
use one_ui::resize_handle::{HandlePlacement, ResizePanel, resize_handle};

use super::TerminalWorkspace;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WorkspaceSidebarResize {
    Left,
    Right,
    Bottom,
}

impl TerminalWorkspace {
    pub(super) fn render_sidebar_resize_handle(
        &self,
        target: WorkspaceSidebarResize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let workspace = cx.entity().downgrade();
        let (id, axis, placement) = match target {
            WorkspaceSidebarResize::Left => (
                "terminal-workspace-left-sidebar-resize",
                gpui::Axis::Horizontal,
                Some(HandlePlacement::Left),
            ),
            WorkspaceSidebarResize::Right => (
                "terminal-workspace-right-sidebar-resize",
                gpui::Axis::Horizontal,
                Some(HandlePlacement::Right),
            ),
            WorkspaceSidebarResize::Bottom => (
                "terminal-workspace-bottom-sidebar-resize",
                gpui::Axis::Vertical,
                None,
            ),
        };
        let handle = resize_handle::<ResizePanel, ResizePanel>(id, axis);
        let handle = match placement {
            Some(placement) => handle.placement(placement),
            None => handle,
        };
        handle.on_drag(ResizePanel, move |info, _, _, cx| {
            cx.stop_propagation();
            let _ = workspace.update(cx, |workspace, cx| {
                workspace.sidebar_resizing = Some(target);
                cx.notify();
            });
            cx.new(|_| info.deref().clone())
        })
    }

    fn resize_sidebar(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(target) = self.sidebar_resizing else {
            return;
        };
        let size = match target {
            WorkspaceSidebarResize::Left => position.x - self.workspace_bounds.left(),
            WorkspaceSidebarResize::Right => self.workspace_bounds.right() - position.x,
            WorkspaceSidebarResize::Bottom => self.workspace_bounds.bottom() - position.y,
        };
        self.sidebar_panel_size = size.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
        cx.notify();
    }

    fn finish_sidebar_resize(&mut self, cx: &mut Context<Self>) {
        self.sidebar_resizing = None;
        cx.notify();
    }
}

pub(super) struct WorkspaceResizeEventHandler {
    pub(super) workspace: Entity<TerminalWorkspace>,
}

impl IntoElement for WorkspaceResizeEventHandler {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for WorkspaceResizeEventHandler {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (window.request_layout(Style::default(), None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.workspace.update(cx, |workspace, _| {
            workspace.workspace_bounds = bounds;
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        window.on_mouse_event({
            let workspace = self.workspace.clone();
            move |event: &MouseMoveEvent, phase, _, cx| {
                if phase.bubble() && workspace.read(cx).sidebar_resizing.is_some() {
                    workspace.update(cx, |workspace, cx| {
                        workspace.resize_sidebar(event.position, cx);
                    });
                }
            }
        });
        window.on_mouse_event({
            let workspace = self.workspace.clone();
            move |_: &MouseUpEvent, phase, _, cx| {
                if phase.bubble() {
                    workspace.update(cx, |workspace, cx| {
                        workspace.finish_sidebar_resize(cx);
                    });
                }
            }
        });
    }
}
