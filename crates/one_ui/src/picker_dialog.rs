//! 主机/端点选择弹窗：搜索、上下键选择、回车确认，列表自带滚动。
//!
//! 抽出来的动机是"选主机"这件事在两个地方都要做：SFTP 的端点切换、终端文件
//! 面板的远端目标。两边原本各写一份下拉，一份没有滚动条，配色也各自为政。
//! 这里统一成一个弹窗组件，调用方只管给出候选和回调。

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, RenderOnce, SharedString, Styled as _, Task, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::list::{List, ListDelegate, ListState};
use gpui_component::{
    ActiveTheme, Icon, IconName, IndexPath, Selectable, Sizable, Size, WindowExt as _, h_flex,
};

use crate::theme_geometry;

const DIALOG_WIDTH: f32 = 520.0;
const DIALOG_MAX_HEIGHT: f32 = 420.0;

/// 一行候选。`value` 是回吐给调用方的载荷，其余字段只用于展示。
#[derive(Clone)]
pub struct PickerEntry<V> {
    /// 行元素 id，需在本次列表内唯一。
    pub id: SharedString,
    pub value: V,
    pub title: SharedString,
    /// 行尾的次要说明（如 `user@host:22`）。可以没有，右对齐显示。
    pub subtitle: Option<SharedString>,
    /// 标题旁的小标签（如"终端"）。
    pub badge: Option<SharedString>,
    /// 图标；mono/color 模式由调用方按需要设好。
    pub icon: Icon,
    /// 当前项：常亮高亮并显示勾选。
    pub active: bool,
}

/// 弹窗文案，由调用方从自己的 locale 里取。
pub struct PickerDialogLabels {
    pub title: SharedString,
    pub search_placeholder: SharedString,
    /// 没有任何候选（含搜索无结果）时显示。
    pub empty: SharedString,
}

/// 打开选择弹窗。
///
/// 选中（点击或回车）后先调用 `on_pick`，再关闭弹窗——回调里不要再关一次。
pub fn open_picker_dialog<V, F>(
    labels: PickerDialogLabels,
    entries: Vec<PickerEntry<V>>,
    window: &mut Window,
    cx: &mut App,
    on_pick: F,
) where
    V: Clone + 'static,
    F: Fn(V, &mut Window, &mut App) + 'static,
{
    let PickerDialogLabels {
        title,
        search_placeholder,
        empty,
    } = labels;
    let active_row = entries.iter().position(|entry| entry.active).unwrap_or(0);
    let delegate = PickerDelegate {
        filtered: entries.clone(),
        entries,
        selected_index: None,
        empty,
        on_pick: Rc::new(on_pick),
    };
    let list = cx.new(|cx| {
        let mut list = ListState::new(delegate, window, cx).searchable(true);
        list.set_selected_index(Some(IndexPath::new(active_row)), window, cx);
        list
    });
    let dialog_list = list.clone();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let search_placeholder = search_placeholder.clone();
        dialog
            .w(px(DIALOG_WIDTH))
            .margin_top(px(72.0))
            .close_button(false)
            .title(title.clone())
            .content({
                let list = dialog_list.clone();
                move |content, _window, _cx| {
                    content.p_0().child(
                        div().id("picker-dialog").child(
                            List::new(&list)
                                .search_placeholder(search_placeholder.clone())
                                .with_size(Size::Large)
                                .max_h(px(DIALOG_MAX_HEIGHT)),
                        ),
                    )
                }
            })
    });
    list.update(cx, |list, cx| list.focus(window, cx));
}

/// 标题/副标题的大小写不敏感包含匹配；空查询返回全部。
pub(crate) fn filter_picker_entries<V: Clone>(
    entries: &[PickerEntry<V>],
    query: &str,
) -> Vec<PickerEntry<V>> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return entries.to_vec();
    }
    entries
        .iter()
        .filter(|entry| {
            entry.title.to_lowercase().contains(&query)
                || entry
                    .subtitle
                    .as_ref()
                    .is_some_and(|subtitle| subtitle.to_lowercase().contains(&query))
        })
        .cloned()
        .collect()
}

struct PickerDelegate<V: Clone + 'static> {
    entries: Vec<PickerEntry<V>>,
    filtered: Vec<PickerEntry<V>>,
    selected_index: Option<IndexPath>,
    empty: SharedString,
    on_pick: Rc<dyn Fn(V, &mut Window, &mut App)>,
}

