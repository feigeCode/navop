use std::sync::Arc;

use extension_runtime::MainExtensionViewHost;
use extension_view::{
    ExtensionManagerView, ExtensionViewHost, MarketplaceEntry, filter_updatable_marketplace,
};
use gpui::http_client::HttpClient;
use gpui::{App, AppContext, Entity, Window};
use gpui_component::WindowExt;
use gpui_component::button::Button;
use gpui_component::notification::Notification;
use one_core::gpui_tokio::Tokio;
use one_core::tab_container::{TabContainer, TabItem};
use rust_i18n::t;

use crate::onetcli_app::GlobalTabContainer;
use crate::setting_tab::AppSettings;

/// 插件更新提示通知的唯一标识，避免被其它通知覆盖。
struct PluginUpdateNotification;

/// 应用启动时调用：在后台检查已安装插件是否有市场新版本。
///
/// - 跟随「自动检查更新」设置（`auto_update`），关闭自动更新时不做检查；
/// - 每个更新集合只提示一次：本次更新集合的签名与上次提示一致时不再打扰。
pub fn schedule_plugin_update_check(window: &mut Window, cx: &mut App) {
    if !AppSettings::global(cx).auto_update {
        return;
    }

    let http_client = cx.http_client();
    let host = MainExtensionViewHost;
    let previously_notified = AppSettings::global(cx)
        .plugin_update_notified_signature
        .clone();
    let task = Tokio::spawn(cx, check_for_plugin_updates(host, http_client));

    window
        .spawn(cx, async move |cx| {
            let updates = match task.await {
                Ok(Ok(updates)) => updates,
                Ok(Err(err)) => {
                    tracing::warn!("插件更新检查失败: {err:#}");
                    return;
                }
                Err(err) => {
                    tracing::warn!("插件更新检查任务执行失败: {err}");
                    return;
                }
            };
            if updates.is_empty() {
                return;
            }
            let signature = update_signature(&updates);
            if previously_notified.as_deref() == Some(signature.as_str()) {
                return;
            }
            let _ = cx.update(|_window, cx| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.plugin_update_notified_signature = Some(signature.clone());
                });
            });
            show_plugin_update_notification(updates, cx);
        })
        .detach();
}

/// 在后台查询已安装插件中哪些在市场上存在可用的新版本。
async fn check_for_plugin_updates(
    host: MainExtensionViewHost,
    http_client: Arc<dyn HttpClient>,
) -> anyhow::Result<Vec<MarketplaceEntry>> {
    let installed = host.list_installed()?;
    let entries = host.load_marketplace_entries(http_client).await?;
    Ok(filter_updatable_marketplace(&entries, &installed))
}

fn show_plugin_update_notification(
    updates: Vec<MarketplaceEntry>,
    cx: &mut gpui::AsyncWindowContext,
) {
    let count = updates.len();
    let _ = cx.update(|window, cx| {
        window.push_notification(
            Notification::info(t!("PluginUpdate.available", count = count).to_string())
                .id::<PluginUpdateNotification>()
                .title(t!("PluginUpdate.title").to_string())
                .action(|_this, _window, _cx| {
                    Button::new("plugin-update-open-marketplace")
                        .label(t!("PluginUpdate.view_updates").to_string())
                        .on_click(|_event, window, cx| {
                            open_extension_marketplace(window, cx);
                        })
                }),
            cx,
        );
    });
}

/// 打开「扩展市场」页签并只保留需要更新的扩展。
///
/// 页签已存在时 `activate_or_add_tab_lazy` 只会激活、不会重建视图，
/// 因此无论新建还是复用，都要对最终视图统一应用“有更新”过滤。
fn open_extension_marketplace(window: &mut Window, cx: &mut App) {
    let Some(tab_container) = cx
        .try_global::<GlobalTabContainer>()
        .map(|global| global.tab_container.clone())
    else {
        return;
    };
    window.defer(cx, move |window, cx| {
        tab_container.update(cx, |tc, cx| {
            tc.activate_or_add_tab_lazy(
                "extensions-marketplace",
                |win, cx| {
                    let host = Arc::new(MainExtensionViewHost);
                    let view = cx
                        .new(|cx| ExtensionManagerView::new_marketplace_search(host, "", win, cx));
                    TabItem::new("extensions-marketplace", "home", view)
                },
                window,
                cx,
            );
            if let Some(view) = extension_manager_view(tc) {
                view.update(cx, |view, cx| view.show_updates_only(window, cx));
            }
        });
    });
}

fn extension_manager_view(tc: &mut TabContainer) -> Option<Entity<ExtensionManagerView>> {
    let tab = tc
        .tabs()
        .iter()
        .find(|tab| tab.id() == "extensions-marketplace")?;
    tab.content().view().downcast::<ExtensionManagerView>().ok()
}

/// 依据更新集合计算签名：按插件 id 排序后拼接 `id@version`，保证结果稳定。
fn update_signature(updates: &[MarketplaceEntry]) -> String {
    let mut items = updates
        .iter()
        .map(|entry| (entry.id.clone(), entry.version.clone()))
        .collect::<Vec<_>>();
    items.sort();
    items
        .into_iter()
        .map(|(id, version)| format!("{id}@{version}"))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use extension_view::{ExtensionKind, MarketplaceEntry};

    use super::update_signature;

    fn entry(id: &str, version: &str) -> MarketplaceEntry {
        MarketplaceEntry {
            id: id.to_string(),
            kind: ExtensionKind::DatabaseDriver,
            name: id.to_string(),
            version: version.to_string(),
            description: String::new(),
            file_extensions: Vec::new(),
            required_host_version: None,
            host_compatible: true,
            asset_url: String::new(),
            sha256: None,
            fallback_asset_url: None,
            manifest_url: None,
            manifest_fallback_url: None,
        }
    }

    #[test]
    fn signature_is_deterministic_regardless_of_entry_order() {
        let mut updates = vec![entry("kingbase", "0.1.7"), entry("dm", "1.0.3")];
        updates.reverse();
        let first = update_signature(&updates);
        updates.reverse();
        assert_eq!(first, update_signature(&updates));
        assert_eq!("dm@1.0.3,kingbase@0.1.7", first);
    }

    #[test]
    fn signature_changes_when_versions_change() {
        let old = update_signature(&[entry("kingbase", "0.1.6")]);
        let new = update_signature(&[entry("kingbase", "0.1.7")]);
        assert_ne!(old, new);
    }
}
