use std::sync::Arc;

use gpui::{AppContext, AsyncApp, Context, PromptLevel, WeakEntity, Window};
use gpui_component::{WindowExt, notification::Notification};
use one_core::gpui_tokio::Tokio;
use remote_desktop::RemoteDesktopProtocol;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::database_driver_install_progress::{
    DriverInstallProgressSnapshot, DriverInstallProgressView, driver_install_progress_callback,
    mark_driver_install_finished, open_driver_install_progress_dialog,
    watch_driver_install_progress,
};
use crate::extension::{ExtensionKind, ExtensionRegistry, ExtensionSummary};
use crate::extension_downloader::{
    DownloadProgressCallback, MarketplaceEntry,
    download_marketplace_entry_to_staging_with_progress, fetch_default_manifest_url,
    fetch_manifest_url, install_from_staging_generic, install_marketplace_entry_generic,
};
use one_core::storage::StoredConnection;
use one_core::tab_container::TabOpenMode;

pub trait RemoteDesktopConnectionOpener: Sized + 'static {
    fn open_remote_desktop_connection(
        &mut self,
        connection: &StoredConnection,
        protocol: RemoteDesktopProtocol,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
}

pub fn required_provider_for_protocol(protocol: RemoteDesktopProtocol) -> &'static str {
    protocol.provider_id()
}

pub fn find_remote_desktop_provider_entry<'a>(
    entries: &'a [MarketplaceEntry],
    provider_id: &str,
) -> Option<&'a MarketplaceEntry> {
    entries
        .iter()
        .find(|entry| entry.kind == ExtensionKind::RemoteDesktopProvider && entry.id == provider_id)
}

pub async fn install_remote_desktop_provider_from_marketplace_with_registry(
    http_client: Arc<dyn gpui::http_client::HttpClient>,
    manifest_url: &str,
    provider_id: &str,
    registry: &ExtensionRegistry,
) -> anyhow::Result<ExtensionSummary> {
    let manifest = fetch_manifest_url(http_client.clone(), manifest_url).await?;
    let entries = manifest.into_entries();
    let entry = find_remote_desktop_provider_entry(&entries, provider_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("扩展市场未找到远程桌面插件 {provider_id}"))?;
    install_marketplace_entry_generic(http_client, &entry, registry).await
}

pub fn open_remote_desktop_connection_with_provider_guard<T>(
    home: &mut T,
    connection: StoredConnection,
    protocol: RemoteDesktopProtocol,
    mode: TabOpenMode,
    window: &mut Window,
    cx: &mut Context<T>,
) where
    T: RemoteDesktopConnectionOpener,
{
    if remote_desktop::RemoteDesktopProviderRegistry::load_default()
        .find(protocol)
        .is_some()
    {
        home.open_remote_desktop_connection(&connection, protocol, mode, window, cx);
        return;
    }
    prompt_install_provider(connection, protocol, mode, window, cx);
}

fn prompt_install_provider<T>(
    connection: StoredConnection,
    protocol: RemoteDesktopProtocol,
    mode: TabOpenMode,
    window: &mut Window,
    cx: &mut Context<T>,
) where
    T: RemoteDesktopConnectionOpener,
{
    let provider_id = required_provider_for_protocol(protocol).to_string();
    let connection_name = connection.name.clone();
    prompt_install_provider_with_completion(
        provider_id.clone(),
        protocol,
        connection_name,
        window,
        cx,
        move |home, window, cx| {
            home.open_remote_desktop_connection(&connection, protocol, mode, window, cx);
        },
    );
}

fn prompt_install_provider_with_completion<T, F>(
    provider_id: String,
    protocol: RemoteDesktopProtocol,
    connection_name: String,
    window: &mut Window,
    cx: &mut Context<T>,
    on_success: F,
) where
    T: 'static,
    F: FnOnce(&mut T, &mut Window, &mut Context<T>) + 'static,
{
    if ExtensionRegistry::global().is_none() {
        notify_error(window, cx, "扩展系统未初始化，无法安装远程桌面插件");
        return;
    }

    let answer = window.prompt(
        PromptLevel::Warning,
        "需要安装远程桌面插件",
        Some(&format!(
            "连接「{}」需要安装「{}」远程桌面插件。",
            connection_name,
            protocol.label()
        )),
        &["下载并安装", "取消"],
        cx,
    );
    let http_client = cx.http_client();
    let window_handle = window.window_handle();
    let progress_view = cx.new(|_| DriverInstallProgressView::new(&provider_id, &connection_name));
    let progress_view_weak = progress_view.downgrade();
    let progress_snapshot = Arc::new(Mutex::new(DriverInstallProgressSnapshot::default()));
    let progress_finished = Arc::new(AtomicBool::new(false));
    watch_driver_install_progress(
        progress_view_weak.clone(),
        Arc::clone(&progress_snapshot),
        Arc::clone(&progress_finished),
        cx,
    );

    cx.spawn(async move |this: WeakEntity<T>, cx: &mut AsyncApp| {
        if answer.await.ok() != Some(0) {
            progress_finished.store(true, Ordering::Relaxed);
            return;
        }
        open_install_progress_dialog(window_handle, progress_view, cx);
        let install_provider_id = provider_id.clone();
        let progress_callback = driver_install_progress_callback(progress_snapshot);
        let task = Tokio::spawn(cx, async move {
            install_remote_desktop_provider_from_marketplace(
                http_client,
                &install_provider_id,
                progress_callback,
            )
            .await
        });
        let outcome = match task.await {
            Ok(Ok(_summary)) => Ok(()),
            Ok(Err(error)) => Err(format!("{error:?}")),
            Err(error) => Err(format!("任务执行失败: {error}")),
        };
        progress_finished.store(true, Ordering::Relaxed);
        finish_install_and_open(
            window_handle,
            this,
            provider_id,
            progress_view_weak,
            outcome,
            on_success,
            cx,
        );
    })
    .detach();
}

