//! 扩展目录卡片布局：虚线饰线 / 图标标题行 / 两行描述 / 标签·操作页脚。
//! 布局与具体 View 解耦（只依赖 `&App` 主题），供 `card_view` 独立实体渲染。

use gpui::{
    App, FontWeight, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder, px, rems,
};
use gpui_component::{
    ActiveTheme, ColorName, Icon, Sizable, Size, button::Button, h_flex, tag::Tag, v_flex,
};
use one_assets::IconName;
use one_ui::IconSize;
use rust_i18n::t;

use crate::{ExtensionKind, MarketplaceEntry, MarketplaceInstallState};

const CARD_ICON_TILE_REMS: f32 = 2.25;
/// 卡片顶部悬空虚线饰线的高度。
const DASHED_RULE_HEIGHT_PX: f32 = 3.0;
const CARD_ROUNDED_PX: f32 = 14.0;

pub(crate) fn kind_icon(kind: ExtensionKind) -> IconName {
    match kind {
        ExtensionKind::Language => IconName::ExtensionsLine,
        ExtensionKind::LanguageBundle => IconName::Apps,
        ExtensionKind::DatabaseDriver => IconName::Database,
        ExtensionKind::RemoteDesktopProvider => IconName::RdpLine,
        ExtensionKind::Composite => IconName::ExtensionsColor,
    }
}

/// 每种扩展类型的品牌色，用于卡片图标底色与分类 Tag，形成扫读锚点。
pub(crate) fn kind_color(kind: ExtensionKind) -> ColorName {
    match kind {
        ExtensionKind::Language => ColorName::Blue,
        ExtensionKind::LanguageBundle => ColorName::Cyan,
        ExtensionKind::DatabaseDriver => ColorName::Emerald,
        ExtensionKind::RemoteDesktopProvider => ColorName::Violet,
        ExtensionKind::Composite => ColorName::Pink,
    }
}

pub(crate) fn kind_label(kind: ExtensionKind) -> String {
    match kind {
        ExtensionKind::Language => t!("Extension.kind_language").to_string(),
        ExtensionKind::LanguageBundle => t!("Extension.kind_language_bundle").to_string(),
        ExtensionKind::DatabaseDriver => t!("Extension.kind_database_driver").to_string(),
        ExtensionKind::RemoteDesktopProvider => {
            t!("Extension.kind_remote_desktop_provider").to_string()
        }
        ExtensionKind::Composite => t!("Extension.kind_composite").to_string(),
    }
}

pub(crate) fn extension_kind_id(kind: ExtensionKind) -> &'static str {
    match kind {
        ExtensionKind::Language => "language",
        ExtensionKind::LanguageBundle => "language-bundle",
        ExtensionKind::DatabaseDriver => "database-driver",
        ExtensionKind::RemoteDesktopProvider => "remote-desktop-provider",
        ExtensionKind::Composite => "composite",
    }
}

/// Market 分区标题：大号 semibold 标题 + 灰色计数。
pub(crate) fn section_header(
    title: String,
    count: usize,
    fg: gpui::Hsla,
    muted: gpui::Hsla,
) -> impl IntoElement {
    h_flex()
        .gap_2p5()
        .items_baseline()
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg)
                .child(title),
        )
        .child(div().text_sm().text_color(muted).child(format!("{count}")))
}

/// 卡片外壳数据：由调用方（独立卡片实体）组装。
pub(crate) struct CardShell {
    pub icon: IconName,
    pub kind: ExtensionKind,
    pub name: String,
    pub version: String,
    pub description: String,
    /// 语言包支持的语言；其它类型为空。
    pub languages: Vec<String>,
    pub actions: Vec<Button>,
}

/// 渲染不可点击的卡片外壳（已安装页）。`card_id` 必须由数据推导且跨帧稳定。
pub(crate) fn card_shell(shell: CardShell, card_id: String, cx: &App) -> gpui::AnyElement {
    wrap_with_dashed_rule(build_card(shell, cx).id(card_id), cx)
}

