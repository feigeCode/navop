use gpui::{Edges, Pixels, px};
use gpui_component::Size;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

/// Application-wide UI density. This scales application-owned geometry and the
/// GPUI rem basis together so dense desktop surfaces stay internally coherent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DensityScale {
    Compact,
    #[default]
    Default,
    Comfortable,
}

impl DensityScale {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Default, Self::Comfortable];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Default => "default",
            Self::Comfortable => "comfortable",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "compact" => Self::Compact,
            "comfortable" => Self::Comfortable,
            _ => Self::Default,
        }
    }

    /// Geometry and spacing multiplier.
    pub fn geometry_scale(self) -> f32 {
        match self {
            Self::Compact => 0.94,
            Self::Default => 1.0,
            Self::Comfortable => 1.08,
        }
    }

    /// Typography multiplier applied to the GPUI rem basis.
    pub fn font_scale(self) -> f32 {
        match self {
            Self::Compact => 0.97,
            Self::Default => 1.0,
            Self::Comfortable => 1.06,
        }
    }
}

static ACTIVE_DENSITY: AtomicU8 = AtomicU8::new(1);

/// Set the process-wide density used by [`theme_geometry`].
///
/// The application calls this from `AppSettings::apply`; tests that do not
/// initialize application settings continue to see [`DensityScale::Default`].
pub fn set_density_scale(density: DensityScale) {
    let encoded = match density {
        DensityScale::Compact => 0,
        DensityScale::Default => 1,
        DensityScale::Comfortable => 2,
    };
    ACTIVE_DENSITY.store(encoded, Ordering::Relaxed);
}

pub fn active_density_scale() -> DensityScale {
    match ACTIVE_DENSITY.load(Ordering::Relaxed) {
        0 => DensityScale::Compact,
        2 => DensityScale::Comfortable,
        _ => DensityScale::Default,
    }
}

/// Geometry tokens shared by the application shell, components, and pages.
///
/// These values intentionally live beside the color theme while remaining
/// independent from user-authored color theme files. This gives migrated UI a
/// single source of truth without changing the existing `ThemeConfig` schema.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    /// Build geometry for a density without changing the process-wide setting.
    pub fn for_density(density: DensityScale) -> Self {
        let mut geometry = Self::default();
        let scale = density.geometry_scale();
        geometry.spacing = geometry.spacing.scaled(scale);
        geometry.control = geometry.control.scaled(scale);
        geometry.layout = geometry.layout.scaled(scale);
        geometry.tree = geometry.tree.scaled(scale);
        geometry
    }

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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

impl SpacingTokens {
    fn scaled(self, scale: f32) -> Self {
        Self {
            space_1: (self.space_1 * scale).round(),
            space_2: (self.space_2 * scale).round(),
            space_3: (self.space_3 * scale).round(),
            space_4: (self.space_4 * scale).round(),
            space_5: (self.space_5 * scale).round(),
            space_6: (self.space_6 * scale).round(),
            space_8: (self.space_8 * scale).round(),
        }
    }
}

/// Semantic corner-radius scale.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

impl ControlSizeTokens {
    fn scaled(self, scale: f32) -> Self {
        Self {
            compact: (self.compact * scale).round(),
            small: (self.small * scale).round(),
            default: (self.default * scale).round(),
            medium: (self.medium * scale).round(),
            large: (self.large * scale).round(),
            xlarge: (self.xlarge * scale).round(),
            hero: (self.hero * scale).round(),
        }
    }
}

/// Application shell and workspace role sizes.
///
/// Candidate dimensions such as the 56px global rail are defined here before
/// page migration. Existing surfaces should switch to these roles atomically
/// with their related offsets and platform safe areas.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
            global_rail: px(56.),
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

