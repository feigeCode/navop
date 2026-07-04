use gpui::{
    AnyElement, App, FontWeight, Hsla, IntoElement, ParentElement, Pixels, Styled, div, px,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable, Size};

pub(super) const WORKSPACE_TOOLBAR_HEIGHT: Pixels = px(72.0);
pub(super) const WORKSPACE_TOOLBAR_ITEM_WIDTH: Pixels = px(76.0);
pub(super) const WORKSPACE_TOOLBAR_ITEM_HEIGHT: Pixels = px(58.0);
pub(super) const WORKSPACE_TOOLBAR_ICON_SIZE: Pixels = px(34.0);
pub(super) const WORKSPACE_TOOLBAR_ITEM_RADIUS: Pixels = px(8.0);
pub(super) const WORKSPACE_TOOLBAR_HOVER_ALPHA: f32 = 0.55;
pub(super) const WORKSPACE_TOOLBAR_ICON_BG_ALPHA: f32 = 0.12;

#[derive(Clone, Copy)]
pub(super) enum DatabaseToolbarAction {
    ShowObjects,
    CreateQuery,
    Users,
    CompareSchema,
    CompareData,
    DataGenerator,
    Backup,
    Automation,
    Model,
    Bi,
}

#[derive(Clone, Copy)]
pub(super) enum DatabaseToolbarTone {
    Primary,
    Success,
    Warning,
    Info,
}

#[derive(Clone)]
pub(super) struct DatabaseToolbarItem {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: IconName,
    pub action: DatabaseToolbarAction,
    pub tone: DatabaseToolbarTone,
}

pub(super) fn database_toolbar_items() -> Vec<DatabaseToolbarItem> {
    vec![
        toolbar_item(
            "db-toolbar-show",
            "显示",
            IconName::Eye,
            DatabaseToolbarAction::ShowObjects,
            DatabaseToolbarTone::Primary,
        ),
        toolbar_item(
            "db-toolbar-query",
            "新建查询",
            IconName::Query,
            DatabaseToolbarAction::CreateQuery,
            DatabaseToolbarTone::Info,
        ),
        toolbar_item(
            "db-toolbar-users",
            "用户",
            IconName::User,
            DatabaseToolbarAction::Users,
            DatabaseToolbarTone::Warning,
        ),
        toolbar_item(
            "db-toolbar-schema-compare",
            "比较结构",
            IconName::SchemaCompare,
            DatabaseToolbarAction::CompareSchema,
            DatabaseToolbarTone::Primary,
        ),
        toolbar_item(
            "db-toolbar-data-compare",
            "比较数据",
            IconName::Sync,
            DatabaseToolbarAction::CompareData,
            DatabaseToolbarTone::Success,
        ),
        toolbar_item(
            "db-toolbar-data-generator",
            "数据生成",
            IconName::TableDesignTool,
            DatabaseToolbarAction::DataGenerator,
            DatabaseToolbarTone::Warning,
        ),
        toolbar_item(
            "db-toolbar-backup",
            "备份",
            IconName::Export,
            DatabaseToolbarAction::Backup,
            DatabaseToolbarTone::Primary,
        ),
        toolbar_item(
            "db-toolbar-automation",
            "自动运行",
            IconName::Play,
            DatabaseToolbarAction::Automation,
            DatabaseToolbarTone::Success,
        ),
        toolbar_item(
            "db-toolbar-model",
            "模型",
            IconName::DataModel,
            DatabaseToolbarAction::Model,
            DatabaseToolbarTone::Info,
        ),
        toolbar_item(
            "db-toolbar-bi",
            "BI",
            IconName::ChartPie,
            DatabaseToolbarAction::Bi,
            DatabaseToolbarTone::Primary,
        ),
    ]
}

fn toolbar_item(
    id: &'static str,
    label: &'static str,
    icon: IconName,
    action: DatabaseToolbarAction,
    tone: DatabaseToolbarTone,
) -> DatabaseToolbarItem {
    DatabaseToolbarItem {
        id,
        label,
        icon,
        action,
        tone,
    }
}

pub(super) fn toolbar_tone_color(tone: DatabaseToolbarTone, cx: &App) -> Hsla {
    match tone {
        DatabaseToolbarTone::Primary => cx.theme().primary,
        DatabaseToolbarTone::Success => cx.theme().success,
        DatabaseToolbarTone::Warning => cx.theme().warning,
        DatabaseToolbarTone::Info => cx.theme().info,
    }
}

pub(super) fn toolbar_item_icon(icon: IconName, color: Hsla) -> AnyElement {
    div()
        .w(WORKSPACE_TOOLBAR_ICON_SIZE)
        .h(WORKSPACE_TOOLBAR_ICON_SIZE)
        .flex()
        .items_center()
        .justify_center()
        .rounded(WORKSPACE_TOOLBAR_ITEM_RADIUS)
        .bg(color.opacity(WORKSPACE_TOOLBAR_ICON_BG_ALPHA))
        .child(Icon::new(icon).with_size(Size::Large).text_color(color))
        .into_any_element()
}

pub(super) fn toolbar_item_label(label: &'static str, cx: &App) -> AnyElement {
    div()
        .w_full()
        .text_center()
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.theme().foreground)
        .whitespace_nowrap()
        .child(label)
        .into_any_element()
}
