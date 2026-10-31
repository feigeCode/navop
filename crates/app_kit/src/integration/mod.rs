//! 桌面系统集成：窗口状态、系统托盘、单实例、菜单与全局快捷键。
//!
//! 这一层只依赖 `one_core` 与 `gpui`，不依赖任何领域 crate，因此三个产品都能用。

/// 窗口尺寸与位置持久化。
pub mod shortcuts;
pub mod single_instance;
pub mod tray;
pub mod window_state;