impl<V: Clone + 'static> ListDelegate for PickerDelegate<V> {
    type Item = PickerItem;

    fn perform_search(
        &mut self,
        query: &str,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.filtered = filter_picker_entries(&self.entries, query);
        cx.notify();
        Task::ready(())
    }

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.filtered.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let entry = self.filtered.get(ix.row)?.clone();
        let on_pick = self.on_pick.clone();
        let value = entry.value.clone();
        Some(PickerItem::new(
            entry,
            self.selected_index == Some(ix),
            move |window, cx| on_pick(value.clone(), window, cx),
        ))
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = ix;
    }

    fn confirm(&mut self, _secondary: bool, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        let Some(ix) = self.selected_index else {
            return;
        };
        let Some(entry) = self.filtered.get(ix.row) else {
            return;
        };
        let on_pick = self.on_pick.clone();
        let value = entry.value.clone();
        on_pick(value, window, cx);
        window.close_dialog(cx);
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        window.close_dialog(cx);
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        h_flex()
            .size_full()
            .justify_center()
            .py_3()
            .text_color(cx.theme().muted_foreground)
            .child(self.empty.clone())
    }
}

#[derive(IntoElement)]
struct PickerItem {
    id: SharedString,
    title: SharedString,
    subtitle: Option<SharedString>,
    badge: Option<SharedString>,
    icon: Icon,
    active: bool,
    selected: bool,
    on_pick: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl PickerItem {
    fn new<V: Clone + 'static>(
        entry: PickerEntry<V>,
        selected: bool,
        on_pick: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: entry.id,
            title: entry.title,
            subtitle: entry.subtitle,
            badge: entry.badge,
            icon: entry.icon,
            active: entry.active,
            selected,
            on_pick: Rc::new(on_pick),
        }
    }
}

impl Selectable for PickerItem {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl RenderOnce for PickerItem {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let geometry = theme_geometry().clone();
        let on_pick = self.on_pick.clone();
        // 当前项和键盘选中的行一样常亮，用户才知道面板正连着谁。
        let highlighted = self.selected || self.active;
        let muted_foreground = cx.theme().muted_foreground;

        h_flex()
            .id(self.id)
            .h(geometry.control.xlarge)
            .mx_2()
            .px(geometry.spacing.space_3)
            .rounded(geometry.radius.sm)
            .items_center()
            .gap_3()
            .cursor_pointer()
            .text_color(cx.theme().foreground)
            .when(highlighted, |el| el.bg(cx.theme().list_active))
            .when(!highlighted, |el| {
                el.text_color(muted_foreground)
                    .hover(|style| style.bg(cx.theme().list_hover))
            })
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                on_pick(window, cx);
                window.close_dialog(cx);
            })
            .child(self.icon.with_size(crate::IconSize::Large))
            .child(
                // 单行：名称和标签贴在一起占满剩余空间，副标题右对齐，
                // 和"快捷打开"（标签页切换）的列表行是同一种排法。
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .child(self.title.clone()),
                    )
                    .when_some(self.badge.clone(), |el, badge| {
                        el.child(
                            div()
                                .flex_shrink_0()
                                .px_1()
                                .rounded_sm()
                                .bg(cx.theme().muted)
                                .text_xs()
                                .text_color(muted_foreground)
                                .child(badge),
                        )
                    }),
            )
            .when_some(self.subtitle.clone(), |el, subtitle| {
                el.child(
                    div()
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(muted_foreground)
                        .child(subtitle),
                )
            })
            .when(self.active, |el| {
                el.child(Icon::new(IconName::Check).small().text_color(muted_foreground))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{PickerEntry, filter_picker_entries};
    use gpui::SharedString;
    use gpui_component::Icon;

    fn entry(title: &str, subtitle: Option<&str>) -> PickerEntry<i64> {
        PickerEntry {
            id: SharedString::from(title.to_string()),
            value: 0,
            title: SharedString::from(title.to_string()),
            subtitle: subtitle.map(|subtitle| SharedString::from(subtitle.to_string())),
            badge: None,
            icon: Icon::default(),
            active: false,
        }
    }

    #[test]
    fn filtering_is_case_insensitive_and_covers_subtitle() {
        let entries = vec![
            entry("Production", Some("deploy@prod.internal:22")),
            entry("Staging", None),
        ];

        assert_eq!(2, filter_picker_entries(&entries, "  ").len());
        assert_eq!(1, filter_picker_entries(&entries, "prod").len());
        assert_eq!(1, filter_picker_entries(&entries, "PROD.INTERNAL").len());
        assert!(filter_picker_entries(&entries, "staging").len() == 1);
        assert!(filter_picker_entries(&entries, "nowhere").is_empty());
    }
}
