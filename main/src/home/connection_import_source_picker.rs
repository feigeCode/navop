use super::connection_import_source_icon::source_icon;
use connection_importer::{ImportSourceKind, ImportSourceStatus, SourceAvailability};
use gpui::{
    AnyElement, Context, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, px,
};
use gpui_component::{ActiveTheme, Disableable, checkbox::Checkbox, h_flex, v_flex};

pub(crate) struct ConnectionImportSourcePicker {
    rows: Vec<ImportSourceRow>,
}

struct ImportSourceRow {
    status: ImportSourceStatus,
    checked: bool,
}

impl ConnectionImportSourcePicker {
    pub(crate) fn new(sources: Vec<ImportSourceStatus>) -> Self {
        let rows = sources
            .into_iter()
            .map(|status| ImportSourceRow {
                checked: is_available_source(&status),
                status,
            })
            .collect();
        Self { rows }
    }

    pub(crate) fn selected_sources(&self) -> Vec<ImportSourceKind> {
        self.rows
            .iter()
            .filter(|row| row.checked && is_available_source(&row.status))
            .map(|row| row.status.kind)
            .collect()
    }

    pub(crate) fn toggle_source(&mut self, kind: ImportSourceKind) {
        let Some(row) = self.rows.iter_mut().find(|row| row.status.kind == kind) else {
            return;
        };
        if is_available_source(&row.status) {
            row.checked = !row.checked;
        }
    }

    fn render_summary(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("选择要扫描的应用，下一步会读取这些应用里的连接配置。"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("已选择 {} 个应用", self.selected_sources().len())),
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

        v_flex()
            .gap_4()
            .child(self.render_summary(cx))
            .child(v_flex().gap_2().children(rows))
    }
}

fn render_source_row(
    row: &ImportSourceRow,
    cx: &mut Context<ConnectionImportSourcePicker>,
) -> AnyElement {
    let available = is_available_source(&row.status);
    let kind = row.status.kind;
    h_flex()
        .items_center()
        .justify_between()
        .gap_3()
        .p_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(6.0))
        .child(render_source_label(row, cx))
        .child(
            Checkbox::new(format!("import-source-{:?}", kind))
                .checked(row.checked && available)
                .disabled(!available)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_source(kind);
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
        .child(source_icon(row.status.kind))
        .child(
            v_flex()
                .gap_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child(row.status.display_name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(source_availability_summary(&row.status.availability)),
                ),
        )
}

pub(crate) fn is_available_source(source: &ImportSourceStatus) -> bool {
    matches!(
        source.availability,
        SourceAvailability::Available { .. }
            | SourceAvailability::Installed
            | SourceAvailability::NoConnections
    )
}

pub(crate) fn source_availability_summary(availability: &SourceAvailability) -> SharedString {
    match availability {
        SourceAvailability::Available { connection_count } => {
            format!("找到 {} 个连接", connection_count).into()
        }
        SourceAvailability::Installed => "已安装，未发现连接".into(),
        SourceAvailability::NoConnections => "未发现连接".into(),
        SourceAvailability::NotInstalled => "未检测到应用数据".into(),
        SourceAvailability::Unsupported => "暂不支持".into(),
        SourceAvailability::PermissionRequired => "需要文件访问权限".into(),
        SourceAvailability::Error { message } => format!("读取失败：{}", message).into(),
    }
}

#[cfg(test)]
mod tests {
    use connection_importer::{ImportSourceKind, ImportSourceStatus, SourceAvailability};

    use super::ConnectionImportSourcePicker;

    #[test]
    fn source_picker_returns_only_checked_available_sources() {
        let mut picker = ConnectionImportSourcePicker::new(vec![
            status(
                ImportSourceKind::DataGrip,
                SourceAvailability::Available {
                    connection_count: 2,
                },
            ),
            status(
                ImportSourceKind::Xshell,
                SourceAvailability::Available {
                    connection_count: 1,
                },
            ),
            status(ImportSourceKind::Navicat, SourceAvailability::NotInstalled),
        ]);

        picker.toggle_source(ImportSourceKind::Xshell);

        assert_eq!(vec![ImportSourceKind::DataGrip], picker.selected_sources());
    }

    #[test]
    fn source_picker_does_not_select_unavailable_sources() {
        let mut picker = ConnectionImportSourcePicker::new(vec![
            status(
                ImportSourceKind::DataGrip,
                SourceAvailability::Available {
                    connection_count: 2,
                },
            ),
            status(ImportSourceKind::Navicat, SourceAvailability::NotInstalled),
        ]);

        picker.toggle_source(ImportSourceKind::Navicat);

        assert_eq!(vec![ImportSourceKind::DataGrip], picker.selected_sources());
    }

    fn status(kind: ImportSourceKind, availability: SourceAvailability) -> ImportSourceStatus {
        ImportSourceStatus::new(kind, availability)
    }
}
