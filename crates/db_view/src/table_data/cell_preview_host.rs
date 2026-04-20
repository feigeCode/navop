use crate::sidebar::cell_preview_panel::CellPreviewPanel;
use crate::table_data::data_grid::{DataGrid, DataGridEvent};
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    Styled, Subscription, Window, div, px,
};
use gpui_component::{ActiveTheme, h_flex};

const PAGE_PREVIEW_WIDTH: gpui::Pixels = px(420.0);

pub struct CellPreviewHost {
    data_grid: Entity<DataGrid>,
    preview_panel: Entity<CellPreviewPanel>,
    is_preview_open: bool,
    _grid_sub: Subscription,
    focus_handle: FocusHandle,
}

impl CellPreviewHost {
    pub fn new(data_grid: Entity<DataGrid>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let preview_panel = cx.new(|cx| CellPreviewPanel::new(window, cx));
        let grid_sub = cx.subscribe_in(
            &data_grid,
            window,
            |this, _, event: &DataGridEvent, window, cx| {
                if let DataGridEvent::ToggleLargeTextEditorRequested = event {
                    this.toggle_preview(window, cx);
                }
            },
        );

        Self {
            data_grid,
            preview_panel,
            is_preview_open: false,
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
                this.child(
                    div()
                        .w(PAGE_PREVIEW_WIDTH)
                        .h_full()
                        .flex_shrink_0()
                        .border_l_1()
                        .border_color(cx.theme().border)
                        .child(self.preview_panel.clone()),
                )
            })
    }
}