async fn install_remote_desktop_provider_from_marketplace(
    http_client: Arc<dyn gpui::http_client::HttpClient>,
    provider_id: &str,
    on_progress: DownloadProgressCallback,
) -> anyhow::Result<ExtensionSummary> {
    let manifest = fetch_default_manifest_url(http_client.clone()).await?;
    let entries = manifest.into_entries();
    let entry = find_remote_desktop_provider_entry(&entries, provider_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("扩展市场未找到远程桌面插件 {provider_id}"))?;
    let staging =
        download_marketplace_entry_to_staging_with_progress(http_client, &entry, on_progress)
            .await?;
    let result = install_staged_remote_desktop_provider(&staging);
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn install_staged_remote_desktop_provider(
    staging: &std::path::Path,
) -> anyhow::Result<ExtensionSummary> {
    let registry =
        ExtensionRegistry::global().ok_or_else(|| anyhow::anyhow!("扩展系统未初始化"))?;
    let registry = registry
        .read()
        .map_err(|error| anyhow::anyhow!("registry lock poisoned: {error}"))?;
    install_from_staging_generic(
        staging,
        &registry,
        Some(ExtensionKind::RemoteDesktopProvider),
    )
}

fn open_install_progress_dialog(
    window_handle: gpui::AnyWindowHandle,
    progress_view: gpui::Entity<DriverInstallProgressView>,
    cx: &mut AsyncApp,
) {
    let _ = cx.update_window(window_handle, |_, window, cx| {
        open_driver_install_progress_dialog(progress_view, window, cx);
    });
}

fn finish_install_and_open<T: 'static>(
    window_handle: gpui::AnyWindowHandle,
    target: WeakEntity<T>,
    provider_id: String,
    progress_view: gpui::WeakEntity<DriverInstallProgressView>,
    outcome: Result<(), String>,
    on_success: impl FnOnce(&mut T, &mut Window, &mut Context<T>) + 'static,
    cx: &mut AsyncApp,
) {
    if outcome.is_ok() {
        mark_driver_install_finished(&progress_view, cx);
    }
    let _ = cx.update_window(window_handle, |_, window, cx| {
        window.close_dialog(cx);
        if let Some(target) = target.upgrade() {
            target.update(cx, |target, cx| match outcome {
                Ok(()) => {
                    notify_success(window, cx, format!("已安装 {provider_id} 远程桌面插件"));
                    on_success(target, window, cx);
                }
                Err(error) => notify_error(window, cx, format!("安装远程桌面插件失败: {error}")),
            });
        }
    });
}

fn notify_error<T>(window: &mut Window, cx: &mut Context<T>, message: impl Into<String>) {
    window.push_notification(Notification::error(message.into()), cx);
}

