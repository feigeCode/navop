//! 字体选项：界面与终端共用的等宽字体清单与字号范围。

/// 最小字号。
pub const MIN_FONT_SIZE: f32 = 8.0;
/// 最大字号。
pub const MAX_FONT_SIZE: f32 = 32.0;

/// 获取可用的等宽字体列表（按操作系统优化排序）。
pub fn available_monospace_fonts() -> Vec<&'static str> {
    if cfg!(target_os = "macos") {
        vec![
            "Menlo",
            "Monaco",
            "SF Mono",
            "Courier New",
            "Fira Code",
            "JetBrains Mono",
            "Source Code Pro",
            "Cascadia Code",
            "Hack",
            "IBM Plex Mono",
        ]
    } else if cfg!(target_os = "windows") {
        vec![
            "Consolas",
            "Cascadia Mono",
            "Cascadia Code",
            "Courier New",
            "Lucida Console",
            "Fira Code",
            "JetBrains Mono",
            "Source Code Pro",
            "Hack",
            "IBM Plex Mono",
        ]
    } else {
        vec![
            "DejaVu Sans Mono",
            "Ubuntu Mono",
            "Liberation Mono",
            "Courier New",
            "Fira Code",
            "JetBrains Mono",
            "Source Code Pro",
            "Cascadia Code",
            "Hack",
            "IBM Plex Mono",
        ]
    }
}
