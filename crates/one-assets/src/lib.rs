//! Navop's own icon catalog.
//!
//! The `IconName` enum is generated from the SVG files in `assets/icons`
//! at build time. It implements `gpui_component::IconNamed` (including
//! `color_mode`), so every variant converts into `gpui_component::Icon`
//! and inherits the right mono/color rendering automatically.

use gpui::{AnyElement, App, IntoElement, RenderOnce, Window};
use gpui_component::Icon;

mod native_assets;
pub use native_assets::Assets;

include!(concat!(env!("OUT_DIR"), "/icon_name.rs"));

impl IconName {
    /// Return the icon as an `Entity<Icon>`.
    pub fn view(self, cx: &mut App) -> gpui::Entity<Icon> {
        Icon::new(self).view(cx)
    }

    /// Render with intrinsic colors (brand / product marks).
    pub fn color(self) -> Icon {
        Icon::new(self).color()
    }

    /// Render tinted by the ambient text color.
    pub fn mono(self) -> Icon {
        Icon::new(self).mono()
    }
}

impl From<IconName> for AnyElement {
    fn from(name: IconName) -> Self {
        Icon::new(name).into_any_element()
    }
}

impl RenderOnce for IconName {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Icon::new(self)
    }
}

// Preserve the established product acronyms as PascalCase aliases without
// hard-coding the registry. `icon_named!` lowercases acronyms (e.g. `Mongodb`),
// so re-expose the familiar spellings used across the codebase.
#[allow(non_upper_case_globals)]
impl IconName {
    pub const AI: Self = Self::Ai;
    pub const AILine: Self = Self::AiLine;
    pub const ClickHouseColor: Self = Self::ClickhouseColor;
    pub const ClickHouseLineColor: Self = Self::ClickhouseLineColor;
    pub const DuckDB: Self = Self::Duckdb;
    pub const GitHub: Self = Self::Github;
    pub const MongoDB: Self = Self::Mongodb;
    pub const MongoDBLine: Self = Self::MongodbLine;
    pub const MSSQLColor: Self = Self::MssqlColor;
    pub const MSSQLLineColor: Self = Self::MssqlLineColor;
    pub const MySQLColor: Self = Self::MysqlColor;
    pub const MySQLLineColor: Self = Self::MysqlLineColor;
    pub const OpenEulerColor: Self = Self::OpeneulerColor;
    pub const PostgreSQLColor: Self = Self::PostgresqlColor;
    pub const PostgreSQLLineColor: Self = Self::PostgresqlLineColor;
    pub const SQLiteColor: Self = Self::SqliteColor;
    pub const SQLiteLineColor: Self = Self::SqliteLineColor;
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_component::IconNamed as _;

    // Ground truth from the CE fork's own icon tests. The SVG content
    // classifier must produce exactly these modes, otherwise brand logos
    // (MongoDB, Redis, ...) would render tinted instead of in their
    // intrinsic colors, or line variants would render colorized.
    #[test]
    fn generated_icons_use_svg_content_for_the_default_mode() {
        for icon in [
            IconName::MongoDB,
            IconName::Redis,
            IconName::Database,
            IconName::Terminal,
            IconName::Vnc,
            IconName::Procedure,
            IconName::FolderFunctions,
            IconName::StatusConnectedLocked,
        ] {
            assert_eq!(icon.color_mode(), gpui_component::IconColorMode::Color);
        }
        assert_eq!(IconName::RdpLine.color_mode(), gpui_component::IconColorMode::Mono);
        assert_eq!(IconName::VncLine.color_mode(), gpui_component::IconColorMode::Mono);
        assert_eq!(IconName::Monitor.color_mode(), gpui_component::IconColorMode::Mono);
        assert_eq!(IconName::Paste.color_mode(), gpui_component::IconColorMode::Mono);
    }

    #[test]
    fn explicit_color_modes_override_the_generated_default() {
        assert_eq!(
            IconName::RdpLine.color().color_mode,
            gpui_component::IconColorMode::Color
        );
        assert_eq!(
            IconName::MongoDB.mono().color_mode,
            gpui_component::IconColorMode::Mono
        );
    }

    #[test]
    fn acronym_aliases_resolve_to_their_generated_variants() {
        assert_eq!(IconName::MongoDB, IconName::Mongodb);
        assert_eq!(IconName::PostgreSQLColor, IconName::PostgresqlColor);
        assert_eq!(IconName::AI, IconName::Ai);
    }

    #[test]
    fn generated_paths_match_the_asset_source() {
        assert_eq!(IconName::MongoDB.path().as_ref(), "icons/mongodb.svg");
        assert_eq!(IconName::ArrowUp.path().as_ref(), "icons/arrow-up.svg");
    }
}