fn notify_success<T>(window: &mut Window, cx: &mut Context<T>, message: impl Into<String>) {
    window.push_notification(Notification::success(message.into()), cx);
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use futures::FutureExt;
    use gpui::http_client::{self, AsyncBody, HttpClient, Url, http};
    use remote_desktop::RemoteDesktopProtocol;

    use crate::extension::{
        ExtensionKind, ExtensionRegistry, RemoteDesktopProviderExtensionProvider,
    };
    use crate::extension_downloader::MarketplaceEntry;

    #[test]
    fn required_provider_for_protocol_uses_stable_ids() {
        assert_eq!(
            "rdp",
            super::required_provider_for_protocol(RemoteDesktopProtocol::Rdp)
        );
        assert_eq!(
            "vnc",
            super::required_provider_for_protocol(RemoteDesktopProtocol::Vnc)
        );
    }

    #[test]
    fn find_remote_desktop_provider_entry_matches_kind_and_id() {
        let entries = vec![
            entry("rdp", ExtensionKind::DatabaseDriver),
            entry("vnc", ExtensionKind::RemoteDesktopProvider),
        ];

        let found = super::find_remote_desktop_provider_entry(&entries, "vnc");

        assert_eq!(Some("vnc"), found.map(|entry| entry.id.as_str()));
    }

    #[test]
    fn install_remote_desktop_provider_from_marketplace_installs_matching_entry() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tarball = remote_desktop_provider_tarball_bytes();
        let sha256 = sha256_hex(&tarball);
        let manifest = format!(
            r#"{{
                "extensions": [{{
                    "id": "rdp",
                    "kind": "remote_desktop_provider",
                    "name": "RDP",
                    "version": "1.2.3",
                    "release_tag": "rdp-v1.2.3",
                    "artifacts": {{
                        "universal": {{
                            "file": "rdp-remote-desktop-provider-universal.tar.gz",
                            "sha256": "{sha256}"
                        }}
                    }}
                }}]
            }}"#
        );
        let client = Arc::new(FakeHttpClient::new(vec![
            FakeHttpClient::response(200, &manifest),
            binary_response(200, tarball),
        ]));
        let mut registry = ExtensionRegistry::new(tmp.path().join("extensions"));
        registry.register_provider(Arc::new(RemoteDesktopProviderExtensionProvider));

        let summary = smol::block_on(
            super::install_remote_desktop_provider_from_marketplace_with_registry(
                client,
                "https://example.test/manifest.json",
                "rdp",
                &registry,
            ),
        )
        .unwrap();

        assert_eq!(ExtensionKind::RemoteDesktopProvider, summary.kind);
        assert_eq!("rdp", summary.name);
        assert!(summary.path.join("remote_desktop_provider.json").exists());
    }

    fn entry(id: &str, kind: ExtensionKind) -> MarketplaceEntry {
        MarketplaceEntry::from_resolved_urls(
            id,
            kind,
            id,
            "1.0.0",
            "",
            Vec::new(),
            vec![format!("https://example.test/{id}.tar.gz")],
            Some("hash".to_string()),
        )
    }

    fn remote_desktop_provider_tarball_bytes() -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        append_bytes(&mut archive, "onetcli-rdp-helper", b"helper");
        append_bytes(
            &mut archive,
            "remote_desktop_provider.json",
            br#"{
                "id": "rdp",
                "name": "RDP",
                "description": "RDP provider",
                "version": "1.2.3",
                "protocol": "rdp",
                "entry": { "command": "./onetcli-rdp-helper" },
                "capabilities": {
                    "resize": "remote_resize",
                    "clipboard_text": true,
                    "cursor_shape": true,
                    "audio": false,
                    "file_transfer": false
                },
                "ui": { "default_port": 3389 }
            }"#,
        );
        let encoder = archive.into_inner().unwrap();
        encoder.finish().unwrap()
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    fn binary_response(status: u16, body: Vec<u8>) -> anyhow::Result<http::Response<AsyncBody>> {
        http::Response::builder()
            .status(status)
            .body(AsyncBody::from(body))
            .map_err(|error| anyhow::anyhow!("构建响应失败: {}", error))
    }

    fn append_bytes(
        archive: &mut tar::Builder<flate2::write::GzEncoder<Vec<u8>>>,
        name: &str,
        bytes: &[u8],
    ) {
        let mut header = tar::Header::new_gnu();
        header.set_path(name).unwrap();
        header.set_size(bytes.len() as u64);
        header.set_cksum();
        archive.append(&header, bytes).unwrap();
    }

    struct FakeHttpClient {
        responses: Mutex<VecDeque<anyhow::Result<http_client::Response<AsyncBody>>>>,
    }

    impl FakeHttpClient {
        fn new(responses: Vec<anyhow::Result<http_client::Response<AsyncBody>>>) -> Self {
            Self {
                responses: Mutex::new(VecDeque::from(responses)),
            }
        }

        fn response(status: u16, body: &str) -> anyhow::Result<http_client::Response<AsyncBody>> {
            http::Response::builder()
                .status(status)
                .body(AsyncBody::from(body.as_bytes().to_vec()))
                .map_err(|error| anyhow::anyhow!("构建响应失败: {}", error))
        }
    }

    impl HttpClient for FakeHttpClient {
        fn proxy(&self) -> Option<&Url> {
            None
        }

        fn user_agent(&self) -> Option<&http::HeaderValue> {
            None
        }

        fn send(
            &self,
            _req: http::Request<AsyncBody>,
        ) -> futures::future::BoxFuture<'static, anyhow::Result<http_client::Response<AsyncBody>>>
        {
            let result = self
                .responses
                .lock()
                .expect("responses 锁失败")
                .pop_front()
                .unwrap_or_else(|| Err(anyhow::anyhow!("缺少 fake response")));

            async move { result }.boxed()
        }
    }
}
