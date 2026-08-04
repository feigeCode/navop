use declarative_ui_demo::{ComponentProps, ComponentRenderer, ComponentResult, RenderContext};
use gpui::{Div, IntoElement, ParentElement, Styled, div, rgb};
use gpui_component::{IconKind, IconName, IconSize, StyledExt as _};

mod card;

use card::render_icon_card;

const CARD_BACKGROUND_RGB: u32 = 0x18_18_1b;
const CARD_BORDER_RGB: u32 = 0x3f_3f_46;
const MUTED_TEXT_RGB: u32 = 0xa1_a1_aa;
const PRIMARY_TEXT_RGB: u32 = 0xf4_f4_f5;
pub(super) const ICON_SIZE_MATRIX: [(IconSize, u8); 7] = [
    (IconSize::Micro, 12),
    (IconSize::Small, 14),
    (IconSize::Default, 16),
    (IconSize::Medium, 20),
    (IconSize::Large, 24),
    (IconSize::Display, 32),
    (IconSize::Hero, 40),
];

pub struct IconGalleryComponent;

impl ComponentRenderer for IconGalleryComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let state = gallery_state(context);
        let matching_icons = matching_icons(&state);
        let gallery = render_gallery(&matching_icons, &state);

        Ok(context.style(gallery, &props).into_any_element())
    }
}

struct GalleryState {
    query: String,
    kind_filter: String,
    icon_size: IconSize,
}

fn gallery_state(context: &RenderContext<'_>) -> GalleryState {
    let query = context
        .get_state("icon_gallery_query")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let kind_filter = context
        .get_state("icon_gallery_kind")
        .unwrap_or_else(|| "all".to_owned());
    let icon_size = parse_icon_size(
        context
            .get_state("icon_gallery_size")
            .as_deref()
            .unwrap_or("16"),
    );
    GalleryState {
        query,
        kind_filter,
        icon_size,
    }
}

fn matching_icons(state: &GalleryState) -> Vec<IconName> {
    IconName::ALL
        .iter()
        .copied()
        .filter(|icon| icon_matches(*icon, &state.query, &state.kind_filter))
        .collect()
}

fn render_gallery(icons: &[IconName], state: &GalleryState) -> Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .min_w_0()
        .gap_3()
        .child(render_summary(icons.len(), state))
        .child(
            div()
                .flex()
                .flex_wrap()
                .w_full()
                .min_w_0()
                .gap_3()
                .children(
                    icons
                        .iter()
                        .copied()
                        .map(|icon| render_icon_card(icon, state.icon_size)),
                ),
        )
}

fn render_summary(icon_count: usize, state: &GalleryState) -> Div {
    let summary = format!(
        "{} of {} icons · {} · selected {} px · matrix 12–40 px",
        icon_count,
        IconName::ALL.len(),
        filter_label(&state.kind_filter),
        icon_size_label(state.icon_size),
    );
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .text_sm()
                .font_semibold()
                .text_color(rgb(PRIMARY_TEXT_RGB))
                .child("Semantic icon registry"),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(MUTED_TEXT_RGB))
                .child(summary),
        )
}

fn icon_matches(icon: IconName, query: &str, kind_filter: &str) -> bool {
    let metadata = icon.metadata();
    let normalized_kind_filter = kind_filter.trim().to_ascii_lowercase();
    let kind_matches =
        normalized_kind_filter == "all" || kind_key(metadata.kind) == normalized_kind_filter;

    if !kind_matches {
        return false;
    }

    if query.is_empty() {
        return true;
    }

    let haystack = format!(
        "{icon:?} {} {} {} {}",
        kind_key(metadata.kind),
        metadata.canonical_path.as_ref(),
        metadata.source.unwrap_or("unknown audit pending"),
        metadata.license.unwrap_or("unknown audit pending"),
    )
    .to_ascii_lowercase();
    haystack.contains(query)
}

