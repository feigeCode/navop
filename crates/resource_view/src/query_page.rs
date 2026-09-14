//! Query 页面输入状态。
//!
//! 输入草稿由独立 Entity 持有:页面切换不丢稿,按键通知不重绘整个工作台
//! (遵循仓库“输入面板独立 Entity”的沉淀经验)。
//!
//! 字段控件按 manifest `editor` 选择(`ResourceWorkbenchInput.editor`):
//! - `select`(且声明了 `options`)→ 下拉选择;选中值存在 `SelectState` 里,
//!   默认选中 `default` 命中的候选项(缺省第一项),与 query 页「必填字段总有值」
//!   的语义一致;
//! - 其余(含 `text`/`number`/`password`/`textarea` 等)沿用文本输入:
//!   `values()` 读回的都是字符串,类型由工作台 `coerce_binding` 按声明强制转换,
//!   所以 `number` 字段不需要单独的控件。
//!
//! 渲染用 `label`(缺省回落字段 id)与 `description`,与清单声明保持一致。

use std::collections::BTreeMap;

use extension_runtime::extension::manifest::{ResourceWorkbenchInput, ResourceWorkbenchPage};
use gpui::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, IndexPath,
    input::{Input, InputState},
    select::{SearchableVec, Select, SelectItem, SelectState},
    v_flex,
};

/// `select` 编辑器的一个候选项。
#[derive(Clone, PartialEq)]
pub struct QueryOptionItem {
    value: String,
    label: SharedString,
}

