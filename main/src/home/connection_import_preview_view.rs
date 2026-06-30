use super::connection_import_draft::{
    EditableImportDraft, ImportDraftEdit, ImportDraftField, ImportDraftKind,
};
use super::connection_import_source_icon::source_icon;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled,
    Window, div, px,
};
use gpui_component::{
    ActiveTheme,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState},
    scroll::ScrollableElement,
    v_flex,
};

pub(crate) struct ConnectionImportPreview {
    rows: Vec<ConnectionImportDraftRow>,
    preview_error: Option<String>,
}

struct ConnectionImportDraftRow {
    draft: EditableImportDraft,
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    database: Entity<InputState>,
    private_key_path: Entity<InputState>,
}

impl ConnectionImportPreview {
    pub(crate) fn new(
        drafts: Vec<EditableImportDraft>,
        preview_error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let rows = drafts
            .into_iter()
            .map(|draft| ConnectionImportDraftRow::new(draft, window, cx))
            .collect();
        Self {
            rows,
            preview_error,
        }
    }

    pub(crate) fn collect_drafts(&self, cx: &App) -> Result<Vec<EditableImportDraft>, String> {
        self.rows.iter().map(|row| row.collect_draft(cx)).collect()
    }

    fn toggle_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get_mut(index) {
            let selected = !row.draft.selected;
            let _ = row.draft.apply_edit(ImportDraftEdit::Selected(selected));
        }
        cx.notify();
    }

    fn render_summary(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.rows.iter().filter(|row| row.draft.selected).count();
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("先勾选并临时编辑要导入的连接，确认后才会写入连接列表。"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("已选择 {} / {} 个连接", selected, self.rows.len())),
            )
    }

    fn render_empty_or_error(&self, cx: &mut Context<Self>) -> AnyElement {
        let message = self
            .preview_error
            .as_ref()
            .map(|error| format!("读取导入来源失败：{}", error))
            .unwrap_or_else(|| "未发现可导入的连接".to_string());
        div()
            .p_4()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(px(6.0))
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(message)
            .into_any_element()
    }
}

impl ConnectionImportDraftRow {
    fn new(
        draft: EditableImportDraft,
        window: &mut Window,
        cx: &mut Context<ConnectionImportPreview>,
    ) -> Self {
        Self {
            name: text_input(draft.name.clone(), window, cx),
            host: text_input(draft.host.clone(), window, cx),
            port: text_input(draft.port.clone(), window, cx),
            username: text_input(draft.username.clone(), window, cx),
            password: text_input(draft.password.clone(), window, cx),
            database: text_input(draft.database.clone(), window, cx),
            private_key_path: text_input(draft.private_key_path.clone(), window, cx),
            draft,
        }
    }

    fn collect_draft(&self, cx: &App) -> Result<EditableImportDraft, String> {
        let mut draft = self.draft.clone();
        for (field, input) in self.editable_inputs() {
            draft.apply_edit(ImportDraftEdit::Text {
                field,
                value: input.read(cx).text().to_string(),
            })?;
        }
        Ok(draft)
    }

    fn editable_inputs(&self) -> Vec<(ImportDraftField, Entity<InputState>)> {
        vec![
            (ImportDraftField::Name, self.name.clone()),
            (ImportDraftField::Host, self.host.clone()),
            (ImportDraftField::Port, self.port.clone()),
            (ImportDraftField::Username, self.username.clone()),
            (ImportDraftField::Password, self.password.clone()),
            (ImportDraftField::Database, self.database.clone()),
            (
                ImportDraftField::PrivateKeyPath,
                self.private_key_path.clone(),
            ),
        ]
    }
}

impl Render for ConnectionImportPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| render_draft_row(index, row, cx))
            .collect::<Vec<_>>();

        v_flex()
            .gap_4()
            .child(self.render_summary(cx))
            .child(if self.rows.is_empty() {
                self.render_empty_or_error(cx)
            } else {
                v_flex()
                    .gap_2()
                    .max_h(px(480.0))
                    .overflow_y_scrollbar()
                    .children(rows)
                    .into_any_element()
            })
    }
}

fn render_draft_row(
    index: usize,
    row: &ConnectionImportDraftRow,
    cx: &mut Context<ConnectionImportPreview>,
) -> AnyElement {
    h_flex()
        .items_start()
        .gap_3()
        .p_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(6.0))
        .child(
            div().pt_1().child(
                Checkbox::new(format!("import-draft-{index}"))
                    .checked(row.draft.selected)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_row(index, cx))),
            ),
        )
        .child(render_draft_body(row, cx))
        .into_any_element()
}

fn render_draft_body(
    row: &ConnectionImportDraftRow,
    cx: &mut Context<ConnectionImportPreview>,
) -> impl IntoElement {
    let kind_text = match row.draft.kind() {
        ImportDraftKind::Database => "数据库",
        ImportDraftKind::Ssh => "SSH / SFTP",
    };
    v_flex()
        .gap_3()
        .flex_1()
        .min_w_0()
        .child(render_draft_header(row, kind_text, cx))
        .child(render_primary_fields(row, cx))
        .child(render_secondary_fields(row, cx))
}

fn render_draft_header(
    row: &ConnectionImportDraftRow,
    kind_text: &str,
    cx: &mut Context<ConnectionImportPreview>,
) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap_2()
        .min_w_0()
        .child(source_icon(row.draft.source_kind()))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{} · {}", row.draft.source_name(), kind_text)),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(row.draft.source_id().to_string()),
        )
}

fn render_primary_fields(
    row: &ConnectionImportDraftRow,
    cx: &mut Context<ConnectionImportPreview>,
) -> impl IntoElement {
    h_flex()
        .gap_2()
        .child(render_input_field("名称", &row.name, px(190.0), cx))
        .child(render_input_field("主机", &row.host, px(190.0), cx))
        .child(render_input_field("端口", &row.port, px(86.0), cx))
        .child(render_input_field("用户", &row.username, px(140.0), cx))
}

fn render_secondary_fields(
    row: &ConnectionImportDraftRow,
    cx: &mut Context<ConnectionImportPreview>,
) -> impl IntoElement {
    h_flex()
        .gap_2()
        .when(row.draft.supports_database_edit(), |this| {
            this.child(render_input_field("数据库", &row.database, px(190.0), cx))
        })
        .when(row.draft.supports_password_edit(), |this| {
            this.child(render_input_field("密码", &row.password, px(190.0), cx))
        })
        .when(row.draft.supports_private_key_edit(), |this| {
            this.child(render_input_field(
                "私钥",
                &row.private_key_path,
                px(388.0),
                cx,
            ))
        })
}

fn render_input_field(
    label: &str,
    input: &Entity<InputState>,
    width: gpui::Pixels,
    cx: &mut Context<ConnectionImportPreview>,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .w(width)
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_string()),
        )
        .child(Input::new(input).w_full())
}

fn text_input(
    value: String,
    window: &mut Window,
    cx: &mut Context<ConnectionImportPreview>,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).default_value(value))
}
