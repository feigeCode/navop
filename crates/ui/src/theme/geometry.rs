use crate::Size;
use gpui::{Edges, Pixels, px};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Geometry tokens shared by the application shell, components, and pages.
///
/// These values intentionally live beside the color theme while remaining
/// independent from user-authored color theme files. This gives migrated UI a
/// single source of truth without changing the existing `ThemeConfig` schema.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ThemeGeometry {
    pub spacing: SpacingTokens,
    pub radius: RadiusTokens,
    pub control: ControlSizeTokens,
    pub layout: LayoutSizeTokens,
    pub tree: TreeListGeometry,
    pub border: BorderTokens,
    pub shadow: ShadowTokens,
    pub opacity: OpacityTokens,
    pub motion: MotionTokens,
    pub overlay: OverlayGeometry,
    pub resize: ResizeGeometry,
}

impl ThemeGeometry {
    /// Resolve a control height from the legacy component [`Size`].
    ///
    /// This compatibility accessor lets components migrate to theme geometry
    /// without changing their public sizing API.
    #[inline]
    pub fn control_height(&self, size: Size) -> Pixels {
        self.control.height(size)
    }

    /// Resolve the legacy table row height.
    #[inline]
    pub fn table_row_height(&self, size: Size) -> Pixels {
        match size {
            Size::XSmall => px(26.),
            Size::Small => px(30.),
            Size::Large => px(40.),
            Size::Medium | Size::Size(_) => px(32.),
        }
    }

    /// Resolve the legacy table cell padding.
    #[inline]
    pub fn table_cell_padding(&self, size: Size) -> Edges<Pixels> {
        match size {
            Size::XSmall => Edges {
                top: px(2.),
                bottom: px(2.),
                left: px(4.),
                right: px(4.),
            },
            Size::Small => Edges {
                top: px(3.),
                bottom: px(3.),
                left: px(6.),
                right: px(6.),
            },
            Size::Large => Edges {
                top: px(8.),
                bottom: px(8.),
                left: px(12.),
                right: px(12.),
            },
            Size::Medium | Size::Size(_) => Edges {
                top: px(4.),
                bottom: px(4.),
                left: px(8.),
                right: px(8.),
            },
        }
    }

    /// Resolve the legacy horizontal input padding.
    #[inline]
    pub fn input_padding_x(&self, size: Size) -> Pixels {
        match size {
            Size::Large => px(16.),
            Size::Medium => px(12.),
            Size::Small => px(8.),
            Size::XSmall => px(4.),
            Size::Size(_) => px(8.),
        }
    }

    /// Resolve the legacy vertical input padding.
    #[inline]
    pub fn input_padding_y(&self, size: Size) -> Pixels {
        match size {
            Size::Large => px(10.),
            Size::Medium => px(8.),
            Size::Small => px(2.),
            Size::XSmall => px(0.),
            Size::Size(_) => px(2.),
        }
    }
}

/// Four-pixel spacing scale used by shell and component layouts.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SpacingTokens {
    pub space_1: Pixels,
    pub space_2: Pixels,
    pub space_3: Pixels,
    pub space_4: Pixels,
    pub space_5: Pixels,
    pub space_6: Pixels,
    pub space_8: Pixels,
}

impl Default for SpacingTokens {
    fn default() -> Self {
        Self {
            space_1: px(4.),
            space_2: px(8.),
            space_3: px(12.),
            space_4: px(16.),
            space_5: px(20.),
            space_6: px(24.),
            space_8: px(32.),
        }
    }
}

/// Semantic corner-radius scale.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RadiusTokens {
    pub none: Pixels,
    pub xs: Pixels,
    pub sm: Pixels,
    pub md: Pixels,
    pub lg: Pixels,
    pub pill: Pixels,
}

impl Default for RadiusTokens {
    fn default() -> Self {
        Self {
            none: px(0.),
            xs: px(4.),
            sm: px(6.),
            md: px(8.),
            lg: px(12.),
            pill: px(999.),
        }
    }
}

/// Standard control heights. Visual icon size is deliberately defined
/// separately by `IconSize`; a larger hit target must not enlarge the glyph.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ControlSizeTokens {
    pub compact: Pixels,
    pub small: Pixels,
    pub default: Pixels,
    pub medium: Pixels,
    pub large: Pixels,
    pub xlarge: Pixels,
    pub hero: Pixels,
}

