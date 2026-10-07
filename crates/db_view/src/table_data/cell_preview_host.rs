use crate::sidebar::cell_preview_panel::CellPreviewPanel;
use crate::table_data::data_grid::{DataGrid, DataGridEvent};
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Context, Div, DragMoveEvent, Entity, EntityId, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, Stateful,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
use gpui_component::{ActiveTheme, h_flex};
use std::{cell::Cell, rc::Rc};

const DEFAULT_PREVIEW_WIDTH: Pixels = px(420.0);
const MIN_PREVIEW_WIDTH: Pixels = px(280.0);
const MAX_PREVIEW_WIDTH: Pixels = px(800.0);
/// 预览分隔条抓取区在分隔线两侧各自的宽度。
const PREVIEW_RESIZE_GRAB_PADDING: Pixels = px(4.0);
/// 分隔线本身的宽度。
const PREVIEW_RESIZE_LINE_WIDTH: Pixels = px(1.0);

#[cfg(test)]
mod resize_tests;

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

    /// 预览分隔条的一侧抓取区：只管命中、光标与拖动，位置与外观由调用方决定。
    ///
    /// 抓取区以分隔线为中心、两侧各 [`PREVIEW_RESIZE_GRAB_PADDING`]，两半分别挂在
    /// 数据表格一侧和预览面板一侧。以前是一条 6px、整条压在数据表格那边的抓取区：
    /// 从预览面板那边靠过来完全没有反馈，鼠标得精确停在那几个像素上才拉得动。
    fn render_preview_resize_grab(
        &self,
        initial_x: Rc<Cell<Option<Pixels>>>,
        selector: &'static str,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let preview_width = self.preview_width;
        div()
            .id(selector)
            .debug_selector(|| selector.to_owned())
            .cursor_col_resize()
            .occlude()
            .flex()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_drag_move(
                cx.listener(|this, e: &DragMoveEvent<ResizeCellPreview>, _window, cx| {
                    let drag = e.drag(cx);
                    if drag.entity_id != cx.entity_id() {
                        return;
                    }
                    let Some(initial_x) = drag.initial_x.get() else {
                        return;
                    };

                    this.preview_width =
                        resized_preview_width(drag.initial_width, initial_x, e.event.position.x);
                    cx.notify();
                }),
            )
            .on_drag(
                ResizeCellPreview {
                    entity_id: cx.entity_id(),
                    initial_width: preview_width,
                    initial_x,
                },
                |drag, _, window, cx| {
                    drag.initial_x.set(Some(window.mouse_position().x));
                    cx.stop_propagation();
                    cx.new(|_| drag.clone())
                },
            )
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
                    self.render_preview_resize_grab(initial_x.clone(), "cell-preview-resize", cx)
                        .group("cell-preview-resize")
                        .w(PREVIEW_RESIZE_GRAB_PADDING)
                        .h_full()
                        .flex_shrink_0()
                        .justify_end()
                        .child(
                            div()
                                .h_full()
                                .w(PREVIEW_RESIZE_LINE_WIDTH)
                                .bg(cx.theme().border)
                                .group_hover("cell-preview-resize", |this| {
                                    this.bg(cx.theme().primary)
                                }),
                        ),
                )
                .child(
                    div()
                        .relative()
                        .w(self.preview_width)
                        .h_full()
                        .flex_shrink_0()
                        .child(self.preview_panel.clone())
                        .child(
                            self.render_preview_resize_grab(
                                initial_x,
                                "cell-preview-resize-leading",
                                cx,
                            )
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left_0()
                            .w(PREVIEW_RESIZE_GRAB_PADDING),
                        ),
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
