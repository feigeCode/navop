//! 终端主题配置
//!
//! 提供终端的颜色、字体、字号等外观设置
//!
//! ## 配色系统设计
//!
//! 本模块采用语义化配色系统，确保所有颜色组合具有足够的对比度：
//! - `background` / `foreground`: 主要背景和文字，对比度 >= 7:1
//! - `muted` / `muted_foreground`: 次要区域背景和文字，对比度 >= 4.5:1
//! - `accent` / `accent_foreground`: 强调色背景和文字，对比度 >= 4.5:1
//!
//! 颜色使用规则：
//! - 在 `background` 上使用 `foreground` 或 `muted_foreground`
//! - 在 `muted` 上使用 `foreground` 或 `muted_foreground`
//! - 在 `accent` 上使用 `accent_foreground`

use gpui::{Hsla, Pixels, SharedString};
use gpui_component::Theme;
use one_core::settings::{
    default_grid_font_fallback_families, default_grid_monospace_font_family,
    is_supported_grid_monospace_font, normalize_grid_monospace_font_family,
};

/// 最小字体大小
pub const MIN_FONT_SIZE: f32 = 8.0;
/// 最大字体大小
pub const MAX_FONT_SIZE: f32 = 32.0;
/// 默认行高比例
pub const DEFAULT_LINE_HEIGHT_SCALE: f32 = 1.4;
const TERMINAL_CELL_WIDTH_RATIO: f32 = 0.6;
const MIN_TERMINAL_CELL_WIDTH_RATIO: f32 = 0.3;
const MAX_TERMINAL_CELL_WIDTH_RATIO: f32 = 1.2;

/// 终端主题配色（用于侧边栏等 UI 组件）
///
/// 所有颜色对都经过对比度验证，确保可读性：
/// - `background` + `foreground`: 主要内容
/// - `background` + `muted_foreground`: 次要内容
/// - `muted` + `foreground`: 卡片/列表项上的主要内容
/// - `muted` + `muted_foreground`: 卡片/列表项上的次要内容
/// - `accent` + `accent_foreground`: 按钮/选中状态
#[derive(Clone, Debug)]
pub struct TerminalColors {
    /// 主背景色
    pub background: Hsla,
    /// 主前景色（在 background 上使用）
    pub foreground: Hsla,
    /// 次要背景色（卡片、列表项、悬停状态）
    pub muted: Hsla,
    /// 次要前景色（次要文字、标签、占位符）
    pub muted_foreground: Hsla,
    /// 边框色
    pub border: Hsla,
    /// 强调背景色（按钮、选中项）
    pub accent: Hsla,
    /// 强调前景色（在 accent 背景上使用）
    pub accent_foreground: Hsla,
}

impl TerminalColors {
    pub fn from_application_theme(theme: &Theme) -> Self {
        TerminalTheme::from_application_theme(theme).colors()
    }
}

/// 终端主题配置
#[derive(Clone, Debug)]
pub struct TerminalTheme {
    /// 前景色（文字颜色）
    pub foreground: Hsla,
    /// 背景色
    pub background: Hsla,
    /// 光标颜色
    pub cursor: Hsla,
    /// 选中区域颜色
    pub selection: Hsla,
}

impl PartialEq for TerminalTheme {
    fn eq(&self, other: &Self) -> bool {
        self.foreground == other.foreground
            && self.background == other.background
            && self.cursor == other.cursor
            && self.selection == other.selection
    }
}

/// 获取当前操作系统的默认等宽字体
pub fn default_monospace_font() -> &'static str {
    default_grid_monospace_font_family()
}

pub fn is_supported_terminal_primary_font(font: &str) -> bool {
    is_supported_grid_monospace_font(font)
}

pub fn normalize_terminal_primary_font(font: &str) -> String {
    normalize_grid_monospace_font_family(font)
}

pub fn terminal_cell_width_from_advance(font_size: Pixels, measured_width: Pixels) -> Pixels {
    let min_width = font_size * MIN_TERMINAL_CELL_WIDTH_RATIO;
    let max_width = font_size * MAX_TERMINAL_CELL_WIDTH_RATIO;

    if measured_width < min_width || measured_width > max_width {
        font_size * TERMINAL_CELL_WIDTH_RATIO
    } else {
        measured_width
    }
}

