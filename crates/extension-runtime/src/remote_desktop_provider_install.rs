use gpui::{Context, Window};
use remote_desktop::{RemoteDesktopProtocol, RemoteDesktopProviderRegistry};
use std::sync::Arc;

use crate::extension::{ExtensionKind, ExtensionRegistry, ExtensionSummary};
use crate::extension_downloader::{
    DownloadProgressCallback, MarketplaceEntry, fetch_default_manifest_url, fetch_manifest_url,
    install_marketplace_entry_generic, install_marketplace_entry_with_progress,
};
use crate::install_flow::{notify_error, run_install_with_progress_prompt};
use one_core::storage::RemoteDesktopBackendPreference;
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
    run_with_remote_desktop_provider_guard(
        home,
        connection.clone(),
        protocol,
        window,
        cx,
        move |home, window, cx| {
            home.open_remote_desktop_connection(&connection, protocol, mode, window, cx);
        },
    );
}

pub fn run_with_remote_desktop_provider_guard<T, F>(
    target: &mut T,
    connection: StoredConnection,
    protocol: RemoteDesktopProtocol,
    window: &mut Window,
    cx: &mut Context<T>,
    on_ready: F,
) where
    T: 'static,
    F: FnOnce(&mut T, &mut Window, &mut Context<T>) + 'static,
{
    if connection_skips_remote_desktop_provider_guard(&connection, protocol)
        || remote_desktop::RemoteDesktopProviderRegistry::load_default()
            .find(protocol)
            .is_some()
    {
        on_ready(target, window, cx);
        return;
    }
    let provider_id = required_provider_for_protocol(protocol).to_string();
    let connection_name = connection.name.clone();
    prompt_install_provider_with_completion(
        provider_id,
        protocol,
        connection_name,
        window,
        cx,
        on_ready,
    );
}

/// Whether opening `connection` can proceed without the helper-based remote
/// desktop provider extension.
///
/// The Windows native MSTSC backend embeds the RDP control directly in the
/// app process and never spawns the `onetcli-rdp-helper` process, so
/// requiring that helper to be installed just to open the connection would
/// mislead users into downloading an extension they do not need. Connections
/// with an explicit WindowsNative backend preference (on a build that
/// actually compiled the native backend, without a SOCKS/HTTP proxy that
/// would force a Canvas fallback) skip the provider guard.
fn connection_skips_remote_desktop_provider_guard(
    connection: &StoredConnection,
    protocol: RemoteDesktopProtocol,
) -> bool {
    remote_desktop::windows_native_rdp_compiled()
        && matches!(protocol, RemoteDesktopProtocol::Rdp)
        && connection
            .to_remote_desktop_params()
            .map(|params| {
                params.backend_preference == RemoteDesktopBackendPreference::WindowsNative
                    && params.proxy.is_none()
            })
            .unwrap_or(false)
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
    let install_provider_id = provider_id.clone();
    run_install_with_progress_prompt(
        window,
        cx,
        (provider_id.clone(), connection_name.clone()),
        "需要安装远程桌面插件",
        format!(
            "连接「{}」需要安装「{}」远程桌面插件。",
            connection_name,
            protocol.label()
        ),
        &["下载并安装", "取消"],
        move |http_client, progress_callback| {
            install_remote_desktop_provider_from_marketplace(
                http_client,
                install_provider_id,
                progress_callback,
            )
        },
        on_success,
        format!("已安装 {provider_id} 远程桌面插件"),
        "安装远程桌面插件失败".to_string(),
    );
}

