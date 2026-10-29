//! 产品能力边界。
//!
//! 三产品共用同一套 UI 与初始化流程，但独立产品只装配自己的领域包。界面上凡是
//! 「这类连接在当前产品里能不能用」的判断都走这里：能力来自 `domain_api` 的
//! 领域注册表，而不是散落各处的 `cfg!` 或产品名比较。

use domain_api::AppIdentity;
use one_core::storage::models::ConnectionType;

/// 当前进程是否是全量产品。
///
/// 身份未配置时按全量产品对待：这是 main 自己的测试与嵌入场景，行为保持原样。
pub(crate) fn is_full_product() -> bool {
    app_kit::identity().is_none_or(|identity| identity.app_id == AppIdentity::FULL.app_id)
}

/// 当前产品是否支持某类连接。
///
/// 内置类型以「领域包是否注册了打开入口」为准：`navop-shell` 没有数据库领域，
/// 数据库连接就不应出现在列表与新建入口里；扩展连接由扩展运行时统一打开，与
/// 产品无关，始终保留。
pub(crate) fn connection_type_supported(connection_type: ConnectionType) -> bool {
    if is_full_product() {
        return true;
    }
    if connection_type == ConnectionType::Extension {
        return true;
    }
    app_kit::registry().has_connection_opener(connection_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfigured_identity_behaves_like_the_full_product() {
        // 测试进程可能已经配置过身份（并行跑 main 的其它测试），这里只断言
        // 全量身份下一切内置类型都可用，避免依赖全局初始化顺序。
        let identity = app_kit::identity();
        if identity.is_none_or(|identity| identity.app_id == AppIdentity::FULL.app_id) {
            assert!(connection_type_supported(ConnectionType::Database));
            assert!(connection_type_supported(ConnectionType::SshSftp));
        }
        assert!(connection_type_supported(ConnectionType::Extension));
    }
}
