use std::cell::RefCell;

use gpui::{App, ElementId, HighlightStyle, Hsla, Pixels, Rems, SharedString, StyleRefinement};
use gpui_base::{TextView, TextViewStyle};
use gpui_component::ActiveTheme;

/// navop 默认界面字号（px）：`sp()` 的书写基准。
///
/// 与 `navop settings` 的 `default_font_size()` 保持一致。术语：这里的
/// 「界面尺寸」指边框、间距、圆角、控件高度这些 chrome；**文字**已经由
/// `text_xs()/text_sm()`（rems）自己缩放，内容面（markdown 正文、代码块、
/// diff 行）保持 `px` 不参与缩放。
const UI_BASE_FONT_SIZE: f32 = 14.0;

/// 按 14px 基准书写的界面尺寸 → rem，随「字体大小 × 界面缩放」缩放。
///
/// navop 的 rem 基准取自主题字号（gpui-component 的 `Root::render` 会
/// `window.set_rem_size(cx.theme().font_size)`，而 `apply_font_settings` 把
/// 缩放倍率乘进了字号）。所以走这里的尺寸会与 gpui-component 组件内部一致地
/// 缩放；写死的 `px` 则不会。**发丝线（边框、分隔线）保持 `px`** —— 缩放会
/// 让 1px 线在非整数倍下发糊。
pub(crate) fn sp(value: f32) -> Rems {
    gpui::rems(value / UI_BASE_FONT_SIZE)
}

#[derive(Clone, Debug)]
pub struct AgentChatTheme {
    pub is_dark: bool,
    pub background: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub muted_foreground: Hsla,
    pub border: Hsla,
    pub panel: Hsla,
    pub panel_hover: Hsla,
    pub accent: Hsla,
    pub accent_foreground: Hsla,
    pub code_background: Hsla,
    pub code_foreground: Hsla,
    pub table_header: Hsla,
    pub table_row: Hsla,
    pub table_row_alt: Hsla,
    pub quote_border: Hsla,
    pub link: Hsla,
    pub text_selection: Hsla,
    pub surface_radius: Pixels,

    // ── 语义槽：面板内的层次与状态色。全部取自宿主主题，不另立调色板，
    // 这样 navop 换肤 / 深浅切换时面板自动跟随。
    /// 浮层表面：popover / 菜单 / 命令面板 / 悬浮卡。
    pub raised: Hsla,
    /// 内嵌区表面：比 `panel` 更沉，用于代码槽、inset 容器。
    pub inset: Hsla,
    /// 悬停 / 选中叠加层，半透明，画在内容之下。
    pub overlay: Hsla,
    /// 更强的叠加层（按下 / 选中强调）。
    pub overlay_strong: Hsla,
    /// 结构性分隔线：比 `border` 明确，用于面板边界 / 标题栏。
    pub border_strong: Hsla,
    /// 最弱一级文字：占位符、禁用、角落元信息。
    pub text_ghost: Hsla,
    /// 用量 / 进度条填充。
    pub gauge: Hsla,
    /// 用量接近上限时的告警填充。
    pub gauge_warning: Hsla,
    /// 用量越过上限时的危险填充。
    pub gauge_danger: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub info: Hsla,
    /// 骨架屏占位底色。
    pub skeleton: Hsla,
    /// 用量历史图表的正向 / 反向柱与网格线。
    pub chart_bullish: Hsla,
    pub chart_bearish: Hsla,
    pub chart_grid: Hsla,
}

impl AgentChatTheme {
    pub fn from_app(cx: &App) -> Self {
        let theme = cx.theme();
        Self {
            is_dark: theme.is_dark(),
            background: theme.background,
            foreground: theme.foreground,
            muted: theme.muted,
            muted_foreground: theme.muted_foreground,
            border: theme.border,
            panel: theme.muted,
            panel_hover: theme.muted.opacity(0.72),
            accent: theme.accent,
            accent_foreground: theme.accent_foreground,
            code_background: theme.muted,
            code_foreground: theme.foreground,
            table_header: theme.muted,
            table_row: theme.background,
            table_row_alt: theme.muted.opacity(0.35),
            quote_border: theme.border,
            link: theme.link,
            text_selection: theme.selection,
            surface_radius: theme.radius,
            raised: theme.colors.popover,
            inset: theme.colors.input,
            overlay: theme.colors.overlay,
            overlay_strong: theme.colors.overlay.opacity(1.6),
            border_strong: theme.colors.window_border,
            text_ghost: theme.muted_foreground.opacity(0.6),
            gauge: theme.colors.progress_bar,
            gauge_warning: theme.colors.warning,
            gauge_danger: theme.colors.danger,
            success: theme.colors.success,
            warning: theme.colors.warning,
            danger: theme.colors.danger,
            info: theme.colors.info,
            skeleton: theme.colors.skeleton,
            chart_bullish: theme.colors.chart_bullish,
            chart_bearish: theme.colors.chart_bearish,
            chart_grid: theme.colors.chart_grid,
        }
    }

