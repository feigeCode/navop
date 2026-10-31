use gpui::App;
use gpui_component::setting::SettingPage;

/// 设置分区提供方：每个分区渲染成设置页里的一个页面。
///
/// 返回 `(order, page)`，`app_kit` 会与通用页面一起交给
/// `gpui_component` 的 `Settings` 容器渲染；独立产品只展示已装配领域提供的页面。
pub trait SettingsSectionProvider: Send + Sync {
    fn pages(&self, cx: &mut App) -> Vec<(i32, SettingPage)>;
}