/// 渲染整卡可点的卡片外壳（市场页：点卡片打开详情弹窗）。
pub(crate) fn clickable_card_shell(
    shell: CardShell,
    card_id: String,
    on_card_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> gpui::AnyElement {
    wrap_with_dashed_rule(
        build_card(shell, cx)
            .id(card_id)
            .on_click(on_card_click)
            .cursor_pointer(),
        cx,
    )
}

fn wrap_with_dashed_rule(card: impl IntoElement, cx: &App) -> gpui::AnyElement {
    v_flex()
        .w_full()
        .gap_1()
        .child(dashed_rule(cx))
        .child(card)
        .into_any_element()
}

fn build_card(shell: CardShell, cx: &App) -> gpui::Div {
    let CardShell {
        icon,
        kind,
        name,
        version,
        description,
        languages,
        actions,
    } = shell;
    let fg = cx.theme().foreground;
    let muted = cx.theme().muted_foreground;
    let tag = Tag::color(kind_color(kind))
        .with_size(Size::Small)
        .rounded_full()
        .child(kind_label(kind));
    // hover 用边框+底色高亮：阴影模糊层的进出会触发大面积重绘，扫过卡片时明显迟滞。
    let hover_border = cx.theme().primary.opacity(0.45);
    let hover_bg = cx.theme().list_hover;
    v_flex()
        .min_h(rems(9.75))
        .gap_3()
        .p_4()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(CARD_ROUNDED_PX))
        .bg(cx.theme().background)
        .hover(move |style| style.border_color(hover_border).bg(hover_bg))
        .child(card_header(&name, &version, icon, kind, fg, muted, cx))
        .child(
            div()
                .text_sm()
                .line_height(rems(1.4))
                .text_color(muted)
                .line_clamp(2)
                .child(description),
        )
        .when(!languages.is_empty(), |this| {
            this.child(languages_line(&languages, muted))
        })
        .child(card_footer(vec![tag], actions))
}

fn dashed_rule(cx: &App) -> impl IntoElement {
    div()
        .w_full()
        .h(px(DASHED_RULE_HEIGHT_PX))
        .border_t_1()
        .border_dashed()
        .border_color(cx.theme().border)
}

fn card_header(
    name: &str,
    version: &str,
    icon: IconName,
    kind: ExtensionKind,
    fg: gpui::Hsla,
    muted: gpui::Hsla,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .gap_3()
        .items_center()
        .child(card_icon_tile(icon, kind, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg)
                .truncate()
                .child(name.to_string()),
        )
        .when_some((!version.is_empty()).then_some(version), |this, version| {
            this.child(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded(px(6.0))
                    .text_xs()
                    .text_color(muted)
                    .bg(gpui::black().opacity(0.04))
                    .child(format!("v{version}")),
            )
        })
        .child(
            Icon::new(IconName::ExternalLink)
                .with_size(IconSize::Small)
                .text_color(muted.opacity(0.7)),
        )
}

/// 卡片上最多预览的语言数量，超出后追加“等 N 种语言”。
const CARD_LANGUAGE_PREVIEW: usize = 6;

/// 语言标识 → 展示名。未知标识做可读化兜底，保证新增语言无需改 UI 代码。
pub(crate) fn language_display_name(language: &str) -> String {
    match language {
        "c" => "C".to_string(),
        "cmake" => "CMake".to_string(),
        "cpp" => "C++".to_string(),
        "csharp" => "C#".to_string(),
        "css" => "CSS".to_string(),
        "ejs" => "EJS".to_string(),
        "erb" => "ERB".to_string(),
        "graphql" => "GraphQL".to_string(),
        "html" => "HTML".to_string(),
        "javascript" => "JavaScript".to_string(),
        "jsdoc" => "JSDoc".to_string(),
        "php" => "PHP".to_string(),
        "proto" | "protobuf" => "Protobuf".to_string(),
        "toml" => "TOML".to_string(),
        "tsx" => "TSX".to_string(),
        "typescript" => "TypeScript".to_string(),
        "yaml" => "YAML".to_string(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => {
                    let mut name = first.to_uppercase().to_string();
                    name.push_str(&chars.as_str().replace('_', " "));
                    name
                }
                None => String::new(),
            }
        }
    }
}

