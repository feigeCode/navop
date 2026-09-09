use gpui::{Pixels, px};

#[derive(Clone, Copy)]
pub(crate) struct Spacing {
    pub space_1: Pixels,
    pub space_2: Pixels,
    pub space_3: Pixels,
    pub space_6: Pixels,
}

#[derive(Clone, Copy)]
pub(crate) struct Layout {
    pub panel_header: Pixels,
    pub embedded_panel_header: Pixels,
    pub dock_panel_header: Pixels,
    pub command_bar: Pixels,
    pub status_bar: Pixels,
}

#[derive(Clone, Copy)]
pub(crate) struct Resize {
    pub edge_padding: Pixels,
    pub visible_line: Pixels,
}

pub(crate) fn spacing() -> Spacing {
    Spacing {
        space_1: px(4.),
        space_2: px(8.),
        space_3: px(12.),
        space_6: px(24.),
    }
}

pub(crate) fn layout() -> Layout {
    Layout {
        panel_header: px(36.),
        embedded_panel_header: px(40.),
        dock_panel_header: px(30.),
        command_bar: px(36.),
        status_bar: px(28.),
    }
}

pub(crate) fn resize() -> Resize {
    Resize {
        edge_padding: px(4.),
        visible_line: px(1.),
    }
}
