use gpui::SharedString;

use super::{IconName, IconNamed};

/// The visual and semantic family an icon belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconKind {
    /// Monochrome outline icon used for actions and controls.
    FunctionalOutline,
    /// Monochrome filled icon used for actions and selected states.
    FunctionalFilled,
    /// Original-color icon used only to identify a product, platform, or database brand.
    BrandColor,
    /// Icon representing a domain object such as a database, table, file, or terminal.
    ObjectGlyph,
}

impl IconKind {
    /// Returns whether this kind is valid for a functional icon wrapper.
    pub const fn is_functional(self) -> bool {
        matches!(self, Self::FunctionalOutline | Self::FunctionalFilled)
    }
}

/// Auditable metadata for an embedded icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconMetadata {
    /// The icon's semantic and visual family.
    pub kind: IconKind,
    /// The canonical asset path used by the renderer.
    pub canonical_path: SharedString,
    /// Optional upstream source attribution.
    pub source: Option<&'static str>,
    /// Optional upstream license identifier.
    pub license: Option<&'static str>,
}

impl IconName {
    /// Returns the icon's semantic and visual family.
    pub const fn kind(self) -> IconKind {
        use IconKind::{BrandColor, FunctionalFilled, FunctionalOutline, ObjectGlyph};

        match self {
            Self::GitHub
            | Self::LinuxPenguinColor
            | Self::UbuntuColor
            | Self::RedhatColor
            | Self::CentosColor
            | Self::DebianColor
            | Self::AlmalinuxColor
            | Self::OpensuseColor
            | Self::MacosColor
            | Self::WindowsColor
            | Self::DockerColor
            | Self::RockyColor
            | Self::FedoraColor
            | Self::ArchColor
            | Self::AlpineColor
            | Self::AmazonColor
            | Self::KaliColor
            | Self::AlinuxColor
            | Self::OpenEulerColor
            | Self::FreebsdColor
            | Self::CiscoColor
            | Self::JuniperColor
            | Self::HuaweiColor
            | Self::H3cColor
            | Self::HpeColor
            | Self::MikrotikColor
            | Self::FortinetColor
            | Self::PaloaltoColor
            | Self::ZyxelColor
            | Self::MongoDB
            | Self::MySQLColor
            | Self::MySQLLineColor
            | Self::SQLiteColor
            | Self::SQLiteLineColor
            | Self::PostgreSQLColor
            | Self::PostgreSQLLineColor
            | Self::MSSQLColor
            | Self::MSSQLLineColor
            | Self::OracleColor
            | Self::OracleLineColor
            | Self::ClickHouseColor
            | Self::ClickHouseLineColor
            | Self::TDengineColor
            | Self::TDengineLineColor
            | Self::Mqtt
            | Self::Redis
            | Self::RedisColor
            | Self::DuckDB
            | Self::AI => BrandColor,

            Self::Paste
            | Self::Dash
            | Self::ResizeCorner
            | Self::StarFill
            | Self::WindowClose
            | Self::WindowMaximize
            | Self::WindowMinimize
            | Self::WindowRestore
            | Self::All
            | Self::Edit
            | Self::Filter
            | Self::Refresh
            | Self::Sync
            | Self::Upload
            | Self::NewFolder
            | Self::EditBorder
            | Self::Remove
            | Self::Export
            | Self::Home
            | Self::StatusConnected
            | Self::StatusConnectedLocked
            | Self::StatusDisconnected => FunctionalFilled,

            Self::NotesColor
            | Self::TeamColor
            | Self::File
            | Self::Json
            | Self::MarkdownColor
            | Self::RichTextColor
            | Self::Folder
            | Self::FolderClosed
            | Self::FolderOpen
            | Self::FolderOpenColor
            | Self::QueryFolderColor
            | Self::QueryFolderOpenColor
            | Self::TerminalFileManagerColor
            | Self::ExtensionsColor
            | Self::HardDrive
            | Self::Inbox
            | Self::MemoryStick
            | Self::Network
            | Self::SquareTerminalColor
            | Self::TerminalQuickCommandColor
            | Self::UserColor
            | Self::Database
            | Self::Table
            | Self::Column
            | Self::Key
            | Self::View
            | Self::Function
            | Self::Schema
            | Self::GoldKey
            | Self::PrimaryKey
            | Self::Procedure
            | Self::Trigger
            | Self::FolderViews
            | Self::FolderQueries
            | Self::FolderFunctions
            | Self::FolderIndexes
            | Self::FolderTables
            | Self::FolderSchema
            | Self::FolderColumns
            | Self::FolderTriggers
            | Self::FolderProcedures
            | Self::FolderForeignKeys
            | Self::FolderCheckConstraints
            | Self::FolderSequences
            | Self::CheckConstraint
            | Self::Sequence
            | Self::Query
            | Self::Index
            | Self::Terminal
            | Self::TerminalColor
            | Self::TerminalHistoryColor
            | Self::TerminalBroadcastColor
            | Self::RichInputColor
            | Self::AppsColor
            | Self::Workspace
            | Self::Folder1
            | Self::FolderOpen1
            | Self::TableData
            | Self::TableDesign
            | Self::TableDesignTool
            | Self::SchemaCompare
            | Self::DataModel
            | Self::Server
            | Self::SettingColor
            | Self::SerialPort
            | Self::Monitor
            | Self::TerminalServerMonitorColor
            | Self::PortForwardingColor
            | Self::Rdp
            | Self::Vnc
            | Self::ServerLine
            | Self::TerminalLine
            | Self::DatabaseLine
            | Self::RedisLine
            | Self::MongoDBLine
            | Self::MqttLine
            | Self::SerialLine
            | Self::PortForwardingLine
            | Self::RdpLine
            | Self::VncLine
            | Self::AILine
            | Self::TeamLine
            | Self::NotesLine
            | Self::ExtensionsLine => ObjectGlyph,

            Self::ALargeSmall
            | Self::AlignCenter
            | Self::AlignLeft
            | Self::AlignRight
            | Self::ArrowDown
            | Self::ArrowLeft
            | Self::ArrowRight
            | Self::ArrowUp
            | Self::Asterisk
            | Self::Battery
            | Self::BatteryCharging
            | Self::BatteryFull
            | Self::BatteryLow
            | Self::BatteryMedium
            | Self::BatteryWarning
            | Self::Bell
            | Self::BookOpen
            | Self::Bot
            | Self::Building2
            | Self::Calendar
            | Self::CaseSensitive
            | Self::ChartPie
            | Self::Check
            | Self::ChevronDown
            | Self::ChevronLeft
            | Self::ChevronRight
            | Self::ChevronsUpDown
            | Self::ChevronUp
            | Self::CircleCheck
            | Self::CircleUser
            | Self::CircleX
            | Self::Close
            | Self::Copy
            | Self::Cpu
            | Self::Delete
            | Self::Ellipsis
            | Self::EllipsisVertical
            | Self::ExternalLink
            | Self::Eye
            | Self::EyeOff
            | Self::Unarchive
            | Self::Frame
            | Self::GalleryVerticalEnd
            | Self::GitBranch
            | Self::Globe
            | Self::Heart
            | Self::HeartOff
            | Self::Info
            | Self::Inspector
            | Self::LayoutDashboard
            | Self::ListChecks
            | Self::Loader
            | Self::LoaderCircle
            | Self::LocateActiveTab
            | Self::Map
            | Self::Maximize
            | Self::Menu
            | Self::Minimize
            | Self::Minus
            | Self::Moon
            | Self::Palette
            | Self::PanelBottom
            | Self::PanelBottomOpen
            | Self::PanelLeft
            | Self::PanelLeftClose
            | Self::PanelLeftOpen
            | Self::PanelRight
            | Self::PanelRightClose
            | Self::PanelRightOpen
            | Self::Pause
            | Self::Pin
            | Self::Play
            | Self::Plus
            | Self::Redo
            | Self::Redo2
            | Self::Replace
            | Self::Save
            | Self::Search
            | Self::Settings
            | Self::Settings2
            | Self::SortAscending
            | Self::SortDescending
            | Self::SquareTerminal
            | Self::Star
            | Self::StarOff
            | Self::Sun
            | Self::ThumbsDown
            | Self::ThumbsUp
            | Self::TriangleAlert
            | Self::Undo
            | Self::Undo2
            | Self::User
            | Self::Apps => FunctionalOutline,
        }
    }

