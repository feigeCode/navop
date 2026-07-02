use super::connection_import_source_icon::importer_icon;
use connection_import_protocol::ImporterDescriptor;
use gpui::{
    AnyElement, Context, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, px,
};
use gpui_component::{ActiveTheme, checkbox::Checkbox, h_flex, scroll::ScrollableElement, v_flex};

pub(crate) struct ConnectionImportSourcePicker {
    rows: Vec<ImportSourceRow>,
}

struct ImportSourceRow {
    descriptor: ImporterDescriptor,
    checked: bool,
}

impl ConnectionImportSourcePicker {
    pub(crate) fn new(sources: Vec<ImporterDescriptor>) -> Self {
        let rows = sources
            .into_iter()
            .map(|descriptor| ImportSourceRow {
                checked: true,
                descriptor,
            })
            .collect();
        Self { rows }
    }

    pub(crate) fn selected_sources(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter(|row| row.checked)
            .map(|row| row.descriptor.id.clone())
            .collect()
    }

    pub(crate) fn toggle_source(&mut self, importer_id: &str) {
        let Some(row) = self
            .rows
            .iter_mut()
            .find(|row| row.descriptor.id == importer_id)
        else {
            return;
        };
        row.checked = !row.checked;
    }

    fn render_summary(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("选择要扫描的导入扩展，下一步会通过 Wasm 宿主读取扩展预览结果。"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("已选择 {} 个扩展", self.selected_sources().len())),
            )
    }
}

impl Render for ConnectionImportSourcePicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self
            .rows
            .iter()
            .map(|row| render_source_row(row, cx))
            .collect::<Vec<_>>();

        v_flex().gap_4().child(self.render_summary(cx)).child(
            v_flex()
                .gap_2()
                .max_h(px(520.0))
                .overflow_y_scrollbar()
                .children(rows),
        )
    }
}

fn render_source_row(
    row: &ImportSourceRow,
    cx: &mut Context<ConnectionImportSourcePicker>,
) -> AnyElement {
    let importer_id = row.descriptor.id.clone();
    h_flex()
        .items_center()
        .justify_between()
        .gap_3()
        .p_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(6.0))
        .min_w_0()
        .child(render_source_label(row, cx))
        .child(
            Checkbox::new(format!("import-source-{importer_id}"))
                .checked(row.checked)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_source(&importer_id);
                    cx.notify();
                })),
        )
        .into_any_element()
}

fn render_source_label(
    row: &ImportSourceRow,
    cx: &mut Context<ConnectionImportSourcePicker>,
) -> impl IntoElement {
    h_flex()
        .gap_3()
        .items_center()
        .min_w_0()
        .child(importer_icon(&row.descriptor))
        .child(
            v_flex()
                .gap_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child(row.descriptor.display_name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(source_availability_summary()),
                ),
        )
}

pub(crate) fn source_availability_summary() -> SharedString {
    "由 Wasm 扩展提供".into()
}

#[cfg(test)]
mod tests {
    use connection_import_protocol::{
        ImportRecordKind, ImporterCapabilities, ImporterDescriptor, Platform,
    };

    use super::ConnectionImportSourcePicker;

    #[test]
    fn source_picker_returns_only_checked_sources() {
        let mut picker = ConnectionImportSourcePicker::new(vec![
            descriptor("datagrip", ImportRecordKind::Database),
            descriptor("xshell", ImportRecordKind::Ssh),
        ]);

        picker.toggle_source("xshell");

        assert_eq!(vec!["datagrip".to_string()], picker.selected_sources());
    }

    fn descriptor(id: &str, kind: ImportRecordKind) -> ImporterDescriptor {
        ImporterDescriptor {
            id: id.to_string(),
            display_name: id.to_string(),
            description: None,
            icon: None,
            vendor: None,
            supported_platforms: vec![Platform::Macos],
            output_kinds: vec![kind],
            capabilities: ImporterCapabilities::default(),
        }
    }
}
