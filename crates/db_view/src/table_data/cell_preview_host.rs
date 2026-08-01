use crate::sidebar::cell_preview_panel::CellPreviewPanel;
use crate::table_data::data_grid::{DataGrid, DataGridEvent};
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Context, DragMoveEvent, Entity, EntityId, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, StatefulInteractiveElement,
    Styled, Subscription, Window, div, px,
};
use gpui_component::{ActiveTheme, h_flex};
use std::{cell::Cell, rc::Rc};

const DEFAULT_PREVIEW_WIDTH: Pixels = px(420.0);
const MIN_PREVIEW_WIDTH: Pixels = px(280.0);
const MAX_PREVIEW_WIDTH: Pixels = px(800.0);
const PREVIEW_RESIZE_HANDLE_WIDTH: Pixels = px(6.0);

#[derive(Clone)]
struct ResizeCellPreview {
    entity_id: EntityId,
    initial_width: Pixels,
    initial_x: Rc<Cell<Option<Pixels>>>,
}

impl Render for ResizeCellPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size(px(0.0))
    }
}

fn resized_preview_width(initial_width: Pixels, initial_x: Pixels, current_x: Pixels) -> Pixels {
    (initial_width + initial_x - current_x)
        .max(MIN_PREVIEW_WIDTH)
        .min(MAX_PREVIEW_WIDTH)
}

fn run_save_if_flushed(flushed: bool, save: impl FnOnce()) -> bool {
    if !flushed {
        return false;
    }
    save();
    true
}

pub struct CellPreviewHost {
    data_grid: Entity<DataGrid>,
    preview_panel: Entity<CellPreviewPanel>,
    is_preview_open: bool,
    preview_width: Pixels,
    _grid_sub: Subscription,
    focus_handle: FocusHandle,
}

impl CellPreviewHost {
    pub fn new(data_grid: Entity<DataGrid>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let preview_panel = cx.new(|cx| CellPreviewPanel::new(window, cx));
        let grid_sub = cx.subscribe_in(
            &data_grid,
            window,
            |this, _, event: &DataGridEvent, window, cx| match event {
                DataGridEvent::ToggleLargeTextEditorRequested => {
                    this.toggle_preview(window, cx);
                }
                DataGridEvent::SaveChangesRequested => {
                    this.save_changes(window, cx);
                }
                DataGridEvent::LargeTextSelectionChanged
                | DataGridEvent::OpenTableDesignerRequested
                | DataGridEvent::OpenTableQueryRequested => {}
            },
        );

        Self {
            data_grid,
            preview_panel,
            is_preview_open: false,
            preview_width: DEFAULT_PREVIEW_WIDTH,
            _grid_sub: grid_sub,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn flush_pending(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.is_preview_open {
            return true;
        }

        self.preview_panel
            .update(cx, |panel, cx| panel.flush_pending(cx))
    }

    fn save_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let flushed = self.flush_pending(cx);
        run_save_if_flushed(flushed, || {
            self.data_grid.update(cx, |grid, cx| {
                grid.save_changes(window, cx);
            });
        })
    }

    fn toggle_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_preview_open {
            self.close_preview(window, cx);
        } else {
            self.open_preview(window, cx);
        }
    }

    fn open_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preview_panel.update(cx, |panel, cx| {
            panel.bind_data_grid(self.data_grid.downgrade(), window, cx);
        });
        self.is_preview_open = true;
        self.sync_button_state(true, cx);
        cx.notify();
    }

    fn close_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush_pending(cx) {
            return;
        }

        self.preview_panel.update(cx, |panel, cx| {
            panel.unbind(window, cx);
        });
        self.is_preview_open = false;
        self.sync_button_state(false, cx);
        cx.notify();
    }

    fn sync_button_state(&self, open: bool, cx: &mut Context<Self>) {
        let _ = self.data_grid.update(cx, |grid, cx| {
            grid.set_large_text_editor_sidebar_open(open, cx);
        });
    }
}

impl Focusable for CellPreviewHost {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CellPreviewHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .size_full()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.data_grid.clone()),
            )
            .when(self.is_preview_open, |this| {
                let initial_x = Rc::new(Cell::new(None));
                this.child(
                    div()
                        .id("cell-preview-resize")
                        .group("cell-preview-resize")
                        .w(PREVIEW_RESIZE_HANDLE_WIDTH)
                        .h_full()
                        .flex_shrink_0()
                        .cursor_col_resize()
                        .occlude()
                        .flex()
                        .justify_end()
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .h_full()
                                .w(px(1.0))
                                .bg(cx.theme().border)
                                .group_hover("cell-preview-resize", |this| {
                                    this.bg(cx.theme().primary)
                                }),
                        )
                        .on_drag_move(cx.listener(
                            |this, e: &DragMoveEvent<ResizeCellPreview>, _window, cx| {
                                let drag = e.drag(cx);
                                if drag.entity_id != cx.entity_id() {
                                    return;
                                }
                                let Some(initial_x) = drag.initial_x.get() else {
                                    return;
                                };

                                this.preview_width = resized_preview_width(
                                    drag.initial_width,
                                    initial_x,
                                    e.event.position.x,
                                );
                                cx.notify();
                            },
                        ))
                        .on_drag(
                            ResizeCellPreview {
                                entity_id: cx.entity_id(),
                                initial_width: self.preview_width,
                                initial_x,
                            },
                            |drag, _, window, cx| {
                                drag.initial_x.set(Some(window.mouse_position().x));
                                cx.stop_propagation();
                                cx.new(|_| drag.clone())
                            },
                        ),
                )
                .child(
                    div()
                        .w(self.preview_width)
                        .h_full()
                        .flex_shrink_0()
                        .child(self.preview_panel.clone()),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell as FlagCell;

    #[test]
    fn save_runs_after_successful_flush() {
        let saved = FlagCell::new(false);

        assert!(run_save_if_flushed(true, || saved.set(true)));
        assert!(saved.get());
    }

    #[test]
    fn save_is_skipped_when_flush_fails() {
        let saved = FlagCell::new(false);

        assert!(!run_save_if_flushed(false, || saved.set(true)));
        assert!(!saved.get());
    }

    #[test]
    fn preview_width_grows_when_left_handle_moves_left() {
        assert_eq!(
            px(480.0),
            resized_preview_width(px(420.0), px(500.0), px(440.0))
        );
    }

    #[test]
    fn preview_width_shrinks_when_left_handle_moves_right() {
        assert_eq!(
            px(360.0),
            resized_preview_width(px(420.0), px(500.0), px(560.0))
        );
    }

    #[test]
    fn preview_width_is_clamped_to_supported_bounds() {
        assert_eq!(
            MIN_PREVIEW_WIDTH,
            resized_preview_width(px(300.0), px(500.0), px(600.0))
        );
        assert_eq!(
            MAX_PREVIEW_WIDTH,
            resized_preview_width(px(780.0), px(500.0), px(400.0))
        );
    }
}