impl ControlSizeTokens {
    /// Resolve the standard height for a legacy component size.
    #[inline]
    pub fn height(&self, size: Size) -> Pixels {
        match size {
            Size::XSmall => self.compact,
            Size::Small => self.small,
            Size::Medium => self.default,
            Size::Large => self.large,
            Size::Size(size) => size,
        }
    }
}

impl Default for ControlSizeTokens {
    fn default() -> Self {
        Self {
            compact: px(24.),
            small: px(28.),
            default: px(32.),
            medium: px(36.),
            large: px(40.),
            xlarge: px(44.),
            hero: px(48.),
        }
    }
}

/// Application shell and workspace role sizes.
///
/// Candidate dimensions such as the 52px global rail are defined here before
/// page migration. Existing surfaces should switch to these roles atomically
/// with their related offsets and platform safe areas.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LayoutSizeTokens {
    pub title_bar: Pixels,
    pub title_bar_content_padding: Pixels,
    pub macos_title_bar_content_padding: Pixels,
    pub macos_compact_title_bar_content_padding: Pixels,
    pub macos_rail_title_bar_height: Pixels,
    pub window_control_width: Pixels,
    pub tab_bar: Pixels,
    pub tab_item: Pixels,
    pub command_bar: Pixels,
    pub panel_header: Pixels,
    pub embedded_panel_header: Pixels,
    pub dock_panel_header: Pixels,
    pub list_header: Pixels,
    pub collapsed_bottom_dock_header: Pixels,
    pub status_bar: Pixels,
    pub global_rail: Pixels,
    pub compact_rail: Pixels,
    pub global_rail_item: Pixels,
    pub context_sidebar_default: Pixels,
    pub context_sidebar_min: Pixels,
    pub context_sidebar_max: Pixels,
    pub utility_panel_default: Pixels,
    pub utility_panel_min: Pixels,
    pub utility_panel_max: Pixels,
    pub sidebar_panel_min: Pixels,
    pub sidebar_center_min: Pixels,
    pub sidebar_bottom_default: Pixels,
    pub workspace_min: Pixels,
}

impl Default for LayoutSizeTokens {
    fn default() -> Self {
        Self {
            title_bar: px(34.),
            title_bar_content_padding: px(12.),
            macos_title_bar_content_padding: px(80.),
            macos_compact_title_bar_content_padding: px(36.),
            macos_rail_title_bar_height: px(40.),
            window_control_width: px(34.),
            tab_bar: px(40.),
            tab_item: px(32.),
            command_bar: px(48.),
            panel_header: px(36.),
            embedded_panel_header: px(40.),
            dock_panel_header: px(30.),
            list_header: px(28.),
            collapsed_bottom_dock_header: px(29.),
            status_bar: px(28.),
            global_rail: px(52.),
            compact_rail: px(44.),
            global_rail_item: px(40.),
            context_sidebar_default: px(260.),
            context_sidebar_min: px(220.),
            context_sidebar_max: px(520.),
            utility_panel_default: px(360.),
            utility_panel_min: px(280.),
            utility_panel_max: px(600.),
            sidebar_panel_min: px(120.),
            sidebar_center_min: px(160.),
            sidebar_bottom_default: px(260.),
            workspace_min: px(640.),
        }
    }
}

/// Shared geometry for compact trees and hierarchical lists.
///
/// These defaults describe the common 28px tree used by data explorers.
/// Surfaces with an intentional density difference may keep a local row height
/// or indent while still sharing base padding and disclosure geometry.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TreeListGeometry {
    pub row_height: Pixels,
    pub indent: Pixels,
    pub base_padding: Pixels,
    pub disclosure_size: Pixels,
}

impl Default for TreeListGeometry {
    fn default() -> Self {
        Self {
            row_height: px(28.),
            indent: px(16.),
            base_padding: px(8.),
            disclosure_size: px(16.),
        }
    }
}

/// Border-width scale. Color remains a semantic color-theme concern.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BorderTokens {
    pub hairline: Pixels,
    pub control: Pixels,
    pub focus: Pixels,
    pub strong: Pixels,
}

impl Default for BorderTokens {
    fn default() -> Self {
        Self {
            hairline: px(1.),
            control: px(1.),
            focus: px(1.5),
            strong: px(2.),
        }
    }
}

/// Geometry of a single elevation step. Shadow color comes from the active
/// light or dark theme.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ShadowGeometry {
    pub offset_x: Pixels,
    pub offset_y: Pixels,
    pub blur: Pixels,
    pub spread: Pixels,
}

impl Default for ShadowGeometry {
    fn default() -> Self {
        Self {
            offset_x: px(0.),
            offset_y: px(0.),
            blur: px(0.),
            spread: px(0.),
        }
    }
}

