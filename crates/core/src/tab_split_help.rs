use gpui::{Context, Window};
use gpui_component::IconName;
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use rust_i18n::t;

pub(crate) struct TerminalSplitHelp {
    supported: bool,
}

impl TerminalSplitHelp {
    pub(crate) fn new(supported: bool) -> Self {
        Self { supported }
    }

    pub(crate) fn append(
        self,
        menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let status = if self.supported {
            t!("TabContextMenu.split_supported").to_string()
        } else {
            t!("TabContextMenu.split_not_supported").to_string()
        };
        menu.submenu_with_icon(
            Some(IconName::Info.into()),
            t!("TabContextMenu.split_help").to_string(),
            window,
            cx,
            move |submenu, _, _| {
                submenu
                    .item(PopupMenuItem::new(status.clone()).disabled(true))
                    .separator()
                    .item(
                        PopupMenuItem::new(t!("TabContextMenu.split_only_terminal")).disabled(true),
                    )
                    .item(PopupMenuItem::new(t!("TabContextMenu.split_drag")).disabled(true))
                    .item(PopupMenuItem::new(t!("TabContextMenu.split_directions")).disabled(true))
                    .item(PopupMenuItem::new(t!("TabContextMenu.split_limit")).disabled(true))
                    .item(PopupMenuItem::new(t!("TabContextMenu.split_cancel")).disabled(true))
            },
        )
    }
}