/// 详情页用的完整语言列表。
pub(crate) fn language_display_list(languages: &[String]) -> String {
    languages
        .iter()
        .map(|language| language_display_name(language))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn language_names_preview(languages: &[String]) -> String {
    languages
        .iter()
        .take(CARD_LANGUAGE_PREVIEW)
        .map(|language| language_display_name(language))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn languages_line(languages: &[String], muted: gpui::Hsla) -> impl IntoElement {
    let names = language_names_preview(languages);
    let text = if languages.len() > CARD_LANGUAGE_PREVIEW {
        t!(
            "Extension.languages_supported_more",
            names = names,
            count = languages.len()
        )
        .to_string()
    } else {
        t!("Extension.languages_supported", names = names).to_string()
    };
    div()
        .text_xs()
        .line_height(rems(1.4))
        .text_color(muted)
        .line_clamp(1)
        .child(text)
}

fn card_footer(tags: Vec<Tag>, actions: Vec<Button>) -> impl IntoElement {
    h_flex()
        .mt_auto()
        .w_full()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .flex_wrap()
                .gap_1p5()
                .children(tags),
        )
        .child(h_flex().flex_none().gap_1().children(actions))
}

/// 图标底座：品牌色浅底圆角块，让卡片头部有视觉重心。
fn card_icon_tile(icon: IconName, kind: ExtensionKind, cx: &App) -> impl IntoElement {
    let color = kind_color(kind);
    // 与 `TagVariant::Color` 底色同规则：亮色取 scale(50)，暗色取 scale(950) 半透明，
    // 保持图标底座与分类 Tag 底色一致。
    let tint = if cx.theme().is_dark() {
        color.scale(950).opacity(0.5)
    } else {
        color.scale(50)
    };
    div()
        .size(rems(CARD_ICON_TILE_REMS))
        .rounded(px(10.0))
        .bg(tint)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .child(Icon::new(icon).with_size(IconSize::Medium).color())
}

pub(crate) fn marketplace_action_label_public(
    state: MarketplaceInstallState,
    host_compatible: bool,
) -> String {
    match state {
        MarketplaceInstallState::NotInstalled if !host_compatible => {
            t!("Extension.requires_upgrade").to_string()
        }
        MarketplaceInstallState::NotInstalled => t!("Extension.install").to_string(),
        MarketplaceInstallState::Installed => t!("Extension.installed").to_string(),
        MarketplaceInstallState::UpdateAvailable if !host_compatible => {
            t!("Extension.requires_upgrade").to_string()
        }
        MarketplaceInstallState::UpdateAvailable => t!("Extension.update").to_string(),
    }
}

pub(crate) fn marketplace_description_public(entry: &MarketplaceEntry) -> String {
    let description = if !entry.description.trim().is_empty() {
        entry.description.clone()
    } else {
        t!("Extension.no_description").to_string()
    };
    if entry.host_compatible {
        return description;
    }
    let Some(required) = entry.required_host_version.as_deref() else {
        return description;
    };
    format!(
        "{description}\n{}",
        t!("Extension.requires_host", version = required)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_language_ids_map_to_expected_display_names() {
        for (id, expected) in [
            ("c", "C"),
            ("cpp", "C++"),
            ("csharp", "C#"),
            ("javascript", "JavaScript"),
            ("typescript", "TypeScript"),
            ("graphql", "GraphQL"),
            ("php", "PHP"),
            ("proto", "Protobuf"),
            ("tsx", "TSX"),
            ("markdown_inline", "Markdown inline"),
        ] {
            assert_eq!(expected, language_display_name(id), "language id {id}");
        }
    }

    #[test]
    fn unknown_language_ids_fall_back_to_readable_names() {
        assert_eq!(
            "Some new language",
            language_display_name("some_new_language")
        );
        assert_eq!("", language_display_name(""));
    }

    #[test]
    fn card_preview_shows_at_most_six_languages() {
        let languages = [
            "rust",
            "typescript",
            "python",
            "go",
            "csharp",
            "cpp",
            "javascript",
            "java",
        ]
        .map(str::to_string);

        assert_eq!(
            "Rust · TypeScript · Python · Go · C# · C++",
            language_names_preview(&languages)
        );
        assert_eq!(
            "Rust · TypeScript · Python · Go · C# · C++ · JavaScript · Java",
            language_display_list(&languages)
        );
    }
}
