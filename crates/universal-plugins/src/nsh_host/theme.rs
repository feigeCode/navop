//! 宿主主题 → nsh 主题（shadcn token）。
//!
//! navop 的组件主题就是 gpui-kit 的 `Theme`，token 与 shadcn 一一对应，
//! 所以这里只做名字映射；页面里写 `bg-card` / `text-muted-foreground` /
//! `border-border` 就能跟着宿主换肤与明暗切换。

use gpui::{App, Entity, Hsla as GpuiHsla};
use gpui_component::{ActiveTheme as _, Theme, ThemeMode};
use nsh_platform_gpui::{AdaptiveColor, AppView, Appearance, Hsla};

/// `nsh_core::element::tokens::{FONT_SANS, FONT_MONO}` 的字面值。
/// nsh-platform-gpui 没有重导出 tokens 模块，这里不为此引入 nsh-core。
const FONT_SANS: &str = "font-sans";
const FONT_MONO: &str = "font-mono";

/// gpui 颜色 → nsh 颜色（通道一一对应）。
fn color(c: GpuiHsla) -> Hsla {
    Hsla {
        h: c.h,
        s: c.s,
        l: c.l,
        a: c.a,
    }
}

/// gpui-component 主题 → nsh 主题（shadcn token），映射同 embed-gpui 示例。
pub(crate) fn nsh_theme(theme: &Theme) -> nsh_platform_gpui::Theme {
    let mut nsh = nsh_platform_gpui::Theme::default();
    let colors = &theme.colors;
    for (name, c) in [
        ("background", colors.background),
        ("foreground", colors.foreground),
        ("card", colors.group_box),
        ("card-foreground", colors.group_box_foreground),
        ("popover", colors.popover),
        ("popover-foreground", colors.popover_foreground),
        ("primary", colors.primary),
        ("primary-foreground", colors.primary_foreground),
        ("secondary", colors.secondary),
        ("secondary-foreground", colors.secondary_foreground),
        ("muted", colors.muted),
        ("muted-foreground", colors.muted_foreground),
        ("accent", colors.accent),
        ("accent-foreground", colors.accent_foreground),
        ("destructive", colors.danger),
        ("border", colors.border),
        ("input", colors.input),
        ("ring", colors.ring),
        ("chart-1", colors.chart_1),
        ("chart-2", colors.chart_2),
        ("chart-3", colors.chart_3),
        ("chart-4", colors.chart_4),
        ("chart-5", colors.chart_5),
        ("sidebar", colors.sidebar),
        ("sidebar-foreground", colors.sidebar_foreground),
        ("sidebar-primary", colors.sidebar_primary),
        (
            "sidebar-primary-foreground",
            colors.sidebar_primary_foreground,
        ),
        ("sidebar-accent", colors.sidebar_accent),
        (
            "sidebar-accent-foreground",
            colors.sidebar_accent_foreground,
        ),
        ("sidebar-border", colors.sidebar_border),
        ("sidebar-ring", colors.ring),
    ] {
        nsh.set_color(name, AdaptiveColor::from(color(c)));
    }
    nsh.set_number("radius", f32::from(theme.radius));
    nsh.set_font(FONT_SANS, theme.font_family.to_string());
    nsh.set_font(FONT_MONO, theme.mono_font_family.to_string());
    nsh
}

pub(crate) fn appearance(mode: ThemeMode) -> Appearance {
    if mode.is_dark() {
        Appearance::Dark
    } else {
        Appearance::Light
    }
}

/// 把宿主当前主题推给一个 `AppView`（首帧前、主题或明暗变化后）。
pub(crate) fn push(view: &Entity<AppView>, cx: &mut App) {
    let theme = cx.theme();
    let (nsh, look) = (nsh_theme(theme), appearance(theme.mode));
    view.update(cx, |view, cx| {
        view.set_theme(Some(nsh), cx);
        view.set_appearance(look, cx);
    });
}
