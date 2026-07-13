//! License 模块
//!
//! 提供付费功能控制和订阅管理。
//!
//! # 概述
//!
//! 本模块实现混合模式的 License 验证系统：
//! - 服务端验证：登录后从 Supabase 获取订阅状态
//! - 本地缓存：支持离线使用（7 天有效期）
//!
//! # 使用方式
//!
//! ```rust,ignore
//! use one_core::license::{LicenseService, Feature, PlanTier};
//! use std::sync::Arc;
//!
//! // 创建服务
//! let storage = Arc::new(LocalLicenseStorage);
//! let service = LicenseService::new(storage);
//!
//! // 尝试从缓存恢复
//! service.restore_from_cache();
//!
//! // 检查功能权限
//! if service.is_feature_enabled(Feature::CloudSync) {
//!     // 执行云同步
//! }
//! ```

mod error;
mod models;
mod service;
mod storage;

use std::sync::Arc;

use gpui::App;

pub use error::LicenseError;
pub use models::{
    Feature, LicenseInfo, OfflineLicenseDocument, OfflineLicensePayload, PlanTier,
    SubscriptionInfo, SubscriptionStatus,
};
pub use service::LicenseService;
pub use storage::{LicenseStorage, LocalLicenseStorage};

#[derive(Clone)]
pub struct GlobalLicenseService(pub Arc<LicenseService>);

impl gpui::Global for GlobalLicenseService {}

pub fn global_license_service(cx: &App) -> Option<Arc<LicenseService>> {
    cx.try_global::<GlobalLicenseService>()
        .map(|global| global.0.clone())
}

pub fn is_feature_enabled(feature: Feature, cx: &App) -> bool {
    global_license_service(cx).is_some_and(|service| service.is_feature_enabled(feature))
}

#[cfg(test)]
mod global_tests {
    use std::sync::Arc;

    use gpui::{AppContext, TestAppContext};

    use super::{
        Feature, GlobalLicenseService, LicenseError, LicenseInfo, LicenseService, LicenseStorage,
        SubscriptionInfo, is_feature_enabled,
    };

    struct MemoryStorage;

    impl LicenseStorage for MemoryStorage {
        fn name(&self) -> &'static str {
            "memory"
        }

        fn save(&self, _: &LicenseInfo) -> Result<(), LicenseError> {
            Ok(())
        }

        fn load(&self) -> Option<LicenseInfo> {
            None
        }

        fn delete(&self) -> Result<(), LicenseError> {
            Ok(())
        }

        fn exists(&self) -> bool {
            false
        }
    }

    #[gpui::test]
    fn global_feature_gate_tracks_the_current_license(cx: &mut TestAppContext) {
        cx.update(|cx| {
            assert!(!is_feature_enabled(Feature::TeamManagement, cx));

            let service = Arc::new(LicenseService::new(Arc::new(MemoryStorage)));
            service
                .update_from_subscription(
                    "user-1".to_string(),
                    Some(SubscriptionInfo {
                        plan: "pro".to_string(),
                        status: "active".to_string(),
                        expires_at: None,
                    }),
                )
                .expect("pro subscription is accepted");
            cx.set_global(GlobalLicenseService(service.clone()));

            assert!(is_feature_enabled(Feature::TeamManagement, cx));
            service.set_free();
            assert!(!is_feature_enabled(Feature::TeamManagement, cx));
        });
    }
}
