//! 应用配置模块
//!
//! 通过 `build.rs` 在编译时将环境变量内嵌到二进制文件中，
//! 运行时也可通过同名环境变量覆盖。
//!
//! # 配置优先级
//! 1. 运行时环境变量（最高优先级，用于开发调试）
//! 2. 编译时环境变量（发布版本内置，由 build.rs 通过 cargo:rustc-env 注入）
//!
//! # 发版构建
//! ```bash
//! SUPABASE_URL=https://xxx.supabase.co \
//! SUPABASE_ANON_KEY=eyJ... \
//! cargo build --release
//! ```

/// Supabase 配置
#[derive(Debug, Clone)]
pub struct SupabaseConfig {
    /// 项目 URL
    pub project_url: String,
    /// API Key (anon key)
    pub api_key: String,
}

/// 应用更新配置
#[derive(Debug, Clone)]
pub struct UpdateConfig {
    /// 版本检查接口地址
    pub update_url: String,
    /// 更新下载页地址（可选）
    pub download_url: Option<String>,
}

pub const DEFAULT_TEAM_MANAGEMENT_URL_TEMPLATE: &str = "/zh-CN/auth/desktop?access_token={access-token}&refresh_token={refresh-token}&next=/zh-CN/dashboard";

pub fn public_base_url_from_parts(
    runtime: Option<&str>,
    build_time: Option<&str>,
) -> Option<String> {
    trimmed_value(runtime)
        .or_else(|| trimmed_value(build_time))
        .map(|value| value.trim_end_matches('/').to_string())
}

pub fn public_base_url() -> Option<String> {
    let runtime = std::env::var("ONETCLI_PUBLIC_BASE_URL").ok();
    public_base_url_from_parts(runtime.as_deref(), option_env!("ONETCLI_PUBLIC_BASE_URL"))
}

pub fn update_url_from_public_base(base_url: &str) -> String {
    format!(
        "{}/updates/latest.json",
        base_url.trim().trim_end_matches('/')
    )
}

pub fn team_management_url_template() -> String {
    runtime_env("ONETCLI_TEAM_MANAGEMENT_URL_TEMPLATE")
        .or_else(|| trimmed_value(option_env!("ONETCLI_TEAM_MANAGEMENT_URL_TEMPLATE")))
        .unwrap_or_else(|| DEFAULT_TEAM_MANAGEMENT_URL_TEMPLATE.to_string())
}

impl UpdateConfig {
    /// 获取更新配置
    ///
    /// 优先级：运行时环境变量 > 编译时环境变量
    pub fn get() -> Self {
        Self {
            update_url: Self::get_update_url(),
            download_url: Self::get_download_url(),
        }
    }

    /// 获取更新接口地址
    fn get_update_url() -> String {
        if let Some(url) = runtime_env("ONETCLI_UPDATE_URL") {
            return url;
        }

        if let Some(url) = trimmed_value(option_env!("ONETCLI_UPDATE_URL")) {
            return url;
        }

        public_base_url()
            .map(|base_url| update_url_from_public_base(&base_url))
            .unwrap_or_default()
    }

    /// 获取下载页地址
    fn get_download_url() -> Option<String> {
        if let Some(url) = runtime_env("ONETCLI_UPDATE_DOWNLOAD_URL") {
            return Some(url);
        }

        trimmed_value(option_env!("ONETCLI_UPDATE_DOWNLOAD_URL"))
    }

    /// 检查配置是否有效
    pub fn is_valid(&self) -> bool {
        !self.update_url.trim().is_empty()
    }
}

fn runtime_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .and_then(|value| trimmed_value(Some(&value)))
}

fn trimmed_value(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self::get()
    }
}

impl SupabaseConfig {
    /// 获取 Supabase 配置
    ///
    /// 优先级：运行时环境变量 > 编译时环境变量
    pub fn get() -> Self {
        Self {
            project_url: Self::get_url(),
            api_key: Self::get_api_key(),
        }
    }

    /// 获取项目 URL
    fn get_url() -> String {
        // 运行时环境变量优先
        if let Ok(url) = std::env::var("SUPABASE_URL") {
            if !url.is_empty() {
                return url;
            }
        }

        // 编译时环境变量
        option_env!("SUPABASE_URL").unwrap_or_default().to_string()
    }

    /// 获取 API Key
    fn get_api_key() -> String {
        // 运行时环境变量优先
        if let Ok(key) = std::env::var("SUPABASE_ANON_KEY") {
            if !key.is_empty() {
                return key;
            }
        }

        // 编译时环境变量
        option_env!("SUPABASE_ANON_KEY")
            .unwrap_or_default()
            .to_string()
    }

    /// 检查配置是否有效
    pub fn is_valid(&self) -> bool {
        !self.project_url.is_empty() && !self.api_key.is_empty()
    }
}

impl Default for SupabaseConfig {
    fn default() -> Self {
        Self::get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_get() {
        let config = SupabaseConfig::get();
        // 测试环境可能没有配置，只验证不会 panic
        let _ = config.is_valid();
    }

    #[test]
    fn test_update_config_get() {
        let config = UpdateConfig::get();
        let _ = config.is_valid();
    }

    #[test]
    fn update_url_from_public_base_points_to_r2_manifest() {
        assert_eq!(
            "https://onetcli.test.cn/updates/latest.json",
            update_url_from_public_base("https://onetcli.test.cn")
        );
    }

    #[test]
    fn update_url_from_public_base_trims_trailing_slash() {
        assert_eq!(
            "https://onetcli.test.cn/updates/latest.json",
            update_url_from_public_base("https://onetcli.test.cn/")
        );
    }

    #[test]
    fn public_base_url_from_parts_has_no_built_in_default() {
        assert_eq!(None, public_base_url_from_parts(None, None));
        assert_eq!(
            Some("https://onetcli.test.cn".to_string()),
            public_base_url_from_parts(None, Some("https://onetcli.test.cn"))
        );
    }
}