impl LayoutSizeTokens {
    fn scaled(self, scale: f32) -> Self {
        Self {
            title_bar: (self.title_bar * scale).round(),
            title_bar_content_padding: (self.title_bar_content_padding * scale).round(),
            macos_title_bar_content_padding: (self.macos_title_bar_content_padding * scale).round(),
            macos_compact_title_bar_content_padding: (self.macos_compact_title_bar_content_padding
                * scale)
                .round(),
            macos_rail_title_bar_height: (self.macos_rail_title_bar_height * scale).round(),
            window_control_width: (self.window_control_width * scale).round(),
            tab_bar: (self.tab_bar * scale).round(),
            tab_item: (self.tab_item * scale).round(),
            command_bar: (self.command_bar * scale).round(),
            panel_header: (self.panel_header * scale).round(),
            embedded_panel_header: (self.embedded_panel_header * scale).round(),
            dock_panel_header: (self.dock_panel_header * scale).round(),
            list_header: (self.list_header * scale).round(),
            collapsed_bottom_dock_header: (self.collapsed_bottom_dock_header * scale).round(),
            status_bar: (self.status_bar * scale).round(),
            global_rail: (self.global_rail * scale).round(),
            compact_rail: (self.compact_rail * scale).round(),
            global_rail_item: (self.global_rail_item * scale).round(),
            context_sidebar_default: (self.context_sidebar_default * scale).round(),
            context_sidebar_min: (self.context_sidebar_min * scale).round(),
            context_sidebar_max: (self.context_sidebar_max * scale).round(),
            utility_panel_default: (self.utility_panel_default * scale).round(),
            utility_panel_min: (self.utility_panel_min * scale).round(),
            utility_panel_max: (self.utility_panel_max * scale).round(),
            sidebar_panel_min: (self.sidebar_panel_min * scale).round(),
            sidebar_center_min: (self.sidebar_center_min * scale).round(),
            sidebar_bottom_default: (self.sidebar_bottom_default * scale).round(),
            workspace_min: (self.workspace_min * scale).round(),
        }
    }
}

/// Shared geometry for compact trees and hierarchical lists.
///
/// These defaults describe the common 32px tree used by data explorers.
/// Surfaces with an intentional density difference may keep a local row height
/// or indent while still sharing base padding and disclosure geometry.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
            row_height: px(32.),
            indent: px(16.),
            base_padding: px(8.),
            disclosure_size: px(16.),
        }
    }
}

impl TreeListGeometry {
    fn scaled(self, scale: f32) -> Self {
        Self {
            row_height: (self.row_height * scale).round(),
            indent: (self.indent * scale).round(),
            base_padding: (self.base_padding * scale).round(),
            disclosure_size: (self.disclosure_size * scale).round(),
        }
    }
}

/// Border-width scale. Color remains a semantic color-theme concern.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
    use gpui::px;
    use gpui_component::Size;

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
        assert_eq!(layout.global_rail, px(56.));
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

        assert_eq!(tree.row_height, px(32.));
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

/// The application geometry tokens for the active density.
///
/// Navop keeps these in the application layer rather than upstream
/// `gpui-component`. Density is a process-wide preference because geometry is
/// consumed by synchronous render helpers that do not take an `App`.
pub fn theme_geometry() -> ThemeGeometry {
    ThemeGeometry::for_density(active_density_scale())
}

#[cfg(test)]
mod density_tests {
    use super::*;

    #[test]
    fn density_scales_keep_compact_default_and_comfortable_ordered() {
        let compact = ThemeGeometry::for_density(DensityScale::Compact);
        let default = ThemeGeometry::for_density(DensityScale::Default);
        let comfortable = ThemeGeometry::for_density(DensityScale::Comfortable);

        assert!(compact.tree.row_height < default.tree.row_height);
        assert!(default.tree.row_height < comfortable.tree.row_height);
        assert!(compact.control.default < default.control.default);
        assert!(default.control.default < comfortable.control.default);
    }

    #[test]
    fn density_round_trips_through_its_storage_value() {
        for density in DensityScale::ALL {
            assert_eq!(density, DensityScale::from_str(density.as_str()));
        }
    }
}