fn parse_icon_size(value: &str) -> IconSize {
    match value.trim() {
        "12" => IconSize::Micro,
        "14" => IconSize::Small,
        "20" => IconSize::Medium,
        "24" => IconSize::Large,
        "32" => IconSize::Display,
        "40" => IconSize::Hero,
        "16" | _ => IconSize::Default,
    }
}

fn icon_size_label(size: IconSize) -> u8 {
    match size {
        IconSize::Micro => 12,
        IconSize::Small => 14,
        IconSize::Default => 16,
        IconSize::Medium => 20,
        IconSize::Large => 24,
        IconSize::Display => 32,
        IconSize::Hero => 40,
    }
}

fn kind_key(kind: IconKind) -> &'static str {
    match kind {
        IconKind::FunctionalOutline => "functional-outline",
        IconKind::FunctionalFilled => "functional-filled",
        IconKind::BrandColor => "brand-color",
        IconKind::ObjectGlyph => "object-glyph",
    }
}

pub(super) fn kind_label(kind: IconKind) -> &'static str {
    match kind {
        IconKind::FunctionalOutline => "Functional Outline",
        IconKind::FunctionalFilled => "Functional Filled",
        IconKind::BrandColor => "Brand Color",
        IconKind::ObjectGlyph => "Object Glyph",
    }
}

fn filter_label(filter: &str) -> &'static str {
    match filter.trim().to_ascii_lowercase().as_str() {
        "functional-outline" => "Functional Outline",
        "functional-filled" => "Functional Filled",
        "brand-color" => "Brand Color",
        "object-glyph" => "Object Glyph",
        _ => "All kinds",
    }
}

#[cfg(test)]
mod tests {
    use super::{ICON_SIZE_MATRIX, icon_matches, kind_key, parse_icon_size};
    use gpui_component::{IconKind, IconName, IconSize};

    #[test]
    fn icon_size_state_maps_to_the_seven_visual_tokens() {
        assert_eq!(parse_icon_size("12"), IconSize::Micro);
        assert_eq!(parse_icon_size("14"), IconSize::Small);
        assert_eq!(parse_icon_size("16"), IconSize::Default);
        assert_eq!(parse_icon_size("20"), IconSize::Medium);
        assert_eq!(parse_icon_size("24"), IconSize::Large);
        assert_eq!(parse_icon_size("32"), IconSize::Display);
        assert_eq!(parse_icon_size("40"), IconSize::Hero);
        assert_eq!(parse_icon_size("invalid"), IconSize::Default);
    }

    #[test]
    fn icon_size_matrix_keeps_the_canonical_visual_order() {
        assert_eq!(
            ICON_SIZE_MATRIX,
            [
                (IconSize::Micro, 12),
                (IconSize::Small, 14),
                (IconSize::Default, 16),
                (IconSize::Medium, 20),
                (IconSize::Large, 24),
                (IconSize::Display, 32),
                (IconSize::Hero, 40),
            ]
        );
    }

    #[test]
    fn search_matches_name_kind_and_canonical_path() {
        assert!(icon_matches(IconName::PostgreSQLColor, "postgresql", "all"));
        assert!(icon_matches(
            IconName::PostgreSQLColor,
            "brand-color",
            "all"
        ));
        assert!(icon_matches(
            IconName::PostgreSQLColor,
            "icons/postgresql_color.svg",
            "all"
        ));
        assert!(icon_matches(
            IconName::PostgreSQLColor,
            "audit pending",
            "all"
        ));
        assert!(!icon_matches(IconName::PostgreSQLColor, "redis", "all"));
    }

    #[test]
    fn kind_filter_uses_semantic_metadata() {
        assert_eq!(kind_key(IconKind::ObjectGlyph), "object-glyph");
        assert!(icon_matches(IconName::Table, "", "object-glyph"));
        assert!(!icon_matches(IconName::Table, "", "functional-outline"));
    }
}
