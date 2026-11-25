use gpui::{App, Hsla};
use gpui_component::ActiveTheme as _;

/// 应用外壳的中性表面。菜单、Tab 与内容各自有明确的表面归属。
///
/// 外壳（左侧图标 rail / 应用菜单 / Tab 栏）必须只有一个来源：`shell`。
/// 侧边栏与 Tab 栏同色是产品硬性要求，任何按模式拆开取色的写法都会在深色
/// 主题里重新制造「rail ≠ tab_bar」的裂缝，所以这里不再按 `is_dark()` 分支。
///
/// 表面层次由主题 token 决定：
/// `theme.background`（内容：toolbar / 卡片 / active tab）
///   > `theme.muted`（内容画布）
///   > `theme.tab_bar`（外壳）。见 `one_core::themes` 的三级表面不变量测试。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ChromePalette {
    pub rail: Hsla,
    pub menu: Hsla,
    pub tab_bar: Hsla,
    pub content: Hsla,
}

pub(crate) fn chrome_palette(cx: &App) -> ChromePalette {
    let theme = cx.theme();
    let shell = theme.tab_bar;
    ChromePalette {
        rail: shell,
        menu: shell,
        tab_bar: shell,
        content: theme.background,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn chrome_surfaces_share_the_shell_token() {
        let palette = include_str!("chrome_palette.rs");
        let app = include_str!("navop_app.rs");
        let sidebar = include_str!("persistent_connection_sidebar/mod.rs");
        let navigation = include_str!("home_tab/sidebar.rs");

        assert!(palette.contains("let shell = theme.tab_bar;"));
        assert!(palette.contains("rail: shell"));
        assert!(palette.contains("menu: shell"));
        assert!(palette.contains("tab_bar: shell"));
        assert!(palette.contains("content: theme.background"));
        assert!(app.contains("tabs.set_tab_bar_bg_color(chrome_palette.tab_bar, cx)"));
        assert!(app.contains("tabs.set_active_tab_bg_color(chrome_palette.content, cx)"));
        // 非活动 Tab 必须有胶囊底：壳色 Tab 栏上默认透明时看不出这是 Tab。
        assert!(app.contains("tabs.set_inactive_tab_bg_color(cx.theme().muted, cx)"));
        assert!(app.contains("tabs.set_inactive_tab_hover_color(cx.theme().background, cx)"));
        assert!(app.contains("tabs.set_inactive_tab_border_color(cx.theme().border, cx)"));
        assert!(app.contains("let chrome_palette = crate::chrome_palette::chrome_palette(cx)"));
        assert!(app.contains(".bg(chrome_palette.rail)"));
        assert!(navigation.contains("if global {"));
        assert!(sidebar.contains("chrome_palette::chrome_palette(cx).content"));
    }

    /// 侧边栏与 Tab 栏同色是产品硬性要求；浅色与深色两套主题都必须成立，
    /// 且 active tab / 画布不能与外壳混成一片。
    #[gpui::test]
    fn sidebar_and_tab_bar_share_one_shell_color_in_both_modes(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            one_core::themes::load_bundled(cx);

            for mode in ["light", "dark"] {
                let settings = one_core::settings::AppSettings {
                    theme_mode: mode.to_string(),
                    ..Default::default()
                };
                one_core::themes::apply_appearance(&settings, cx);

                let palette = super::chrome_palette(cx);
                assert_eq!(palette.rail, palette.menu, "{mode}: rail 与菜单必须同色");
                assert_eq!(
                    palette.rail, palette.tab_bar,
                    "{mode}: 侧边栏与 Tab 栏必须同色"
                );
                assert_ne!(
                    palette.tab_bar, palette.content,
                    "{mode}: active tab 必须能从外壳里区分出来"
                );
            }
        });
    }
}