/// Standard elevation geometry.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ShadowTokens {
    pub small: ShadowGeometry,
    pub medium: ShadowGeometry,
    pub large: ShadowGeometry,
}

impl Default for ShadowTokens {
    fn default() -> Self {
        Self {
            small: ShadowGeometry {
                offset_y: px(1.),
                blur: px(3.),
                ..ShadowGeometry::default()
            },
            medium: ShadowGeometry {
                offset_y: px(4.),
                blur: px(12.),
                spread: px(-2.),
                ..ShadowGeometry::default()
            },
            large: ShadowGeometry {
                offset_y: px(12.),
                blur: px(32.),
                spread: px(-4.),
                ..ShadowGeometry::default()
            },
        }
    }
}

/// Opacity scale for interaction and hierarchy states.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OpacityTokens {
    pub disabled: f32,
    pub muted: f32,
    pub subtle: f32,
    pub hover_overlay: f32,
    pub pressed_overlay: f32,
    /// Scrim used while content remains visible but temporarily unavailable.
    pub loading_scrim: f32,
    /// Standard blocking backdrop for dialogs and disconnected workspaces.
    pub scrim: f32,
}

impl Default for OpacityTokens {
    fn default() -> Self {
        Self {
            disabled: 0.45,
            muted: 0.65,
            subtle: 0.08,
            hover_overlay: 0.08,
            pressed_overlay: 0.12,
            loading_scrim: 0.24,
            scrim: 0.48,
        }
    }
}

/// Motion durations in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MotionTokens {
    pub fast_ms: u64,
    pub normal_ms: u64,
    pub slow_ms: u64,
}

impl MotionTokens {
    #[inline]
    pub fn fast(&self) -> Duration {
        Duration::from_millis(self.fast_ms)
    }

    #[inline]
    pub fn normal(&self) -> Duration {
        Duration::from_millis(self.normal_ms)
    }

    #[inline]
    pub fn slow(&self) -> Duration {
        Duration::from_millis(self.slow_ms)
    }
}

impl Default for MotionTokens {
    fn default() -> Self {
        Self {
            fast_ms: 80,
            normal_ms: 160,
            slow_ms: 240,
        }
    }
}

/// Shared geometry for popovers, dialogs, command palettes, and sheets.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OverlayGeometry {
    pub edge_margin: Pixels,
    pub content_padding: Pixels,
    pub anchor_gap: Pixels,
    pub command_palette_max_width: Pixels,
    pub dialog_max_width: Pixels,
}

impl Default for OverlayGeometry {
    fn default() -> Self {
        Self {
            edge_margin: px(16.),
            content_padding: px(16.),
            anchor_gap: px(8.),
            command_palette_max_width: px(640.),
            dialog_max_width: px(720.),
        }
    }
}

/// Resize-handle geometry. The visible separator and pointer hit target are
/// separate by design.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ResizeGeometry {
    pub visible_line: Pixels,
    pub edge_padding: Pixels,
    pub collapsed_threshold: Pixels,
}

impl ResizeGeometry {
    #[inline]
    pub fn hit_area(&self) -> Pixels {
        self.visible_line + self.edge_padding * 2.
    }
}