    /// Returns auditable metadata for this icon.
    pub fn metadata(self) -> IconMetadata {
        IconMetadata {
            kind: self.kind(),
            canonical_path: self.path(),
            source: None,
            license: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{IconKind, IconName};

    #[test]
    fn representative_icons_have_semantic_kinds() {
        assert_eq!(IconName::Plus.kind(), IconKind::FunctionalOutline);
        assert_eq!(IconName::StarFill.kind(), IconKind::FunctionalFilled);
        assert_eq!(IconName::PostgreSQLColor.kind(), IconKind::BrandColor);
        assert_eq!(IconName::Table.kind(), IconKind::ObjectGlyph);
    }

    #[test]
    fn color_suffix_does_not_imply_brand_identity() {
        assert_eq!(IconName::NotesColor.kind(), IconKind::ObjectGlyph);
        assert_eq!(
            IconName::TerminalQuickCommandColor.kind(),
            IconKind::ObjectGlyph
        );
    }

    #[test]
    fn brand_identity_does_not_require_color_suffix() {
        assert_eq!(IconName::GitHub.kind(), IconKind::BrandColor);
        assert_eq!(IconName::MongoDB.kind(), IconKind::BrandColor);
        assert_eq!(IconName::Redis.kind(), IconKind::BrandColor);
        assert_eq!(IconName::DuckDB.kind(), IconKind::BrandColor);
    }

    #[test]
    fn connection_navigation_icons_are_monochrome_object_glyphs() {
        assert_eq!(IconName::RedisLine.kind(), IconKind::ObjectGlyph);
        assert_eq!(IconName::MongoDBLine.kind(), IconKind::ObjectGlyph);
    }

    #[test]
    fn metadata_uses_the_renderers_canonical_asset_path() {
        let metadata = IconName::PostgreSQLColor.metadata();

        assert_eq!(metadata.kind, IconKind::BrandColor);
        assert_eq!(
            metadata.canonical_path.as_ref(),
            "icons/postgresql_color.svg"
        );
    }
}