impl QueryOptionItem {
    fn new(value: impl Into<String>, label: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

impl SelectItem for QueryOptionItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

/// 单字段的控件状态。
enum QueryFieldControl {
    /// 文本类字段(值即 `InputState` 文本)。
    Text(Entity<InputState>),
    /// 下拉字段。
    Select(Entity<SelectState<SearchableVec<QueryOptionItem>>>),
}

/// 一个已初始化的输入字段。
struct QueryField {
    id: String,
    label: SharedString,
    description: Option<SharedString>,
    control: QueryFieldControl,
}

/// 该字段是否渲染为下拉(`editor = "select"` 且声明了候选项)。
fn is_select_input(input: &ResourceWorkbenchInput) -> bool {
    input.editor == "select" && !input.options.is_empty()
}

/// 下拉字段的默认选中下标:命中 `default` 的候选项,否则第一项。
fn select_initial_index(input: &ResourceWorkbenchInput) -> usize {
    let initial = input.default.as_deref().unwrap_or_default();
    input
        .options
        .iter()
        .position(|option| option.value == initial)
        .unwrap_or(0)
}

/// 字段显示名:`label`,缺省回落字段 id。
fn field_label(input: &ResourceWorkbenchInput) -> String {
    match input.label.as_deref() {
        Some(label) if !label.trim().is_empty() => label.to_string(),
        _ => input.id.clone(),
    }
}

/// 建一个文本类字段 state(`placeholder` 与 `default` 都来自清单声明)。
fn text_field(
    initial: String,
    placeholder: Option<String>,
    window: &mut Window,
    cx: &mut Context<QueryInputState>,
) -> Entity<InputState> {
    cx.new(move |cx| {
        let mut state = InputState::new(window, cx);
        if let Some(placeholder) = placeholder {
            state = state.placeholder(placeholder);
        }
        if !initial.is_empty() {
            state.set_value(initial, window, cx);
        }
        state
    })
}

/// 建一个下拉字段 state(候选项与默认选中项都来自清单声明)。
fn select_field(
    input: &ResourceWorkbenchInput,
    window: &mut Window,
    cx: &mut Context<QueryInputState>,
) -> Entity<SelectState<SearchableVec<QueryOptionItem>>> {
    let items = input
        .options
        .iter()
        .map(|option| QueryOptionItem::new(option.value.clone(), option.label.clone()))
        .collect::<Vec<_>>();
    let selected = select_initial_index(input);
    cx.new(move |cx| {
        SelectState::new(
            SearchableVec::new(items),
            Some(IndexPath::new(selected)),
            window,
            cx,
        )
    })
}

pub struct QueryInputState {
    fields: Vec<QueryField>,
    focus_handle: FocusHandle,
}

impl QueryInputState {
    pub fn new(page: &ResourceWorkbenchPage, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut fields = Vec::new();
        for input in &page.inputs {
            let control = if is_select_input(input) {
                QueryFieldControl::Select(select_field(input, window, cx))
            } else {
                QueryFieldControl::Text(text_field(
                    input.default.clone().unwrap_or_default(),
                    input.placeholder.clone(),
                    window,
                    cx,
                ))
            };
            fields.push(QueryField {
                id: input.id.clone(),
                label: field_label(input).into(),
                description: input
                    .description
                    .as_deref()
                    .map(str::trim)
                    .filter(|description| !description.is_empty())
                    .map(SharedString::from),
                control,
            });
        }
        Self {
            fields,
            focus_handle: cx.focus_handle(),
        }
    }

    /// 当前所有输入值(按字段 id)。下拉字段给出候选项 `value`,文本字段给出文本。
    pub fn values(&self, cx: &App) -> BTreeMap<String, String> {
        self.fields
            .iter()
            .map(|field| {
                let value = match &field.control {
                    QueryFieldControl::Text(state) => state.read(cx).text().to_string(),
                    QueryFieldControl::Select(state) => state
                        .read(cx)
                        .selected_value()
                        .cloned()
                        .unwrap_or_default(),
                };
                (field.id.clone(), value)
            })
            .collect()
    }
}

impl Focusable for QueryInputState {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for QueryInputState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        div().flex().flex_col().gap_2().children(
            self.fields
                .iter()
                .map(|field| {
                    let control: AnyElement = match &field.control {
                        QueryFieldControl::Text(state) => Input::new(state).into_any_element(),
                        QueryFieldControl::Select(state) => Select::new(state).into_any_element(),
                    };
                    let mut control_column = v_flex().flex_1().min_w_0().gap_1().child(control);
                    if let Some(description) = &field.description {
                        control_column = control_column.child(
                            div()
                                .text_size(px(11.))
                                .text_color(muted)
                                .child(description.clone()),
                        );
                    }
                    div()
                        .flex()
                        .flex_row()
                        .items_start()
                        .gap_2()
                        .child(div().w(px(120.)).flex_shrink_0().child(field.label.clone()))
                        .child(control_column)
                })
                .collect::<Vec<_>>(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension_runtime::extension::manifest::ResourceWorkbenchInputOption;

    fn input(
        id: &str,
        editor: &str,
        default: Option<&str>,
        options: &[(&str, &str)],
    ) -> ResourceWorkbenchInput {
        ResourceWorkbenchInput {
            id: id.to_string(),
            value_type: "string".to_string(),
            editor: editor.to_string(),
            default: default.map(str::to_string),
            required: false,
            label: None,
            placeholder: None,
            description: None,
            options: options
                .iter()
                .map(|(value, label)| ResourceWorkbenchInputOption {
                    value: (*value).to_string(),
                    label: (*label).to_string(),
                })
                .collect(),
            rows: None,
        }
    }

    const QOS: &[(&str, &str)] = &[("0", "QoS 0"), ("1", "QoS 1"), ("2", "QoS 2")];

    #[test]
    fn select_editor_requires_options() {
        // 声明 select 但没有候选项:回落文本输入,避免渲染一个空下拉
        assert!(!is_select_input(&input("qos", "select", None, &[])));
        assert!(is_select_input(&input("qos", "select", None, QOS)));
        assert!(!is_select_input(&input("topic", "text", None, QOS)));
    }

    #[test]
    fn select_defaults_to_the_matching_option() {
        assert_eq!(1, select_initial_index(&input("qos", "select", Some("1"), QOS)));
        assert_eq!(2, select_initial_index(&input("qos", "select", Some("2"), QOS)));
        // 缺省或未命中:取第一项(必填字段总有值,与查询前置校验一致)
        assert_eq!(0, select_initial_index(&input("qos", "select", None, QOS)));
        assert_eq!(0, select_initial_index(&input("qos", "select", Some("9"), QOS)));
    }

    #[test]
    fn field_label_falls_back_to_id() {
        assert_eq!("qos", field_label(&input("qos", "text", None, &[])));
        let mut labelled = input("qos", "text", None, &[]);
        labelled.label = Some("  QoS  ".to_string());
        assert_eq!("  QoS  ", field_label(&labelled));
        let mut blank = input("qos", "text", None, &[]);
        blank.label = Some("   ".to_string());
        assert_eq!("qos", field_label(&blank));
    }
}
