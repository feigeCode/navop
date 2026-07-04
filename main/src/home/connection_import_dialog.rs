use super::connection_import_actions::{preview_import_drafts, save_selected_import_drafts};
use super::connection_import_preview_view::ConnectionImportPreview;
use super::connection_import_source_picker::ConnectionImportSourcePicker;
use connection_import_protocol::ImporterDescriptor;
use extension_runtime::{
    connection_import_provider::list_manifest_connection_importers,
    extension::{ExtensionKind, extensions_root},
};
use gpui::{App, AppContext, AsyncApp, ParentElement, Window, px};
use gpui_component::{WindowExt, dialog::DialogButtonProps};
use one_core::gpui_tokio::Tokio;
use rust_i18n::t;

pub(crate) fn show_connection_import_dialog(window: &mut Window, cx: &mut App) {
    let sources = list_connection_importers();
    if sources.is_empty() {
        window.push_notification("未安装连接导入扩展".to_string(), cx);
        return;
    }

    let picker = cx.new(|_| ConnectionImportSourcePicker::new(sources));
    let picker_for_render = picker.clone();
    let picker_for_ok = picker.clone();

    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .title(t!("Home.import").to_string())
            .w(px(620.0))
            .child(picker_for_render.clone())
            .confirm()
            .button_props(
                DialogButtonProps::default()
                    .ok_text("扫描")
                    .cancel_text(t!("Common.cancel"))
                    .show_cancel(true),
            )
            .on_ok({
                let picker_for_ok = picker_for_ok.clone();
                move |_, window, cx| {
                    let selected = picker_for_ok.read(cx).selected_sources();
                    if selected.is_empty() {
                        window.push_notification("请选择要扫描的导入扩展".to_string(), cx);
                        return false;
                    }
                    window.close_dialog(cx);
                    window.defer(cx, move |window, cx| {
                        show_connection_import_preview_dialog(selected.clone(), window, cx);
                    });
                    false
                }
            })
    });
}

fn list_connection_importers() -> Vec<ImporterDescriptor> {
    let Some(root) = extensions_root() else {
        return Vec::new();
    };
    let composite_root = root.join(ExtensionKind::Composite.dir_name());
    match list_manifest_connection_importers(&composite_root) {
        Ok(importers) => importers
            .into_iter()
            .map(|importer| importer.descriptor)
            .collect(),
        Err(error) => {
            tracing::warn!("加载连接导入扩展失败: {error:?}");
            Vec::new()
        }
    }
}

fn show_connection_import_preview_dialog(
    importer_ids: Vec<String>,
    window: &mut Window,
    cx: &mut App,
) {
    let preview = cx.new(|_| ConnectionImportPreview::loading());
    let preview_for_render = preview.clone();
    let preview_for_ok = preview.clone();
    let window_handle = window.window_handle();
    let preview_task = Tokio::spawn(cx, async move { preview_import_drafts(importer_ids).await });

    cx.spawn({
        let preview = preview.clone();
        async move |cx: &mut AsyncApp| {
            let result = match preview_task.await {
                Ok(result) => result,
                Err(error) => Err(format!("导入预览任务失败: {}", error)),
            };
            let _ = cx.update_window(window_handle, |_, window, cx| {
                preview.update(cx, |preview, cx| match result {
                    Ok(drafts) => preview.set_preview_result(drafts, None, window, cx),
                    Err(error) => preview.set_preview_result(Vec::new(), Some(error), window, cx),
                });
            });
        }
    })
    .detach();

    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .title(t!("Home.import").to_string())
            .w(px(760.0))
            .child(preview_for_render.clone())
            .confirm()
            .button_props(
                DialogButtonProps::default()
                    .ok_text(t!("Home.import"))
                    .cancel_text(t!("Common.cancel"))
                    .show_cancel(true),
            )
            .on_ok({
                let preview_for_ok = preview_for_ok.clone();
                move |_, window, cx| {
                    let drafts = match preview_for_ok.read(cx).collect_drafts(cx) {
                        Ok(drafts) => drafts,
                        Err(error) => {
                            window.push_notification(format!("导入失败：{}", error), cx);
                            return false;
                        }
                    };
                    save_preview_drafts(&drafts, window, cx)
                }
            })
    });
}

fn save_preview_drafts(
    drafts: &[super::connection_import_draft::EditableImportDraft],
    window: &mut Window,
    cx: &mut App,
) -> bool {
    match save_selected_import_drafts(drafts, cx) {
        Ok(0) => {
            window.push_notification("没有选择要导入的连接".to_string(), cx);
            false
        }
        Ok(count) => {
            window.push_notification(t!("Home.import_success", count = count).to_string(), cx);
            true
        }
        Err(error) => {
            window.push_notification(format!("导入失败：{}", error), cx);
            false
        }
    }
}
