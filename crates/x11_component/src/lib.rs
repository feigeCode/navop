//! 内置与本机 X11 服务的解析、验签安装及进程生命周期。

use serde::{Deserialize, Serialize};

mod archive;
mod bundle;
mod command;
mod download;
mod install_work;
mod installer;
mod launch;
mod lock;
mod manifest;
mod metrics;
mod paths;
mod process;
mod resolver;
mod restart;
mod rollout;
mod runtime;
mod signature;
mod state;
mod supervisor;
#[cfg(test)]
mod test_support;
mod vulkan;
mod xauth;

pub use installer::{install_with_config, install_with_config_for_mode, redownload_with_config};
pub use metrics::{X11ComponentMetrics, metrics};
pub use resolver::{installed_version, redownload, resolve_server};
pub use signature::verify_ed25519;
pub use state::{
    configured_mode, current_status, resolution_generation, set_configured_mode, set_data_dir,
    set_navop_version, take_restart_notification, take_restart_notification_for,
};

/// 固定组件 Release 的清单地址，不发现 tag、不追踪 latest。
pub const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/feigeCode/yserver/releases/download/navop-v1.9.0-1/navop-x11-manifest.json";
/// 与程序更新公钥独立；可通过构建环境覆盖，仓库内固定公钥用于离线内嵌组件。
pub const X11_PUBLIC_KEY_BASE64: &str = match option_env!("NAVOP_X11_PUBLIC_KEY_BASE64") {
    Some(key) => key,
    None => include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../resources/windows/x11-component/public-key.txt"
    )),
};

/// X11 服务选择策略。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum X11ServerMode {
    #[default]
    Auto,
    External,
    Embedded,
}

/// X11 转发当前状态。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum X11ForwardingStatus {
    External { server: String },
    Embedded { version: String },
    Disabled { reason: String },
}

/// 后台崩溃重启通知；generation 单调递增，UI 消费后从队列移除。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct X11RestartNotification {
    pub generation: u64,
    pub reason: String,
}

/// X11 组件操作错误。
#[derive(Debug, thiserror::Error)]
pub enum X11ComponentError {
    #[error("当前平台不支持内置 X11 服务：{0}")]
    UnsupportedPlatform(String),
    #[error("组件清单无效：{0}")]
    InvalidManifest(String),
    #[error("当前平台没有 X11 组件：{0}")]
    MissingArtifact(String),
    #[error("下载 X11 组件失败：{0}")]
    Download(String),
    #[error("X11 组件 SHA256 不匹配")]
    Sha256Mismatch { expected: String, actual: String },
    #[error("X11 组件 Ed25519 验签失败：{0}")]
    SignatureInvalid(String),
    #[error("X11 组件安装失败：{0}")]
    Install(String),
    #[error("Vulkan 能力不满足要求：需要 {required}，实际 {actual:?}")]
    VulkanUnavailable {
        required: String,
        actual: Option<String>,
    },
    #[error("X11 握手失败：{0}")]
    Handshake(String),
    #[error("内置 X11 服务已禁用：{0}")]
    EmbeddedDisabled(String),
    #[error("另一 Navop 实例已启动内置 X11 服务：{0}")]
    ActiveOtherInstance(String),
    #[error("X11 服务进程失败：{0}")]
    Process(String),
    #[error("X11 配置失败：{0}")]
    Config(String),
}

/// X11 解析结果。
#[derive(Debug)]
pub struct X11Resolution {
    pub proxy: Option<x11_forwarding::X11Proxy>,
    pub status: X11ForwardingStatus,
}

/// 下载与安装所需配置；集成测试通过它注入隔离根目录与密钥。
#[derive(Clone, Debug)]
pub struct InstallConfig {
    pub data_dir: std::path::PathBuf,
    pub manifest_url: String,
    pub public_key_base64: String,
    pub navop_version: semver::Version,
    pub target: String,
    /// 测试使用直连客户端，生产配置必须保持 false 以继承系统代理。
    pub direct_http: bool,
}
