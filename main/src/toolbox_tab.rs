//! Built-in tools use a registry so contributions do not change the home menu.
//! Extension tools (`contributes.shellViews` with `surface: "toolbox"`) are
//! appended under their categories; connection-style extensions never appear
//! here — they belong to `contributes.connections`.
use crate::{home_tab::HomePage, navigation_applications::NavigationApplication};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, ColorExt as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, IconName, Sizable, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;

pub(crate) struct ToolEntry {
    pub id: &'static str,
    pub title: SharedString,
    pub description: SharedString,
    pub icon: IconName,
    pub launch: ToolLaunch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolLaunch {
    OpenTab(NavigationApplication),
}

/// 扩展贡献的 toolbox shell 视图（surface: "toolbox"）。
pub(crate) struct ExtensionTool {
    pub extension_id: String,
    pub view_id: String,
    pub title: String,
    pub description: Option<String>,
    pub category: String,
    pub keywords: Vec<String>,
}

impl ExtensionTool {
    /// title/description/keywords 对搜索词的大小写不敏感匹配。
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        self.title.to_lowercase().contains(&query)
            || self
                .description
                .as_deref()
                .is_some_and(|description| description.to_lowercase().contains(&query))
            || self
                .keywords
                .iter()
                .any(|keyword| keyword.to_lowercase().contains(&query))
    }
}

/// 从全局 runtime catalog 读取 toolbox surface 的 shell 视图。
pub(crate) fn extension_tools(cx: &App) -> Vec<ExtensionTool> {
    let Some(catalog) = extension_runtime::global_catalog(cx) else {
        return Vec::new();
    };
    catalog
        .toolbox_views()
        .into_iter()
        .map(|view| ExtensionTool {
            extension_id: view.extension_id.clone(),
            view_id: view.id.clone(),
            title: view.title.clone(),
            description: view.description.clone(),
            category: view.category.clone().unwrap_or_else(|| "general".into()),
            keywords: view.keywords.clone(),
        })
        .collect()
}

pub(crate) fn registered_tools(_cx: &App) -> Vec<ToolEntry> {
    builtin_tools()
}

fn builtin_tools() -> Vec<ToolEntry> {
    vec![ToolEntry {
        id: "json-formatter",
        title: t!("Home.json_formatter").into(),
        description: t!("Home.toolbox_json_description").into(),
        icon: IconName::Json,
        launch: ToolLaunch::OpenTab(NavigationApplication::JsonFormatter),
    }]
}

