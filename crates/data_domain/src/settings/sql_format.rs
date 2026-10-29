use gpui::{
    App, AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, div, px,
};
use gpui_component::{
    ActiveTheme,
    input::{InputEvent, Textarea, TextareaState},
    setting::{SettingField, SettingGroup, SettingItem},
    v_flex,
};
use one_core::settings::{AppSettings, SqlIndentStyle, SqlKeywordCase};

use db::{SqlFormatOptions, format_sql_with_options};

const SAMPLE_SQL: &str = "SELECT u.id, u.name FROM users u WHERE u.status = 'ACTIVE' ${if(len(actual_controller_nm)==0,\"\",\" AND actual_controller_nm LIKE '%公司%' \")} AND u.created_at >= {{ params.start_date }} ORDER BY u.id DESC";

pub fn sql_format_setting_group() -> SettingGroup {
    SettingGroup::new()
        .title("SQL 格式化")
        .item(keyword_case_item())
        .item(indent_item())
        .item(
            SettingItem::render(|_options, window, cx| render_preview(window, cx)).keywords([
                "实时预览".to_string(),
                "编辑示例 SQL，下方按当前配置实时显示格式化结果；${...}、#{...}、{{...}} 模板片段会原样保留。".to_string(),
            ]),
        )
}

fn keyword_case_item() -> SettingItem {
    SettingItem::new(
        "关键字大小写",
        SettingField::dropdown(
            vec![
                (SqlKeywordCase::Preserve.as_str().into(), "保持原样".into()),
                (SqlKeywordCase::Upper.as_str().into(), "全部大写".into()),
                (SqlKeywordCase::Lower.as_str().into(), "全部小写".into()),
            ],
            |cx: &App| SharedString::from(AppSettings::global(cx).sql_format.keyword_case.as_str()),
            |val: SharedString, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.sql_format.keyword_case = SqlKeywordCase::from_value(&val);
                });
                cx.refresh_windows();
            },
        )
        .default_value(SharedString::from(SqlKeywordCase::Preserve.as_str())),
    )
    .description("格式化时 SQL 关键字的大小写策略；“保持原样”不改变原文大小写".to_string())
}

fn indent_item() -> SettingItem {
    SettingItem::new(
        "缩进风格",
        SettingField::dropdown(
            vec![
                (SqlIndentStyle::TwoSpaces.as_str().into(), "2 空格".into()),
                (SqlIndentStyle::FourSpaces.as_str().into(), "4 空格".into()),
                (SqlIndentStyle::Tabs.as_str().into(), "Tab 缩进".into()),
            ],
            |cx: &App| SharedString::from(AppSettings::global(cx).sql_format.indent.as_str()),
            |val: SharedString, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.sql_format.indent = SqlIndentStyle::from_value(&val);
                });
                cx.refresh_windows();
            },
        )
        .default_value(SharedString::from(SqlIndentStyle::TwoSpaces.as_str())),
    )
    .description("格式化后 SQL 使用的缩进".to_string())
}

struct SqlFormatPreview {
    input: Entity<TextareaState>,
    _subscriptions: Vec<gpui::Subscription>,
}

fn render_preview(window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    let editor = window.use_keyed_state("sql-format-preview", cx, |window, cx| {
        SqlFormatPreview::new(window, cx)
    });
    editor.into_any_element()
}

impl SqlFormatPreview {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 8)
                .default_value(SAMPLE_SQL.to_string())
        });
        let subscription = cx.subscribe(&input, |_preview, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            input,
            _subscriptions: vec![subscription],
        }
    }
}

impl Render for SqlFormatPreview {
    // 示例文本较短，渲染时同步格式化即可保证配置或输入变化后预览总是最新
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let options = SqlFormatOptions::from_settings(&AppSettings::global(cx).sql_format);
        let value = self.input.read(cx).value();
        let formatted = format_sql_with_options(&value, options);
        v_flex()
            .w_full()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("编辑示例 SQL，下方按当前配置实时显示格式化结果；${...}、#{...}、{{...}} 模板片段会原样保留。"),
            )
            .child(div().max_w(px(640.)).child(Textarea::new(&self.input)))
            .child(
                div()
                    .max_w(px(640.))
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .p_2()
                    .text_sm()
                    .child(div().child(formatted)),
            )
    }
}
