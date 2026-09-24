use crate::{SftpView, endpoint::LeftEndpointValue};
use gpui::{App, Entity, SharedString, Window};
use gpui_component::Icon;
use one_assets::IconName;
use rust_i18n::t;

/// 候选项：`value` 决定切到哪，其余字段只用于展示。
#[derive(Clone)]
pub(crate) struct EndpointSwitcherEntry {
    pub value: LeftEndpointValue,
    pub title: SharedString,
    /// `user@host:port`，与终端文件面板的目标选择器同款。
    pub subtitle: Option<SharedString>,
    pub icon: IconName,
    pub active: bool,
}

pub(crate) fn open_endpoint_switcher_dialog(
    view: Entity<SftpView>,
    entries: Vec<EndpointSwitcherEntry>,
    window: &mut Window,
    cx: &mut App,
) {
    // 列表弹窗本体在 one_ui：终端文件面板的远端目标选择器用的是同一个。
    let entries = entries
        .into_iter()
        .map(|entry| one_ui::PickerEntry {
            id: entry_row_id(&entry.value),
            value: entry.value,
            title: entry.title,
            subtitle: entry.subtitle,
            badge: None,
            icon: Icon::new(entry.icon),
            active: entry.active,
        })
        .collect();

    one_ui::open_picker_dialog(
        one_ui::PickerDialogLabels {
            title: t!("Endpoint.switch_title").to_string().into(),
            search_placeholder: t!("Endpoint.search").to_string().into(),
            empty: t!("Endpoint.no_results").to_string().into(),
        },
        entries,
        window,
        cx,
        move |value, window, cx| {
            view.update(cx, |view, cx| view.switch_left_endpoint(value, window, cx));
        },
    );
}

fn entry_row_id(value: &LeftEndpointValue) -> SharedString {
    match value {
        LeftEndpointValue::Local => "endpoint-local".into(),
        LeftEndpointValue::Remote(id) => format!("endpoint-remote-{id}").into(),
    }
}