async fn install_remote_desktop_provider_from_marketplace(
    http_client: Arc<dyn gpui::http_client::HttpClient>,
    provider_id: String,
    on_progress: DownloadProgressCallback,
) -> anyhow::Result<ExtensionSummary> {
    let manifest = fetch_default_manifest_url(http_client.clone()).await?;
    let entries = manifest.into_entries();
    let entry = find_remote_desktop_provider_entry(&entries, &provider_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("扩展市场未找到远程桌面插件 {provider_id}"))?;
    let summary = install_marketplace_entry_with_progress(
        http_client,
        &entry,
        ExtensionKind::RemoteDesktopProvider,
        on_progress,
    )
    .await?;
    RemoteDesktopProviderRegistry::refresh_global_registry();
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use futures::FutureExt;
    use gpui::http_client::{self, AsyncBody, HttpClient, Url, http};
    use one_core::storage::{
        ProxyConfig, ProxyType, RemoteDesktopBackendPreference, StoredConnection,
    };
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
    fn provider_guard_skipped_only_for_native_rdp_without_proxy() {
        // The guard must follow the shared compile-time marker rather than a
        // local feature: `remote_desktop_view/windows-native-rdp` enables
        // `remote_desktop/windows-native-rdp`, which this predicate reads.
        let connection = StoredConnection::new_remote_desktop(
            "native".to_string(),
            remote_desktop_params(RemoteDesktopBackendPreference::WindowsNative, None),
            None,
        );

        assert_eq!(
            remote_desktop::windows_native_rdp_compiled(),
            super::connection_skips_remote_desktop_provider_guard(
                &connection,
                RemoteDesktopProtocol::Rdp
            )
        );
    }

    #[test]
    fn provider_guard_kept_for_canvas_preference() {
        let connection = StoredConnection::new_remote_desktop(
            "canvas".to_string(),
            remote_desktop_params(RemoteDesktopBackendPreference::Canvas, None),
            None,
        );

        assert!(!super::connection_skips_remote_desktop_provider_guard(
            &connection,
            RemoteDesktopProtocol::Rdp
        ));
    }

    #[test]
    fn provider_guard_kept_for_auto_preference() {
        // Auto may fall back to the Canvas (helper) presentation when the
        // native backend is unavailable, so the provider guard stays on.
        let connection = StoredConnection::new_remote_desktop(
            "auto".to_string(),
            remote_desktop_params(RemoteDesktopBackendPreference::Auto, None),
            None,
        );

        assert!(!super::connection_skips_remote_desktop_provider_guard(
            &connection,
            RemoteDesktopProtocol::Rdp
        ));
    }

    #[test]
    fn provider_guard_kept_when_proxy_is_configured() {
        // A SOCKS/HTTP proxy forces the Canvas (helper) presentation even
        // with an explicit WindowsNative preference, so the helper must be
        // installed before the connection can open.
        let connection = StoredConnection::new_remote_desktop(
            "proxied".to_string(),
            remote_desktop_params(
                RemoteDesktopBackendPreference::WindowsNative,
                Some(ProxyConfig {
                    proxy_type: ProxyType::Socks5,
                    host: "127.0.0.1".to_string(),
                    port: 7897,
                    username: None,
                    password: None,
                    credential_reference: None,
                }),
            ),
            None,
        );

        assert!(!super::connection_skips_remote_desktop_provider_guard(
            &connection,
            RemoteDesktopProtocol::Rdp
        ));
    }

    #[test]
    fn provider_guard_kept_for_vnc_protocol() {
        let connection = StoredConnection::new_remote_desktop(
            "vnc".to_string(),
            remote_desktop_params(RemoteDesktopBackendPreference::WindowsNative, None),
            None,
        );

        assert!(!super::connection_skips_remote_desktop_provider_guard(
            &connection,
            RemoteDesktopProtocol::Vnc
        ));
    }

    #[test]
    fn provider_guard_kept_for_unparseable_params() {
        let mut connection = StoredConnection::new_remote_desktop(
            "broken".to_string(),
            remote_desktop_params(RemoteDesktopBackendPreference::WindowsNative, None),
            None,
        );
        connection.params = "not-json".to_string();

        assert!(!super::connection_skips_remote_desktop_provider_guard(
            &connection,
            RemoteDesktopProtocol::Rdp
        ));
    }

    fn remote_desktop_params(
        backend_preference: RemoteDesktopBackendPreference,
        proxy: Option<ProxyConfig>,
    ) -> one_core::storage::RemoteDesktopParams {
        one_core::storage::RemoteDesktopParams {
            protocol: one_core::storage::RemoteDesktopProtocol::Rdp,
            host: "127.0.0.1".to_string(),
            port: 3389,
            username: None,
            password: None,
            credential_reference: None,
            domain: None,
            read_only: false,
            audio_playback: false,
            proxy,
            backend_preference,
            rdp: None,
        }
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
