use std::sync::Arc;

use anyhow::Result;
use dashmap::DashMap;
use gpui::Global;
use parking_lot::RwLock;

use super::connector::{LlmConnector, LlmProvider};
use super::onet_cli_provider::OnetCliLLMProvider;
use super::types::{ProviderConfig, ProviderType};
use crate::cloud_sync::client::CloudApiClient;
use crate::settings::GlobalProxySettings;

struct ProviderCacheEntry {
    signature: String,
    provider: Arc<dyn LlmProvider>,
}

pub struct ProviderManager {
    providers: Arc<DashMap<i64, ProviderCacheEntry>>,
    cloud_client: RwLock<Option<Arc<dyn CloudApiClient>>>,
    proxy_url: RwLock<Option<String>>,
}

impl ProviderManager {
    pub fn new() -> Self {
        Self {
            providers: Arc::new(DashMap::new()),
            cloud_client: RwLock::new(None),
            proxy_url: RwLock::new(None),
        }
    }

    /// 设置云端 API 客户端（用于 OnetCli Provider）
    pub fn set_cloud_client(&self, client: Arc<dyn CloudApiClient>) {
        *self.cloud_client.write() = Some(client);
    }

    pub fn set_proxy_url(&self, proxy_url: Option<String>) {
        let proxy_url = proxy_url
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty());
        let mut current = self.proxy_url.write();
        if *current != proxy_url {
            *current = proxy_url;
            self.clear_cache();
        }
    }

    pub fn proxy_url(&self) -> Option<String> {
        self.proxy_url.read().clone()
    }

    pub async fn get_provider(&self, config: &ProviderConfig) -> Result<Arc<dyn LlmProvider>> {
        let id = config.id;
        let proxy_url = self.proxy_url();
        let signature = provider_cache_signature(config, proxy_url.as_deref());

        if let Some(entry) = self.providers.get(&id)
            && entry.signature == signature
        {
            return Ok(Arc::clone(&entry.provider));
        }

        if !config.enabled {
            anyhow::bail!("Provider is disabled: {}", id);
        }

        let provider: Arc<dyn LlmProvider> = match config.provider_type {
            ProviderType::OnetCli => {
                let cloud_client = self.cloud_client.read().clone().ok_or_else(|| {
                    anyhow::anyhow!("CloudApiClient not set for OnetCli provider")
                })?;

                let onet_provider = OnetCliLLMProvider::new(cloud_client);

                Arc::new(onet_provider)
            }
            _ => {
                let connector = LlmConnector::from_config_with_proxy(config, proxy_url.as_deref())?;
                Arc::new(connector)
            }
        };

        self.providers.insert(
            id,
            ProviderCacheEntry {
                signature,
                provider: Arc::clone(&provider),
            },
        );

        Ok(provider)
    }

    pub fn remove_provider(&self, id: i64) {
        self.providers.remove(&id);
    }

    pub fn clear_cache(&self) {
        self.providers.clear();
    }
}

impl Default for ProviderManager {
    fn default() -> Self {
        Self::new()
    }
}

fn provider_cache_signature(config: &ProviderConfig, proxy_url: Option<&str>) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        config.provider_type.as_str(),
        config.model,
        config.api_base.as_deref().unwrap_or_default(),
        config.api_version.as_deref().unwrap_or_default(),
        config.api_key.as_deref().unwrap_or_default(),
        config.enabled,
        config.name,
        proxy_url.unwrap_or_default(),
    )
}

pub struct GlobalProviderState {
    manager: Arc<ProviderManager>,
}

impl Clone for GlobalProviderState {
    fn clone(&self) -> Self {
        Self {
            manager: Arc::clone(&self.manager),
        }
    }
}

impl GlobalProviderState {
    pub fn new() -> Self {
        Self {
            manager: Arc::new(ProviderManager::new()),
        }
    }

    pub fn manager(&self) -> Arc<ProviderManager> {
        Arc::clone(&self.manager)
    }

    /// 设置云端 API 客户端
    pub fn set_cloud_client(&self, client: Arc<dyn CloudApiClient>) {
        self.manager.set_cloud_client(client);
    }

    pub fn set_proxy_settings(&self, proxy_settings: &GlobalProxySettings) -> Result<(), String> {
        self.manager.set_proxy_url(
            proxy_settings
                .to_proxy_url()?
                .map(|proxy_url| proxy_url.to_string()),
        );
        Ok(())
    }

    pub fn set_proxy_url(&self, proxy_url: Option<String>) {
        self.manager.set_proxy_url(proxy_url);
    }
}

impl Default for GlobalProviderState {
    fn default() -> Self {
        Self::new()
    }
}

impl Global for GlobalProviderState {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_cache_signature_changes_with_model() {
        let base = ProviderConfig {
            id: 1,
            provider_type: ProviderType::Aliyun,
            name: "aliyun".to_string(),
            api_key: Some("sk-test".to_string()),
            model: "qwen-plus".to_string(),
            ..Default::default()
        };
        let mut changed = base.clone();
        changed.model = "qwen3.5-plus".to_string();

        assert_ne!(
            provider_cache_signature(&base, None),
            provider_cache_signature(&changed, None)
        );
    }

    #[test]
    fn provider_cache_signature_changes_with_proxy() {
        let config = ProviderConfig {
            id: 1,
            provider_type: ProviderType::OpenAI,
            name: "openai".to_string(),
            api_key: Some("sk-test".to_string()),
            model: "gpt-4o-mini".to_string(),
            ..Default::default()
        };

        assert_ne!(
            provider_cache_signature(&config, None),
            provider_cache_signature(&config, Some("socks5://127.0.0.1:1080"))
        );
    }

    #[test]
    fn provider_manager_tracks_current_proxy_url() {
        let manager = ProviderManager::new();

        assert_eq!(manager.proxy_url(), None);

        manager.set_proxy_url(Some("http://127.0.0.1:7890".to_string()));
        assert_eq!(
            manager.proxy_url(),
            Some("http://127.0.0.1:7890".to_string())
        );

        manager.set_proxy_url(None);
        assert_eq!(manager.proxy_url(), None);
    }

    #[test]
    fn global_provider_state_applies_enabled_changed_and_disabled_proxy_settings() {
        let state = GlobalProviderState::new();
        let enabled = GlobalProxySettings {
            enabled: true,
            proxy_type: crate::settings::ProxyType::Http,
            host: "127.0.0.1".to_string(),
            port: 7890,
            ..Default::default()
        };
        let changed = GlobalProxySettings {
            enabled: true,
            proxy_type: crate::settings::ProxyType::Http,
            host: "127.0.0.1".to_string(),
            port: 7891,
            ..Default::default()
        };
        let disabled = GlobalProxySettings {
            enabled: false,
            ..changed.clone()
        };

        state.set_proxy_settings(&enabled).unwrap();
        assert_eq!(
            state.manager().proxy_url(),
            Some("http://127.0.0.1:7890/".to_string())
        );

        state.set_proxy_settings(&changed).unwrap();
        assert_eq!(
            state.manager().proxy_url(),
            Some("http://127.0.0.1:7891/".to_string())
        );

        state.set_proxy_settings(&disabled).unwrap();
        assert_eq!(state.manager().proxy_url(), None);
    }
}
