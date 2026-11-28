use gpui::{
    App, AsyncApp, Context, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Task, WeakEntity, Window, div,
};
use std::collections::HashSet;
use std::rc::Rc;

use gpui_component::list::{ListDelegate, ListState};
use gpui_component::tooltip::Tooltip;
use gpui_component::{
    ActiveTheme, IndexPath, Selectable, checkbox::Checkbox, h_flex, label::Label,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FilterValueKey {
    Null,
    Text(String),
}

impl FilterValueKey {
    fn element_id(&self) -> String {
        match self {
            Self::Null => "null".to_string(),
            Self::Text(value) => format!("text-{value}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FilterValue {
    pub key: FilterValueKey,
    pub value: String,
    pub count: usize,
    pub checked: bool,
    pub selected: bool,
    /// 用户手动输入、当前结果集里并不存在的值（issue #305）。
    ///
    /// 这类条目没有计数可显示，也不参与「全选」的「所有值」判定。
    pub custom: bool,
}

impl FilterValue {
    pub fn new(value: String, count: usize) -> Self {
        Self {
            key: FilterValueKey::Text(value.clone()),
            value,
            count,
            checked: false,
            selected: false,
            custom: false,
        }
    }

    /// 用户手动输入的筛选值：不在当前结果集里，计数无意义。
    pub fn custom(value: String) -> Self {
        Self {
            key: FilterValueKey::Text(value.clone()),
            value,
            count: 0,
            checked: false,
            selected: false,
            custom: true,
        }
    }

    pub fn null(count: usize) -> Self {
        Self {
            key: FilterValueKey::Null,
            value: "<NULL>".to_string(),
            count,
            checked: false,
            selected: false,
            custom: false,
        }
    }

    pub fn from_key(key: FilterValueKey, count: usize) -> Self {
        match key {
            FilterValueKey::Null => Self::null(count),
            FilterValueKey::Text(value) => Self::new(value, count),
        }
    }
}

#[derive(IntoElement)]
pub struct FilterListItem {
    pub value: Rc<FilterValue>,
    pub selected: bool,
    pub on_toggle: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl FilterListItem {
    pub fn new(value: Rc<FilterValue>, selected: bool) -> Self {
        Self {
            value,
            selected,
            on_toggle: None,
        }
    }

    pub fn on_toggle(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(handler));
        self
    }
}

impl Selectable for FilterListItem {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl RenderOnce for FilterListItem {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let on_toggle = self.on_toggle.clone();
        let value_str = self.value.value.clone();
        let element_id = self.value.key.element_id();
        let checked = self.value.checked;

        h_flex()
            .id(SharedString::from(format!("filter-item-{element_id}")))
            .w_full()
            .px_2()
            .py_1()
            .items_center()
            .justify_between()
            .gap_2()
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                if let Some(handler) = on_toggle.as_ref() {
                    handler(window, cx);
                }
            })
            .child(
                h_flex()
                    .id(SharedString::from(format!("label-{element_id}")))
                    .flex_1()
                    .gap_2()
                    .items_center()
                    .overflow_x_hidden()
                    .child(
                        Checkbox::new(SharedString::from(format!("filter-{element_id}")))
                            .checked(checked),
                    )
                    .child(Label::new(self.value.value.clone()))
                    .tooltip(move |window, cx| Tooltip::new(value_str.clone()).build(window, cx)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.value.custom {
                        // 语言无关的「手动输入」标记，避免在 one_ui 里引入 i18n。
                        "✎".to_string()
                    } else {
                        format!("({})", self.value.count)
                    }),
            )
    }
}

pub struct FilterPanel {
    pub(crate) values: Vec<FilterValue>,
    selected_index: Option<IndexPath>,
    confirmed_index: Option<IndexPath>,
    on_toggle: Option<Rc<dyn Fn(&FilterValueKey, &mut Window, &mut App)>>,
    filtered_values: Vec<FilterValue>,
}

impl FilterPanel {
    pub fn new(values: Vec<FilterValue>) -> Self {
        let filtered_values = values.clone();
        Self {
            values,
            selected_index: None,
            confirmed_index: None,
            on_toggle: None,
            filtered_values,
        }
    }

    pub fn on_toggle(
        mut self,
        handler: impl Fn(&FilterValueKey, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Rc::new(handler));
        self
    }

    pub fn get_selected_values(&self) -> HashSet<FilterValueKey> {
        // 手动输入的自定义值只存在于 `filtered_values` 里，必须一并收集，
        // 否则勾了「beta」也不会真正参与筛选（issue #305）。
        let mut selected: HashSet<FilterValueKey> = self
            .values
            .iter()
            .filter(|value| value.selected)
            .map(|value| value.key.clone())
            .collect();
        selected.extend(
            self.filtered_values
                .iter()
                .filter(|value| value.selected && value.custom)
                .map(|value| value.key.clone()),
        );
        selected
    }

    pub fn toggle_value(&mut self, key: &FilterValueKey) {
        if let Some(v) = self.values.iter_mut().find(|v| &v.key == key) {
            v.selected = !v.selected;
            v.checked = v.selected;
        }

        if let Some(v) = self.filtered_values.iter_mut().find(|v| &v.key == key) {
            v.selected = !v.selected;
            v.checked = v.selected;
        }
    }

    pub fn select_all(&mut self) {
        let visible_values: HashSet<FilterValueKey> =
            self.filtered_values.iter().map(|v| v.key.clone()).collect();

        for v in &mut self.values {
            if visible_values.contains(&v.key) {
                v.selected = true;
                v.checked = true;
            }
        }

        for v in &mut self.filtered_values {
            v.selected = true;
            v.checked = true;
        }
    }

    pub fn deselect_all(&mut self) {
        let visible_values: HashSet<FilterValueKey> =
            self.filtered_values.iter().map(|v| v.key.clone()).collect();

        for v in &mut self.values {
            if visible_values.contains(&v.key) {
                v.selected = false;
                v.checked = false;
            }
        }

        for v in &mut self.filtered_values {
            v.selected = false;
            v.checked = false;
        }
    }

    fn update_filtered_values(&mut self, search_query: String) {
        if search_query.trim().is_empty() {
            self.filtered_values = self.values.clone();
        } else {
            let query_lower = search_query.to_lowercase();
            self.filtered_values = self
                .values
                .iter()
                .filter(|v| v.value.to_lowercase().contains(&query_lower))
                .cloned()
                .collect();
            // issue #305：结果集里没有这个值时，允许用户直接把输入的文本当筛选值用，
            // 否则只能从「已加载到的值」里选，筛选前几页之外的枚举值根本选不到。
            let trimmed = search_query.trim();
            if !trimmed.is_empty()
                && !self
                    .filtered_values
                    .iter()
                    .any(|value| value.key == FilterValueKey::Text(trimmed.to_string()))
            {
                self.filtered_values
                    .push(FilterValue::custom(trimmed.to_string()));
            }
        }
    }

    pub fn filtered_values(&self) -> &[FilterValue] {
        &self.filtered_values
    }

    /// 把「当前筛选中已生效、但当前结果集里查不到」的值补进列表。
    ///
    /// 否则用户手动输入一个值筛选后，重开筛选面板会因为列表里没有它而
    /// 无法取消勾选，只能清空整列筛选（issue #305 的后续问题）。
    pub fn with_active_custom_keys(mut self, selected_keys: &HashSet<FilterValueKey>) -> Self {
        for key in selected_keys {
            if self.values.iter().any(|value| &value.key == key) {
                continue;
            }
            let mut value = FilterValue::from_key(key.clone(), 0);
            value.custom = true;
            value.selected = true;
            value.checked = true;
            self.values.push(value.clone());
            self.filtered_values.push(value);
        }
        self
    }
}

impl ListDelegate for FilterPanel {
    type Item = FilterListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        let query = query.to_string();
        cx.spawn(
            async move |entity: WeakEntity<ListState<Self>>, cx: &mut AsyncApp| {
                let result = entity.update(cx, move |this, _cx| {
                    this.delegate_mut().update_filtered_values(query);
                });
                result.unwrap_or_else(|_| {
                    eprint!("Failed to update search query");
                })
            },
        )
    }

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.filtered_values.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let selected = Some(ix) == self.selected_index || Some(ix) == self.confirmed_index;
        if let Some(value) = self.filtered_values.get(ix.row) {
            let value_rc = Rc::from(value.clone());
            let mut item = FilterListItem::new(value_rc.clone(), selected);

            if let Some(on_toggle) = self.on_toggle.as_ref() {
                let on_toggle = on_toggle.clone();
                let value_key = value.key.clone();
                item = item.on_toggle(move |window, cx| {
                    on_toggle(&value_key, window, cx);
                });
            }

            return Some(item);
        }
        None
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = ix;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(values: Vec<FilterValue>) -> FilterPanel {
        FilterPanel::new(values)
    }

    #[test]
    fn searching_a_missing_value_offers_it_as_a_custom_entry() {
        // issue #305：结果集里只有已加载的行，筛选前几页之外的值必须能手动输入。
        let mut panel = panel(vec![FilterValue::new("alpha".to_string(), 2)]);

        panel.update_filtered_values("beta".to_string());

        let custom = panel
            .filtered_values()
            .iter()
            .find(|value| value.custom)
            .expect("custom entry");
        assert_eq!(custom.key, FilterValueKey::Text("beta".to_string()));
        assert_eq!(0, custom.count);
    }

    #[test]
    fn searching_a_known_value_does_not_add_a_duplicate_custom_entry() {
        let mut panel = panel(vec![FilterValue::new("alpha".to_string(), 2)]);

        panel.update_filtered_values("alpha".to_string());

        assert!(!panel.filtered_values().iter().any(|value| value.custom));
        assert_eq!(1, panel.filtered_values().len());
    }

    #[test]
    fn custom_entries_can_be_toggled_like_listed_values() {
        let mut panel = panel(vec![FilterValue::new("alpha".to_string(), 2)]);
        panel.update_filtered_values("beta".to_string());

        panel.toggle_value(&FilterValueKey::Text("beta".to_string()));

        assert!(
            panel
                .get_selected_values()
                .contains(&FilterValueKey::Text("beta".to_string()))
        );
    }

    #[test]
    fn reopening_the_panel_keeps_active_custom_values_visible() {
        // #305 后续：手动输入的值筛选后，重开面板必须还能看到并取消勾选。
        let mut keys = HashSet::new();
        keys.insert(FilterValueKey::Text("beta".to_string()));

        let panel =
            panel(vec![FilterValue::new("alpha".to_string(), 2)]).with_active_custom_keys(&keys);

        let restored = panel
            .filtered_values()
            .iter()
            .find(|value| value.key == FilterValueKey::Text("beta".to_string()))
            .expect("custom value kept");
        assert!(restored.custom);
        assert!(restored.selected);
        assert!(
            panel
                .get_selected_values()
                .contains(&FilterValueKey::Text("beta".to_string()))
        );
    }

    #[test]
    fn active_keys_already_present_are_not_duplicated() {
        let mut keys = HashSet::new();
        keys.insert(FilterValueKey::Text("alpha".to_string()));

        let panel =
            panel(vec![FilterValue::new("alpha".to_string(), 2)]).with_active_custom_keys(&keys);

        assert_eq!(1, panel.filtered_values().len());
        assert!(!panel.filtered_values()[0].custom);
    }

    #[test]
    fn blank_search_query_keeps_the_plain_value_list() {
        let mut panel = panel(vec![FilterValue::new("alpha".to_string(), 2)]);

        panel.update_filtered_values("   ".to_string());

        assert_eq!(1, panel.filtered_values().len());
    }
}