pub(crate) struct ToolboxTab {
    home: Entity<HomePage>,
    focus_handle: FocusHandle,
    search_input: Entity<InputState>,
    tool_search: String,
}
impl ToolboxTab {
    pub(crate) fn new(home: Entity<HomePage>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索工具…")
                .clean_on_escape()
        });
        cx.subscribe(&search_input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.tool_search = input.read(cx).value().to_string();
                cx.notify();
            }
        })
        .detach();
        Self {
            home,
            focus_handle: cx.focus_handle(),
            search_input,
            tool_search: String::new(),
        }
    }
}
impl Focusable for ToolboxTab {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
impl EventEmitter<TabContentEvent> for ToolboxTab {}
impl TabContent for ToolboxTab {
    fn content_key(&self) -> &'static str {
        "Toolbox"
    }
    fn title(&self, _cx: &App) -> SharedString {
        t!("Home.toolbox").into()
    }
    fn icon(&self, _cx: &App) -> Option<gpui_component::Icon> {
        Some(IconName::LayoutDashboard.into())
    }
}
impl ToolboxTab {
    /// 内置工具卡片。
    fn render_tool_card(&self, tool: ToolEntry, cx: &Context<Self>) -> impl IntoElement {
        let home = self.home.clone();
        let accent = cx.theme().accent;
        let accent_soft = cx.theme().accent.opacity(0.12);
        v_flex()
            .id(SharedString::from(format!("tool-{}", tool.id)))
            .w(gpui::rems(18.0))
            .min_h(gpui::rems(7.5))
            .flex_shrink_0()
            .p_3()
            .gap_2()
            .rounded(px(8.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .cursor_pointer()
            .hover(move |style| style.border_color(accent))
            .child(
                div()
                    .w(px(32.0))
                    .h(px(32.0))
                    .rounded(px(8.0))
                    .bg(accent_soft)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        gpui_component::Icon::new(tool.icon)
                            .with_size(gpui_component::IconSize::Large)
                            .text_color(accent),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(tool.title),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tool.description),
            )
            .on_click(move |_, window, cx| {
                let ToolLaunch::OpenTab(application) = tool.launch;
                home.update(cx, |home, cx| {
                    home.activate_navigation_application(application, window, cx)
                });
            })
    }

    /// 扩展工具卡片（surface: toolbox 的 shell 视图）。
    fn render_extension_tool_card(
        &self,
        tool: &ExtensionTool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let accent = cx.theme().accent;
        let accent_soft = cx.theme().accent.opacity(0.12);
        let (extension_id, view_id) = (tool.extension_id.clone(), tool.view_id.clone());
        v_flex()
            .id(SharedString::from(format!(
                "ext-tool-{}-{}",
                tool.extension_id, tool.view_id
            )))
            .w(gpui::rems(18.0))
            .min_h(gpui::rems(7.5))
            .flex_shrink_0()
            .p_3()
            .gap_2()
            .rounded(px(8.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .cursor_pointer()
            .hover(move |style| style.border_color(accent))
            .child(
                div()
                    .w(px(32.0))
                    .h(px(32.0))
                    .rounded(px(8.0))
                    .bg(accent_soft)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        gpui_component::Icon::new(IconName::TableDesignTool)
                            .with_size(gpui_component::IconSize::Large)
                            .text_color(accent),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(tool.title.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{}  ·  {}",
                        tool.category,
                        tool.description.clone().unwrap_or_default()
                    )),
            )
            .on_click(move |_, window, cx| {
                extension_view::open_shell_view(&extension_id, &view_id, window, cx);
            })
    }

    fn filtered_extension_tools(&self, cx: &Context<Self>) -> Vec<ExtensionTool> {
        extension_tools(cx)
            .into_iter()
            .filter(|tool| tool.matches(&self.tool_search))
            .collect()
    }

    /// 「更多工具」占位卡（demo：虚线边框、无阴影、居中）。
    fn render_ghost_card(&self, cx: &Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .w(gpui::rems(18.0))
            .min_h(gpui::rems(7.5))
            .flex_shrink_0()
            .p_4()
            .gap_2p5()
            .rounded(px(8.0))
            .border_1()
            .border_dashed()
            .border_color(cx.theme().border)
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(38.0))
                    .h(px(38.0))
                    .rounded(px(10.0))
                    .bg(cx.theme().muted)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        gpui_component::Icon::new(IconName::Plus)
                            .with_size(gpui_component::IconSize::Large)
                            .text_color(muted),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child(t!("Home.toolbox_more_tools").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(t!("Home.toolbox_more_hint").to_string()),
            )
    }
}

impl Render for ToolboxTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.tool_search.trim().to_lowercase();
        let builtin_tools = registered_tools(cx)
            .into_iter()
            .filter(|tool| {
                query.is_empty()
                    || tool.title.to_lowercase().contains(&query)
                    || tool.description.to_lowercase().contains(&query)
                    || tool.id.contains(&query)
            })
            .collect::<Vec<_>>();
        let extension_tools = self.filtered_extension_tools(cx);
        let has_results = !builtin_tools.is_empty() || !extension_tools.is_empty();
        let mut tools = h_flex().w_full().flex_wrap().gap_3();
        for tool in builtin_tools {
            tools = tools.child(self.render_tool_card(tool, cx));
        }
        for tool in &extension_tools {
            tools = tools.child(self.render_extension_tool_card(tool, cx));
        }
        if query.is_empty() {
            tools = tools.child(self.render_ghost_card(cx));
        }
        v_flex()
            .id("toolbox-content")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .px_8()
            .py_7()
            .bg(cx.theme().background)
            .child(
                div()
                    .text_xl()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .mb_2()
                    .child(t!("Home.toolbox").to_string()),
            )
            .child(
                h_flex()
                    .w_full()
                    .mb_6()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("Home.toolbox_description").to_string()),
                    )
                    .child(Input::new(&self.search_input).w(gpui::rems(18.0)).small()),
            )
            .when(has_results || query.is_empty(), |container| {
                container.child(tools)
            })
            .when(!has_results && !query.is_empty(), |container| {
                container.child(
                    v_flex()
                        .w_full()
                        .py_12()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("未找到工具"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("尝试搜索名称、描述、分类或关键词。"),
                        ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtin_registry_routes_json_to_the_existing_deduplicated_application() {
        let tools = builtin_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].id, "json-formatter");
        assert_eq!(
            tools[0].launch,
            ToolLaunch::OpenTab(NavigationApplication::JsonFormatter)
        );
    }

    #[test]
    fn extension_tool_matches_searches_title_description_and_keywords() {
        let tool = ExtensionTool {
            extension_id: "com.example".into(),
            view_id: "hosts".into(),
            title: "Hosts Editor".into(),
            description: Some("Edit /etc/hosts".into()),
            category: "system".into(),
            keywords: vec!["dns".into(), "resolve".into()],
        };
        assert!(tool.matches(""));
        assert!(tool.matches("HOSTS"));
        assert!(tool.matches("etc"));
        assert!(tool.matches("DNS"));
        assert!(!tool.matches("docker"));
    }
}
