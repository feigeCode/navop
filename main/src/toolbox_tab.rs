//! Built-in tools use a registry so contributions do not change the home menu.
use crate::{home_tab::HomePage, navigation_applications::NavigationApplication};
use gpui::{
    App, ColorExt as _, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, px,
};
use gpui_component::{ActiveTheme, IconName, Sizable, h_flex, v_flex};
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
}
impl ToolboxTab {
    pub(crate) fn new(home: Entity<HomePage>, cx: &mut Context<Self>) -> Self {
        Self {
            home,
            focus_handle: cx.focus_handle(),
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
    /// 工具卡片（demo：圆角 12px、padding 16px、图标 38×38 accent-soft 底）。
    fn render_tool_card(&self, tool: ToolEntry, cx: &Context<Self>) -> impl IntoElement {
        let home = self.home.clone();
        let accent = cx.theme().accent;
        let accent_soft = cx.theme().accent.opacity(0.12);
        v_flex()
            .id(SharedString::from(format!("tool-{}", tool.id)))
            .min_w(gpui::rems(12.5))
            .flex_basis(gpui::rems(14.0))
            .flex_grow_1()
            .p_4()
            .gap_2p5()
            .rounded(px(12.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .shadow_sm()
            .cursor_pointer()
            .hover(move |style| style.border_color(accent).shadow_sm())
            .child(
                div()
                    .w(px(38.0))
                    .h(px(38.0))
                    .rounded(px(10.0))
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

    /// 「更多工具」占位卡（demo：虚线边框、无阴影、居中）。
    fn render_ghost_card(&self, cx: &Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .min_w(gpui::rems(12.5))
            .flex_basis(gpui::rems(14.0))
            .flex_grow_1()
            .p_4()
            .gap_2p5()
            .rounded(px(12.0))
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
        let mut tools = h_flex().flex_wrap().gap_3p5().max_w(gpui::rems(47.5));
        for tool in registered_tools(cx) {
            tools = tools.child(self.render_tool_card(tool, cx));
        }
        tools = tools.child(self.render_ghost_card(cx));
        v_flex()
            .id("toolbox-content")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .px_7()
            .py_6()
            .bg(cx.theme().background)
            .child(
                div()
                    .text_base()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .mb_1()
                    .child(t!("Home.toolbox").to_string()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .mb_5()
                    .child(t!("Home.toolbox_description").to_string()),
            )
            .child(tools)
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
}