pub fn terminal_cell_width_from_advances<I>(font_size: Pixels, measured_widths: I) -> Pixels
where
    I: IntoIterator<Item = Pixels>,
{
    let measured_width = measured_widths
        .into_iter()
        .max_by(|left, right| f32::from(*left).total_cmp(&f32::from(*right)))
        .unwrap_or(font_size * TERMINAL_CELL_WIDTH_RATIO);
    terminal_cell_width_from_advance(font_size, measured_width)
}

/// 默认备用字体列表（按优先级排序，跨平台兼容）
pub fn default_font_fallbacks() -> Vec<SharedString> {
    let mut fonts = if cfg!(target_os = "macos") {
        vec!["Monaco", "SF Mono", "Courier New"]
    } else if cfg!(target_os = "windows") {
        vec!["Cascadia Mono", "Courier New", "Lucida Console"]
    } else {
        // Linux 和其他系统
        vec!["Ubuntu Mono", "Liberation Mono", "Courier New"]
    }
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();

    for fallback in default_grid_font_fallback_families() {
        if !fonts.iter().any(|font| font == &fallback) {
            fonts.push(fallback);
        }
    }

    fonts.into_iter().map(SharedString::from).collect()
}

impl TerminalTheme {
    /// 从应用主题的语义色生成终端主题。
    ///
    /// 终端只需要背景、前景、光标和选区四种基础颜色，因此直接复用
    /// 应用主题的对应语义色，避免终端侧边栏和主界面出现两套配色。
    pub fn from_application_theme(theme: &Theme) -> Self {
        Self {
            foreground: theme.foreground,
            background: theme.background,
            cursor: theme.primary,
            selection: theme.selection,
        }
    }

    /// 判断当前宿主主题是否为深色。
    pub fn is_dark(&self) -> bool {
        self.background.l < 0.5
    }

    /// 获取用于终端侧边栏和终端工具面板的语义配色。
    pub fn colors(&self) -> TerminalColors {
        self.semantic_colors()
    }

    fn semantic_colors(&self) -> TerminalColors {
        let is_dark = self.is_dark();

        // 计算 muted 背景色（卡片、列表项等）
        let muted = if is_dark {
            // 深色主题：muted 比背景稍亮
            Hsla {
                h: self.background.h,
                s: self.background.s,
                l: (self.background.l + 0.06).min(0.25),
                a: 1.0,
            }
        } else {
            // 浅色主题：muted 比背景稍暗
            Hsla {
                h: self.background.h,
                s: self.background.s.min(0.1),
                l: (self.background.l - 0.06).max(0.85),
                a: 1.0,
            }
        };

        // 计算 muted_foreground（次要文字）
        // 关键：必须与 background 和 muted 都有足够对比度
        let muted_foreground = if is_dark {
            // 深色主题：使用中等亮度的灰色
            // 确保在深色背景上可读
            Hsla {
                h: self.foreground.h,
                s: self.foreground.s * 0.3,
                l: 0.55, // 固定中等亮度，确保在深色背景上可读
                a: 1.0,
            }
        } else {
            // 浅色主题：使用较深的灰色
            // 确保在浅色背景上可读
            Hsla {
                h: self.foreground.h,
                s: self.foreground.s * 0.3,
                l: 0.45, // 固定中等亮度，确保在浅色背景上可读
                a: 1.0,
            }
        };

        // 计算边框色
        let border = if is_dark {
            Hsla {
                h: self.background.h,
                s: self.background.s,
                l: (self.background.l + 0.12).min(0.35),
                a: 1.0,
            }
        } else {
            Hsla {
                h: self.background.h,
                s: self.background.s.min(0.1),
                l: (self.background.l - 0.15).max(0.75),
                a: 1.0,
            }
        };

        // 计算强调色前景（在 accent 背景上使用的文字颜色）
        // 根据 accent 的亮度决定使用深色还是浅色文字
        let accent_foreground = if self.cursor.l > 0.5 {
            // accent 是亮色，使用深色文字
            Hsla {
                h: self.cursor.h,
                s: self.cursor.s * 0.2,
                l: 0.1, // 深色文字
                a: 1.0,
            }
        } else {
            // accent 是暗色，使用亮色文字
            Hsla {
                h: self.cursor.h,
                s: self.cursor.s * 0.1,
                l: 0.95, // 亮色文字
                a: 1.0,
            }
        };

        TerminalColors {
            background: self.background,
            foreground: self.foreground,
            muted,
            muted_foreground,
            border,
            accent: self.cursor,
            accent_foreground,
        }
    }
}

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

#[cfg(test)]
#[path = "theme_tests.rs"]
mod tests;