    pub fn markdown_style(&self) -> TextViewStyle {
        let mut table = StyleRefinement::default();
        table.corner_radii.top_left = Some(self.surface_radius.into());
        table.corner_radii.top_right = Some(self.surface_radius.into());
        table.corner_radii.bottom_left = Some(self.surface_radius.into());
        table.corner_radii.bottom_right = Some(self.surface_radius.into());
        let mut code_block = table.clone();
        code_block.background = Some(self.code_background.into());
        code_block.text.color = Some(self.code_foreground);
        let mut table_head = StyleRefinement::default();
        table_head.background = Some(self.table_header.into());
        table_head.text.color = Some(self.foreground);
        let mut table_cell = StyleRefinement::default();
        table_cell.background = Some(self.table_row.into());
        table_cell.text.color = Some(self.foreground);
        TextViewStyle::default()
            .with_foreground(self.foreground)
            .with_muted_foreground(self.muted_foreground)
            .with_link(self.link)
            .with_selection(self.text_selection)
            .with_code_background(self.code_background)
            .with_border(self.quote_border)
            .with_code_block(code_block)
            .with_inline_code(HighlightStyle {
                color: Some(self.code_foreground),
                background_color: Some(self.code_background),
                ..Default::default()
            })
            .with_table(table)
            .with_table_head(table_head)
            .with_table_cell(table_cell)
            .with_dark(self.is_dark)
    }

    pub fn hover_background(&self) -> Hsla {
        if self.is_dark {
            self.panel_hover
        } else {
            self.accent.opacity(0.14)
        }
    }

    pub fn selection_background(&self) -> Hsla {
        self.accent.opacity(if self.is_dark { 0.22 } else { 0.30 })
    }
}

thread_local! {
    static ACTIVE_AGENT_CHAT_THEME: RefCell<Option<AgentChatTheme>> = const { RefCell::new(None) };
}

pub(crate) fn resolve_agent_chat_theme(theme: Option<&AgentChatTheme>, cx: &App) -> AgentChatTheme {
    theme
        .cloned()
        .unwrap_or_else(|| AgentChatTheme::from_app(cx))
}

pub(crate) fn active_agent_chat_theme(cx: &App) -> AgentChatTheme {
    ACTIVE_AGENT_CHAT_THEME
        .with(|theme| theme.borrow().clone())
        .unwrap_or_else(|| AgentChatTheme::from_app(cx))
}

pub(crate) fn with_agent_chat_theme<T>(theme: &AgentChatTheme, render: impl FnOnce() -> T) -> T {
    let previous = ACTIVE_AGENT_CHAT_THEME.with(|active| active.replace(Some(theme.clone())));
    let output = render();
    ACTIVE_AGENT_CHAT_THEME.with(|active| {
        active.replace(previous);
    });
    output
}

pub(crate) fn themed_markdown(
    id: impl Into<ElementId>,
    markdown: impl Into<SharedString>,
    theme: &AgentChatTheme,
) -> TextView {
    TextView::markdown(id, markdown).style(theme.markdown_style())
}

pub(crate) fn themed_html(
    id: impl Into<ElementId>,
    html: impl Into<SharedString>,
    theme: &AgentChatTheme,
) -> TextView {
    TextView::html(id, html).style(theme.markdown_style())
}

#[cfg(test)]
mod tests {
    use gpui::rgb;

    use super::*;

    fn color(hex: u32) -> Hsla {
        rgb(hex).into()
    }

    fn dark_theme() -> AgentChatTheme {
        AgentChatTheme {
            is_dark: true,
            background: color(0x020617),
            foreground: color(0xf8fafc),
            muted: color(0x0f172a),
            muted_foreground: color(0x94a3b8),
            border: color(0x334155),
            panel: color(0x0f172a),
            panel_hover: color(0x1e293b),
            accent: color(0x38bdf8),
            accent_foreground: color(0x001018),
            code_background: color(0x020617),
            code_foreground: color(0xe2e8f0),
            table_header: color(0x1e293b),
            table_row: color(0x020617),
            table_row_alt: color(0x111827),
            quote_border: color(0x475569),
            link: color(0x38bdf8),
            text_selection: color(0x164e63),
            surface_radius: gpui::px(6.0),
            raised: color(0x1e293b),
            inset: color(0x020617),
            overlay: color(0x334155),
            overlay_strong: color(0x475569),
            border_strong: color(0x475569),
            text_ghost: color(0x64748b),
            gauge: color(0x38bdf8),
            gauge_warning: color(0xf59e0b),
            gauge_danger: color(0xef4444),
            success: color(0x22c55e),
            warning: color(0xf59e0b),
            danger: color(0xef4444),
            info: color(0x38bdf8),
            skeleton: color(0x1e293b),
            chart_bullish: color(0x22c55e),
            chart_bearish: color(0xef4444),
            chart_grid: color(0x334155),
        }
    }

    #[test]
    fn markdown_style_preserves_agent_chat_palette() {
        let theme = dark_theme();
        let style = theme.markdown_style();

        assert!(style.is_dark());
        assert_eq!(style.foreground(), theme.foreground);
        assert_eq!(style.muted_foreground(), theme.muted_foreground);
        assert_eq!(style.link(), theme.link);
        assert_eq!(style.selection(), theme.text_selection);
        assert_eq!(style.code_background(), theme.code_background);
        assert_eq!(style.border(), theme.quote_border);
        assert_eq!(
            Some(theme.code_background.into()),
            style.code_block().background
        );
        assert_eq!(Some(theme.code_foreground), style.code_block().text.color);
        assert_eq!(
            Some(theme.code_background),
            style.inline_code().background_color
        );
        assert_eq!(Some(theme.code_foreground), style.inline_code().color);
        assert_eq!(
            Some(theme.table_header.into()),
            style.table_head().background
        );
        assert_eq!(Some(theme.table_row.into()), style.table_cell().background);
        assert_eq!(
            style.table().corner_radii.top_left,
            Some(theme.surface_radius.into())
        );
    }
}
