//! 桌面系统集成：窗口状态、系统托盘、单实例、菜单与全局快捷键。
//!
//! 这一层只依赖 `one_core` 与 `gpui`，不依赖任何领域 crate，因此三个产品都能用。

/// 系统桌面通知（Windows 原生 Toast，点击可唤醒主窗口并跳转到目标标签页）。
pub mod desktop_notification;
/// 窗口尺寸与位置持久化。
pub mod shortcuts;
pub mod single_instance;
pub mod tray;
pub mod window_state;