impl Default for ResizeGeometry {
    fn default() -> Self {
        Self {
            visible_line: px(1.),
            edge_padding: px(4.),
            collapsed_threshold: px(44.),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Size;
    use gpui::px;

    #[test]
    fn spacing_scale_uses_four_pixel_grid() {
        let spacing = SpacingTokens::default();

        assert_eq!(spacing.space_1, px(4.));
        assert_eq!(spacing.space_2, px(8.));
        assert_eq!(spacing.space_3, px(12.));
        assert_eq!(spacing.space_4, px(16.));
        assert_eq!(spacing.space_5, px(20.));
        assert_eq!(spacing.space_6, px(24.));
        assert_eq!(spacing.space_8, px(32.));
    }

    #[test]
    fn radius_scale_has_stable_semantic_steps() {
        let radius = RadiusTokens::default();

        assert_eq!(radius.none, px(0.));
        assert_eq!(radius.xs, px(4.));
        assert_eq!(radius.sm, px(6.));
        assert_eq!(radius.md, px(8.));
        assert_eq!(radius.lg, px(12.));
        assert_eq!(radius.pill, px(999.));
    }

    #[test]
    fn control_scale_maps_legacy_sizes_without_changing_existing_layouts() {
        let control = ControlSizeTokens::default();

        assert_eq!(control.compact, px(24.));
        assert_eq!(control.small, px(28.));
        assert_eq!(control.default, px(32.));
        assert_eq!(control.medium, px(36.));
        assert_eq!(control.large, px(40.));
        assert_eq!(control.xlarge, px(44.));
        assert_eq!(control.hero, px(48.));

        assert_eq!(control.height(Size::XSmall), px(24.));
        assert_eq!(control.height(Size::Small), px(28.));
        assert_eq!(control.height(Size::Medium), px(32.));
        assert_eq!(control.height(Size::Large), px(40.));
        assert_eq!(control.height(Size::Size(px(37.))), px(37.));
    }

    #[test]
    fn compatibility_accessors_preserve_existing_component_geometry() {
        let geometry = ThemeGeometry::default();

        assert_eq!(geometry.table_row_height(Size::XSmall), px(26.));
        assert_eq!(geometry.table_row_height(Size::Small), px(30.));
        assert_eq!(geometry.table_row_height(Size::Medium), px(32.));
        assert_eq!(geometry.table_row_height(Size::Large), px(40.));

        assert_eq!(geometry.input_padding_x(Size::XSmall), px(4.));
        assert_eq!(geometry.input_padding_x(Size::Small), px(8.));
        assert_eq!(geometry.input_padding_x(Size::Medium), px(12.));
        assert_eq!(geometry.input_padding_x(Size::Large), px(16.));

        assert_eq!(geometry.input_padding_y(Size::XSmall), px(0.));
        assert_eq!(geometry.input_padding_y(Size::Small), px(2.));
        assert_eq!(geometry.input_padding_y(Size::Medium), px(8.));
        assert_eq!(geometry.input_padding_y(Size::Large), px(10.));
    }

    #[test]
    fn shell_layout_defaults_are_internally_consistent() {
        let layout = LayoutSizeTokens::default();

        assert_eq!(layout.title_bar, px(34.));
        assert_eq!(layout.title_bar_content_padding, px(12.));
        assert_eq!(layout.macos_title_bar_content_padding, px(80.));
        assert_eq!(layout.macos_compact_title_bar_content_padding, px(36.));
        assert_eq!(layout.macos_rail_title_bar_height, px(40.));
        assert_eq!(layout.window_control_width, px(34.));
        assert_eq!(layout.tab_bar, px(40.));
        assert_eq!(layout.tab_item, px(32.));
        assert_eq!(layout.command_bar, px(48.));
        assert_eq!(layout.panel_header, px(36.));
        assert_eq!(layout.list_header, px(28.));
        assert_eq!(layout.status_bar, px(28.));
        assert_eq!(layout.global_rail, px(52.));
        assert_eq!(layout.compact_rail, px(44.));
        assert_eq!(layout.global_rail_item, px(40.));
        assert_eq!(layout.workspace_min, px(640.));
        assert!(layout.context_sidebar_min <= layout.context_sidebar_default);
        assert!(layout.context_sidebar_default <= layout.context_sidebar_max);
        assert!(layout.utility_panel_min <= layout.utility_panel_default);
        assert!(layout.utility_panel_default <= layout.utility_panel_max);
    }

    #[test]
    fn tree_geometry_has_stable_density_and_disclosure_slots() {
        let tree = TreeListGeometry::default();

        assert_eq!(tree.row_height, px(28.));
        assert_eq!(tree.indent, px(16.));
        assert_eq!(tree.base_padding, px(8.));
        assert_eq!(tree.disclosure_size, px(16.));
    }

    #[test]
    fn resize_tokens_keep_visual_line_and_hit_target_separate() {
        let resize = ResizeGeometry::default();

        assert_eq!(resize.visible_line, px(1.));
        assert_eq!(resize.edge_padding, px(4.));
        assert_eq!(resize.hit_area(), px(9.));
        assert_eq!(resize.collapsed_threshold, px(44.));
    }

    #[test]
    fn overlay_opacity_and_motion_defaults_follow_the_design_system() {
        let geometry = ThemeGeometry::default();

        assert_eq!(geometry.overlay.edge_margin, px(16.));
        assert_eq!(geometry.overlay.content_padding, px(16.));
        assert_eq!(geometry.opacity.disabled, 0.45);
        assert_eq!(geometry.opacity.loading_scrim, 0.24);
        assert_eq!(geometry.opacity.scrim, 0.48);
        assert!(geometry.motion.fast_ms < geometry.motion.normal_ms);
        assert!(geometry.motion.normal_ms < geometry.motion.slow_ms);
    }
}
