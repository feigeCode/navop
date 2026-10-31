//! 单实例门禁：Windows 命名管道实现已下沉到
//! `app_kit::integration::single_instance`，main 侧只剩按值签名的适配。
//!
//! 全量产品沿用既有语义：名字由**配置目录**派生（`instance_name_for_config_dir`），
//! 所以升级后新旧进程仍能互相识别；独立产品改用 `acquire_for_product(app_id)`，
//! 三套产品各用各的名字，互不干扰。
//!
//! 互斥、补建下一实例、I/O 期限这些实现细节与它们的守卫测试都在 app_kit 里。

#[cfg(target_os = "windows")]
pub(crate) use app_kit::integration::single_instance::{
    SingleInstanceError, SingleInstanceOutcome, StartupRequest,
};

/// 抢主实例资格；名字已被占用时把启动请求转发给已经在跑的实例。
///
/// main 的调用点按值构造 `StartupRequest`（转发后即丢弃），app_kit 的实现按引用
/// 读取；这里只做签名适配，不改变任何行为。
#[cfg(target_os = "windows")]
pub(crate) fn claim_or_forward(
    config_dir: &std::path::Path,
    request: StartupRequest,
    on_request: impl Fn(StartupRequest) -> bool + Send + Sync + 'static,
) -> Result<SingleInstanceOutcome, SingleInstanceError> {
    app_kit::integration::single_instance::claim_or_forward(config_dir, &request, on_request)
}

#[cfg(test)]
mod tests {
    const WINDOWS_SOURCE: &str =
        include_str!("../../../crates/app_kit/src/integration/single_instance/windows.rs");
    const SHIM_SOURCE: &str = include_str!("windows_single_instance.rs");

    #[test]
    fn windows_single_instance_transport_really_lives_in_app_kit() {
        // 守卫必须盯真实实现：互斥语义、名字派生、转发循环都只能在 app_kit 里。
        for needle in [
            "FILE_FLAG_FIRST_PIPE_INSTANCE",
            "pub fn claim_or_forward(",
            "fn is_name_in_use(",
            "Sha256::digest",
            "pub fn pipe_name(",
        ] {
            assert!(
                WINDOWS_SOURCE.contains(needle),
                "app_kit 的 Windows 单实例实现缺少 {needle}"
            );
        }
    }

    #[test]
    fn windows_single_instance_shim_keeps_the_config_dir_scoped_signature() {
        // lib.rs 的源码守卫按 `windows_single_instance::claim_or_forward(` 定位调用点，
        // shim 必须保持这个名字，并且仍然按配置目录隔离。
        assert!(SHIM_SOURCE.contains("pub(crate) fn claim_or_forward("));
        assert!(SHIM_SOURCE.contains("config_dir: &std::path::Path"));
        assert!(
            SHIM_SOURCE.contains(
                "app_kit::integration::single_instance::claim_or_forward(config_dir, &request, on_request)"
            ),
            "shim 必须把请求转发给 app_kit 的真实实现"
        );
    }
}
