use crate::{ActiveTheme, Sizable, Size};
use gpui::{
    AnyElement, App, AppContext, Context, Entity, Hsla, ImageSource, IntoElement, ParentElement,
    Radians, Render, RenderOnce, SharedString, StyleRefinement, Styled, Svg, Transformation,
    Window, div, img, prelude::FluentBuilder as _, svg,
};
use palette::IntoColor;
// use gpui_component_macros::icon_named;
use std::path::PathBuf;

mod metadata;
mod size;
mod wrappers;

pub use metadata::{IconKind, IconMetadata};
pub use size::IconSize;
use size::{resolve_icon_size, should_apply_resolved_size};
pub use wrappers::{BrandIcon, FunctionalIcon, ObjectIcon};

/// Types implementing this trait can automatically be converted to [`Icon`].
///
/// This allows you to implement a custom version of [`IconName`] that functions as a drop-in
/// replacement for other UI components.
pub trait IconNamed {
    /// Returns the embedded path of the icon.
    fn path(self) -> SharedString;
}

impl<T: IconNamed> From<T> for Icon {
    fn from(value: T) -> Self {
        Icon::build(value)
    }
}

// icon_named!(IconName, "../assets/assets/icons");

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IconColorMode {
    /// Monochrome mode: uses SVG with text_color tinting (default)
    #[default]
    Mono,
    /// Color mode: renders the original SVG/image colors
    Color,
}

/// The name of an icon in the asset bundle.
#[derive(IntoElement, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconName {
    ALargeSmall,
    AlignCenter,
    AlignLeft,
    AlignRight,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    Asterisk,
    Battery,
    BatteryCharging,
    BatteryFull,
    BatteryLow,
    BatteryMedium,
    BatteryWarning,
    Bell,
    BookOpen,
    NotesColor,
    Bot,
    Building2,
    TeamColor,
    Calendar,
    CaseSensitive,
    ChartPie,
    Check,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    ChevronsUpDown,
    ChevronUp,
    CircleCheck,
    CircleUser,
    CircleX,
    Close,
    Copy,
    Paste,
    Cpu,
    Dash,
    Delete,
    Ellipsis,
    EllipsisVertical,
    ExternalLink,
    Eye,
    EyeOff,
    File,
    Json,
    MarkdownColor,
    RichTextColor,
    Unarchive,
    Folder,
    FolderClosed,
    FolderOpen,
    FolderOpenColor,
    QueryFolderColor,
    QueryFolderOpenColor,
    TerminalFileManagerColor,
    Frame,
    GalleryVerticalEnd,
    ExtensionsColor,
    GitBranch,
    GitHub,
    Globe,
    HardDrive,
    Heart,
    HeartOff,
    Inbox,
    Info,
    Inspector,
    LayoutDashboard,
    ListChecks,
    Loader,
    LoaderCircle,
    LocateActiveTab,
    Map,
    Maximize,
    MemoryStick,
    Menu,
    Minimize,
    Minus,
    Moon,
    Network,
    Palette,
    PanelBottom,
    PanelBottomOpen,
    PanelLeft,
    PanelLeftClose,
    PanelLeftOpen,
    PanelRight,
    PanelRightClose,
    PanelRightOpen,
    Pause,
    Pin,
    Play,
    Plus,
    Redo,
    Redo2,
    Replace,
    ResizeCorner,
    Save,
    Search,
    Settings,
    Settings2,
    SortAscending,
    SortDescending,
    SquareTerminal,
    SquareTerminalColor,
    TerminalQuickCommandColor,
    Star,
    StarFill,
    StarOff,
    StatusConnected,
    StatusConnectedLocked,
    StatusDisconnected,
    Sun,
    ThumbsDown,
    ThumbsUp,
    TriangleAlert,
    Undo,
    Undo2,
    User,
    UserColor,
    WindowClose,
    WindowMaximize,
    WindowMinimize,
    WindowRestore,
    Database,
    Table,
    Column,
    Key,
    View,
    Function,
    Schema,
    GoldKey,
    PrimaryKey,
    Procedure,
    Trigger,
    FolderViews,
    FolderQueries,
    FolderFunctions,
    FolderIndexes,
    FolderTables,
    FolderSchema,
    FolderColumns,
    FolderTriggers,
    FolderProcedures,
    FolderForeignKeys,
    FolderCheckConstraints,
    FolderSequences,
    CheckConstraint,
    Sequence,
    Query,
    Index,
    Redis,
    Terminal,
    TerminalColor,
    LinuxPenguinColor,
    UbuntuColor,
    RedhatColor,
    CentosColor,
    DebianColor,
    AlmalinuxColor,
    OpensuseColor,
    MacosColor,
    WindowsColor,
    DockerColor,
    RockyColor,
    FedoraColor,
    ArchColor,
    AlpineColor,
    AmazonColor,
    KaliColor,
    AlinuxColor,
    OpenEulerColor,
    FreebsdColor,
    CiscoColor,
    JuniperColor,
    HuaweiColor,
    H3cColor,
    HpeColor,
    MikrotikColor,
    FortinetColor,
    PaloaltoColor,
    ZyxelColor,
    TerminalHistoryColor,
    TerminalBroadcastColor,
    RichInputColor,
    Apps,
    AppsColor,
    MongoDB,
    Mqtt,
    MySQLColor,
    MySQLLineColor,
    SQLiteColor,
    SQLiteLineColor,
    PostgreSQLColor,
    PostgreSQLLineColor,
    MSSQLColor,
    MSSQLLineColor,
    OracleColor,
    OracleLineColor,
    ClickHouseColor,
    ClickHouseLineColor,
    TDengineColor,
    Workspace,
    RedisColor,
    All,
    Edit,
    Filter,
    Refresh,
    Sync,
    Upload,
    NewFolder,
    EditBorder,
    Folder1,
    FolderOpen1,
    Remove,
    TableData,
    TableDesign,
    TableDesignTool,
    SchemaCompare,
    DataModel,
    Server,
    Export,
    AI,
    Home,
    SettingColor,
    SerialPort,
    Monitor,
    TerminalServerMonitorColor,
    PortForwardingColor,
    Rdp,
    Vnc,
    DuckDB,
    ServerLine,
    TerminalLine,
    DatabaseLine,
    RedisLine,
    MongoDBLine,
    MqttLine,
    TDengineLineColor,
    SerialLine,
    PortForwardingLine,
    RdpLine,
    VncLine,
    AILine,
    TeamLine,
    NotesLine,
    ExtensionsLine,
}

impl IconName {
    /// All icons exposed by the embedded asset registry, in stable declaration order.
    pub const ALL: &'static [Self] = &[
        Self::ALargeSmall,
        Self::AlignCenter,
        Self::AlignLeft,
        Self::AlignRight,
        Self::ArrowDown,
        Self::ArrowLeft,
        Self::ArrowRight,
        Self::ArrowUp,
        Self::Asterisk,
        Self::Battery,
        Self::BatteryCharging,
        Self::BatteryFull,
        Self::BatteryLow,
        Self::BatteryMedium,
        Self::BatteryWarning,
        Self::Bell,
        Self::BookOpen,
        Self::NotesColor,
        Self::Bot,
        Self::Building2,
        Self::TeamColor,
        Self::Calendar,
        Self::CaseSensitive,
        Self::ChartPie,
        Self::Check,
        Self::ChevronDown,
        Self::ChevronLeft,
        Self::ChevronRight,
        Self::ChevronsUpDown,
        Self::ChevronUp,
        Self::CircleCheck,
        Self::CircleUser,
        Self::CircleX,
        Self::Close,
        Self::Copy,
        Self::Paste,
        Self::Cpu,
        Self::Dash,
        Self::Delete,
        Self::Ellipsis,
        Self::EllipsisVertical,
        Self::ExternalLink,
        Self::Eye,
        Self::EyeOff,
        Self::File,
        Self::Json,
        Self::MarkdownColor,
        Self::RichTextColor,
        Self::Unarchive,
        Self::Folder,
        Self::FolderClosed,
        Self::FolderOpen,
        Self::FolderOpenColor,
        Self::QueryFolderColor,
        Self::QueryFolderOpenColor,
        Self::TerminalFileManagerColor,
        Self::Frame,
        Self::GalleryVerticalEnd,
        Self::ExtensionsColor,
        Self::GitBranch,
        Self::GitHub,
        Self::Globe,
        Self::HardDrive,
        Self::Heart,
        Self::HeartOff,
        Self::Inbox,
        Self::Info,
        Self::Inspector,
        Self::LayoutDashboard,
        Self::ListChecks,
        Self::Loader,
        Self::LoaderCircle,
        Self::LocateActiveTab,
        Self::Map,
        Self::Maximize,
        Self::MemoryStick,
        Self::Menu,
        Self::Minimize,
        Self::Minus,
        Self::Moon,
        Self::Network,
        Self::Palette,
        Self::PanelBottom,
        Self::PanelBottomOpen,
        Self::PanelLeft,
        Self::PanelLeftClose,
        Self::PanelLeftOpen,
        Self::PanelRight,
        Self::PanelRightClose,
        Self::PanelRightOpen,
        Self::Pause,
        Self::Pin,
        Self::Play,
        Self::Plus,
        Self::Redo,
        Self::Redo2,
        Self::Replace,
        Self::ResizeCorner,
        Self::Save,
        Self::Search,
        Self::Settings,
        Self::Settings2,
        Self::SortAscending,
        Self::SortDescending,
        Self::SquareTerminal,
        Self::SquareTerminalColor,
        Self::TerminalQuickCommandColor,
        Self::Star,
        Self::StarFill,
        Self::StarOff,
        Self::StatusConnected,
        Self::StatusConnectedLocked,
        Self::StatusDisconnected,
        Self::Sun,
        Self::ThumbsDown,
        Self::ThumbsUp,
        Self::TriangleAlert,
        Self::Undo,
        Self::Undo2,
        Self::User,
        Self::UserColor,
        Self::WindowClose,
        Self::WindowMaximize,
        Self::WindowMinimize,
        Self::WindowRestore,
        Self::Database,
        Self::Table,
        Self::Column,
        Self::Key,
        Self::View,
        Self::Function,
        Self::Schema,
        Self::GoldKey,
        Self::PrimaryKey,
        Self::Procedure,
        Self::Trigger,
        Self::FolderViews,
        Self::FolderQueries,
        Self::FolderFunctions,
        Self::FolderIndexes,
        Self::FolderTables,
        Self::FolderSchema,
        Self::FolderColumns,
        Self::FolderTriggers,
        Self::FolderProcedures,
        Self::FolderForeignKeys,
        Self::FolderCheckConstraints,
        Self::FolderSequences,
        Self::CheckConstraint,
        Self::Sequence,
        Self::Query,
        Self::Index,
        Self::Redis,
        Self::Terminal,
        Self::TerminalColor,
        Self::LinuxPenguinColor,
        Self::UbuntuColor,
        Self::RedhatColor,
        Self::CentosColor,
        Self::DebianColor,
        Self::AlmalinuxColor,
        Self::OpensuseColor,
        Self::MacosColor,
        Self::WindowsColor,
        Self::DockerColor,
        Self::RockyColor,
        Self::FedoraColor,
        Self::ArchColor,
        Self::AlpineColor,
        Self::AmazonColor,
        Self::KaliColor,
        Self::AlinuxColor,
        Self::OpenEulerColor,
        Self::FreebsdColor,
        Self::CiscoColor,
        Self::JuniperColor,
        Self::HuaweiColor,
        Self::H3cColor,
        Self::HpeColor,
        Self::MikrotikColor,
        Self::FortinetColor,
        Self::PaloaltoColor,
        Self::ZyxelColor,
        Self::TerminalHistoryColor,
        Self::TerminalBroadcastColor,
        Self::RichInputColor,
        Self::Apps,
        Self::AppsColor,
        Self::MongoDB,
        Self::Mqtt,
        Self::MySQLColor,
        Self::MySQLLineColor,
        Self::SQLiteColor,
        Self::SQLiteLineColor,
        Self::PostgreSQLColor,
        Self::PostgreSQLLineColor,
        Self::MSSQLColor,
        Self::MSSQLLineColor,
        Self::OracleColor,
        Self::OracleLineColor,
        Self::ClickHouseColor,
        Self::ClickHouseLineColor,
        Self::TDengineColor,
        Self::Workspace,
        Self::RedisColor,
        Self::All,
        Self::Edit,
        Self::Filter,
        Self::Refresh,
        Self::Sync,
        Self::Upload,
        Self::NewFolder,
        Self::EditBorder,
        Self::Folder1,
        Self::FolderOpen1,
        Self::Remove,
        Self::TableData,
        Self::TableDesign,
        Self::TableDesignTool,
        Self::SchemaCompare,
        Self::DataModel,
        Self::Server,
        Self::Export,
        Self::AI,
        Self::Home,
        Self::SettingColor,
        Self::SerialPort,
        Self::Monitor,
        Self::TerminalServerMonitorColor,
        Self::PortForwardingColor,
        Self::Rdp,
        Self::Vnc,
        Self::DuckDB,
        Self::ServerLine,
        Self::TerminalLine,
        Self::DatabaseLine,
        Self::RedisLine,
        Self::MongoDBLine,
        Self::SerialLine,
        Self::PortForwardingLine,
        Self::RdpLine,
        Self::VncLine,
        Self::AILine,
        Self::TeamLine,
        Self::NotesLine,
        Self::ExtensionsLine,
    ];

    /// Return the icon as a Entity<Icon>
    pub fn view(self, cx: &mut App) -> Entity<Icon> {
        Icon::build(self).view(cx)
    }

    /// Return the icon in color mode.
    pub fn color(self) -> Icon {
        Icon::build(self).color()
    }

    /// Return the icon in monochrome mode.
    pub fn mono(self) -> Icon {
        Icon::build(self).mono()
    }
}

impl IconNamed for IconName {
    fn path(self) -> SharedString {
        match self {
            Self::ALargeSmall => "icons/a-large-small.svg",
            Self::AlignCenter => "icons/align-center.svg",
            Self::AlignLeft => "icons/align-left.svg",
            Self::AlignRight => "icons/align-right.svg",
            Self::ArrowDown => "icons/arrow-down.svg",
            Self::ArrowLeft => "icons/arrow-left.svg",
            Self::ArrowRight => "icons/arrow-right.svg",
            Self::ArrowUp => "icons/arrow-up.svg",
            Self::Asterisk => "icons/asterisk.svg",
            Self::Battery => "icons/battery.svg",
            Self::BatteryCharging => "icons/battery-charging.svg",
            Self::BatteryFull => "icons/battery-full.svg",
            Self::BatteryLow => "icons/battery-low.svg",
            Self::BatteryMedium => "icons/battery-medium.svg",
            Self::BatteryWarning => "icons/battery-warning.svg",
            Self::Bell => "icons/bell.svg",
            Self::BookOpen => "icons/book-open.svg",
            Self::NotesColor => "icons/notes_color.svg",
            Self::Bot => "icons/bot.svg",
            Self::Building2 => "icons/building-2.svg",
            Self::TeamColor => "icons/team_color.svg",
            Self::Calendar => "icons/calendar.svg",
            Self::CaseSensitive => "icons/case-sensitive.svg",
            Self::ChartPie => "icons/chart-pie.svg",
            Self::Check => "icons/check.svg",
            Self::ChevronDown => "icons/chevron-down.svg",
            Self::ChevronLeft => "icons/chevron-left.svg",
            Self::ChevronRight => "icons/chevron-right.svg",
            Self::ChevronsUpDown => "icons/chevrons-up-down.svg",
            Self::ChevronUp => "icons/chevron-up.svg",
            Self::CircleCheck => "icons/circle-check.svg",
            Self::CircleUser => "icons/circle-user.svg",
            Self::CircleX => "icons/circle-x.svg",
            Self::Close => "icons/close.svg",
            Self::Copy => "icons/copy.svg",
            Self::Paste => "icons/paste.svg",
            Self::Cpu => "icons/cpu.svg",
            Self::Dash => "icons/dash.svg",
            Self::Delete => "icons/delete.svg",
            Self::Ellipsis => "icons/ellipsis.svg",
            Self::EllipsisVertical => "icons/ellipsis-vertical.svg",
            Self::ExternalLink => "icons/external-link.svg",
            Self::Eye => "icons/eye.svg",
            Self::EyeOff => "icons/eye-off.svg",
            Self::File => "icons/file.svg",
            Self::Json => "icons/json.svg",
            Self::MarkdownColor => "icons/markdown_color.svg",
            Self::RichTextColor => "icons/rich_text_color.svg",
            Self::Unarchive => "icons/unarchive.svg",
            Self::Folder => "icons/folder.svg",
            Self::FolderClosed => "icons/folder-closed.svg",
            Self::FolderOpen => "icons/folder-open.svg",
            Self::FolderOpenColor => "icons/folder_open_color.svg",
            Self::QueryFolderColor => "icons/query_folder_color.svg",
            Self::QueryFolderOpenColor => "icons/query_folder_open_color.svg",
            Self::TerminalFileManagerColor => "icons/terminal_file_manager_color.svg",
            Self::Frame => "icons/frame.svg",
            Self::GalleryVerticalEnd => "icons/gallery-vertical-end.svg",
            Self::ExtensionsColor => "icons/extensions_color.svg",
            Self::GitBranch => "icons/git-branch.svg",
            Self::GitHub => "icons/github.svg",
            Self::Globe => "icons/globe.svg",
            Self::HardDrive => "icons/hard-drive.svg",
            Self::Heart => "icons/heart.svg",
            Self::HeartOff => "icons/heart-off.svg",
            Self::Inbox => "icons/inbox.svg",
            Self::Info => "icons/info.svg",
            Self::Inspector => "icons/inspector.svg",
            Self::LayoutDashboard => "icons/layout-dashboard.svg",
            Self::ListChecks => "icons/list-checks.svg",
            Self::Loader => "icons/loader.svg",
            Self::LoaderCircle => "icons/loader-circle.svg",
            Self::LocateActiveTab => "icons/locate-active-tab.svg",
            Self::Map => "icons/map.svg",
            Self::Maximize => "icons/maximize.svg",
            Self::MemoryStick => "icons/memory-stick.svg",
            Self::Menu => "icons/menu.svg",
            Self::Minimize => "icons/minimize.svg",
            Self::Minus => "icons/minus.svg",
            Self::Moon => "icons/moon.svg",
            Self::Network => "icons/network.svg",
            Self::Palette => "icons/palette.svg",
            Self::PanelBottom => "icons/panel-bottom.svg",
            Self::PanelBottomOpen => "icons/panel-bottom-open.svg",
            Self::PanelLeft => "icons/panel-left.svg",
            Self::PanelLeftClose => "icons/panel-left-close.svg",
            Self::PanelLeftOpen => "icons/panel-left-open.svg",
            Self::PanelRight => "icons/panel-right.svg",
            Self::PanelRightClose => "icons/panel-right-close.svg",
            Self::PanelRightOpen => "icons/panel-right-open.svg",
            Self::Pause => "icons/pause.svg",
            Self::Pin => "icons/pin.svg",
            Self::Play => "icons/play.svg",
            Self::Plus => "icons/plus.svg",
            Self::Redo => "icons/redo.svg",
            Self::Redo2 => "icons/redo-2.svg",
            Self::Replace => "icons/replace.svg",
            Self::ResizeCorner => "icons/resize-corner.svg",
            Self::Save => "icons/save.svg",
            Self::Search => "icons/search.svg",
            Self::Settings => "icons/settings.svg",
            Self::Settings2 => "icons/settings-2.svg",
            Self::SortAscending => "icons/sort-ascending.svg",
            Self::SortDescending => "icons/sort-descending.svg",
            Self::SquareTerminal => "icons/square-terminal.svg",
            Self::SquareTerminalColor => "icons/square_terminal_color.svg",
            Self::TerminalQuickCommandColor => "icons/terminal_quick_command_color.svg",
            Self::Star => "icons/star.svg",
            Self::StarFill => "icons/star-fill.svg",
            Self::StarOff => "icons/star-off.svg",
            Self::StatusConnected => "icons/status-connected.svg",
            Self::StatusConnectedLocked => "icons/status-connected-locked.svg",
            Self::StatusDisconnected => "icons/status-disconnected.svg",
            Self::Sun => "icons/sun.svg",
            Self::ThumbsDown => "icons/thumbs-down.svg",
            Self::ThumbsUp => "icons/thumbs-up.svg",
            Self::TriangleAlert => "icons/triangle-alert.svg",
            Self::Undo => "icons/undo.svg",
            Self::Undo2 => "icons/undo-2.svg",
            Self::User => "icons/user.svg",
            Self::UserColor => "icons/user_color.svg",
            Self::WindowClose => "icons/window-close.svg",
            Self::WindowMaximize => "icons/window-maximize.svg",
            Self::WindowMinimize => "icons/window-minimize.svg",
            Self::WindowRestore => "icons/window-restore.svg",
            Self::Database => "icons/db.svg",
            Self::Schema => "icons/schema.svg",
            Self::Table => "icons/table.svg",
            Self::Folder1 => "icons/folder-1.svg",
            Self::FolderOpen1 => "icons/folder-open-1.svg",
            Self::View => "icons/view.svg",
            Self::Function => "icons/function.svg",
            Self::Column => "icons/column.svg",
            Self::Key => "icons/key.svg",
            Self::GoldKey => "icons/gold_key.svg",
            Self::PrimaryKey => "icons/primary-key.svg",
            Self::Procedure => "icons/procedure.svg",
            Self::Trigger => "icons/trigger.svg",
            Self::FolderViews => "icons/folder-views.svg",
            Self::FolderQueries => "icons/folder-queries.svg",
            Self::FolderFunctions => "icons/folder-functions.svg",
            Self::FolderIndexes => "icons/folder-indexes.svg",
            Self::FolderTables => "icons/folder-tables.svg",
            Self::FolderSchema => "icons/folder-schema.svg",
            Self::FolderColumns => "icons/folder-columns.svg",
            Self::FolderTriggers => "icons/folder-triggers.svg",
            Self::FolderProcedures => "icons/folder-procedures.svg",
            Self::FolderForeignKeys => "icons/folder-foreign-keys.svg",
            Self::FolderCheckConstraints => "icons/folder-check-constraints.svg",
            Self::FolderSequences => "icons/folder-sequences.svg",
            Self::CheckConstraint => "icons/check-constraint.svg",
            Self::Sequence => "icons/sequence.svg",
            Self::Query => "icons/query.svg",
            Self::Index => "icons/index.svg",
            Self::Redis => "icons/redis.svg",
            Self::Terminal => "icons/terminal.svg",
            Self::TerminalColor => "icons/terminal_color.svg",
            Self::LinuxPenguinColor => "icons/linux_penguin_color.svg",
            Self::UbuntuColor => "icons/ubuntu_color.svg",
            Self::RedhatColor => "icons/redhat_color.svg",
            Self::CentosColor => "icons/centos_color.svg",
            Self::DebianColor => "icons/debian_color.svg",
            Self::AlmalinuxColor => "icons/almalinux_color.svg",
            Self::OpensuseColor => "icons/opensuse_color.svg",
            Self::MacosColor => "icons/macos_color.svg",
            Self::WindowsColor => "icons/windows_color.svg",
            Self::DockerColor => "icons/docker_color.svg",
            Self::RockyColor => "icons/rocky_color.svg",
            Self::FedoraColor => "icons/fedora_color.svg",
            Self::ArchColor => "icons/arch_color.svg",
            Self::AlpineColor => "icons/alpine_color.svg",
            Self::AmazonColor => "icons/amazon_color.svg",
            Self::KaliColor => "icons/kali_color.svg",
            Self::AlinuxColor => "icons/alinux_color.svg",
            Self::OpenEulerColor => "icons/openeuler_color.svg",
            Self::FreebsdColor => "icons/freebsd_color.svg",
            Self::CiscoColor => "icons/cisco_color.svg",
            Self::JuniperColor => "icons/juniper_color.svg",
            Self::HuaweiColor => "icons/huawei_color.svg",
            Self::H3cColor => "icons/h3c_color.svg",
            Self::HpeColor => "icons/hpe_color.svg",
            Self::MikrotikColor => "icons/mikrotik_color.svg",
            Self::FortinetColor => "icons/fortinet_color.svg",
            Self::PaloaltoColor => "icons/paloalto_color.svg",
            Self::ZyxelColor => "icons/zyxel_color.svg",
            Self::TerminalHistoryColor => "icons/terminal_history_color.svg",
            Self::TerminalBroadcastColor => "icons/terminal_broadcast_color.svg",
            Self::RichInputColor => "icons/rich_input_color.svg",
            Self::Apps => "icons/apps.svg",
            Self::AppsColor => "icons/apps_color.svg",
            Self::MongoDB => "icons/mongodb.svg",
            Self::Mqtt => "icons/mqtt.svg",
            Self::MySQLColor => "icons/mysql_color.svg",
            Self::SQLiteColor => "icons/sqlite_color.svg",
            Self::PostgreSQLColor => "icons/postgresql_color.svg",
            Self::PostgreSQLLineColor => "icons/postgresql_line_color.svg",
            Self::MSSQLColor => "icons/mssql_color.svg",
            Self::MySQLLineColor => "icons/mysql_line_color.svg",
            Self::SQLiteLineColor => "icons/sqlite_line_color.svg",
            Self::OracleColor => "icons/oracle_color.svg",
            Self::Workspace => "icons/workspace.svg",
            Self::RedisColor => "icons/redis_color.svg",
            Self::All => "icons/all.svg",
            Self::Edit => "icons/edit.svg",
            Self::Filter => "icons/filter.svg",
            Self::Refresh => "icons/refresh.svg",
            Self::Sync => "icons/sync.svg",
            Self::Upload => "icons/upload.svg",
            Self::NewFolder => "icons/new_folder.svg",
            Self::EditBorder => "icons/edit_border.svg",
            Self::MSSQLLineColor => "icons/mssql_line_color.svg",
            Self::OracleLineColor => "icons/oracle_line_color.svg",
            Self::ClickHouseColor => "icons/clickhouse_color.svg",
            Self::ClickHouseLineColor => "icons/clickhouse_line_color.svg",
            Self::TDengineColor => "icons/tdengine_color.svg",
            Self::Remove => "icons/remove.svg",
            Self::TableData => "icons/table-data.svg",
            Self::TableDesign => "icons/table-design.svg",
            Self::TableDesignTool => "icons/table-design-tool.svg",
            Self::SchemaCompare => "icons/schema-compare.svg",
            Self::DataModel => "icons/data-model.svg",
            Self::Server => "icons/server.svg",
            Self::Export => "icons/export.svg",
            Self::AI => "icons/ai.svg",
            Self::Home => "icons/home.svg",
            Self::SettingColor => "icons/setting_color.svg",
            Self::SerialPort => "icons/serial_port.svg",
            Self::Monitor => "icons/monitor.svg",
            Self::TerminalServerMonitorColor => "icons/terminal_server_monitor_color.svg",
            Self::PortForwardingColor => "icons/port_forwarding_color.svg",
            Self::Rdp => "icons/rdp.svg",
            Self::Vnc => "icons/vnc.svg",
            Self::DuckDB => "icons/duckdb.svg",
            Self::ServerLine => "icons/server_line.svg",
            Self::TerminalLine => "icons/terminal_line.svg",
            Self::DatabaseLine => "icons/database_line.svg",
            Self::RedisLine => "icons/redis_line.svg",
            Self::MongoDBLine => "icons/mongodb_line.svg",
            Self::MqttLine => "icons/mqtt_line.svg",
            Self::TDengineLineColor => "icons/tdengine_line_color.svg",
            Self::SerialLine => "icons/serial_line.svg",
            Self::PortForwardingLine => "icons/port_forwarding_line.svg",
            Self::RdpLine => "icons/rdp_line.svg",
            Self::VncLine => "icons/vnc_line.svg",
            Self::AILine => "icons/ai_line.svg",
            Self::TeamLine => "icons/team_line.svg",
            Self::NotesLine => "icons/notes_line.svg",
            Self::ExtensionsLine => "icons/extensions_line.svg",
        }
        .into()
    }
}

impl From<IconName> for AnyElement {
    fn from(val: IconName) -> Self {
        Icon::build(val).into_any_element()
    }
}

impl RenderOnce for IconName {
    fn render(self, _: &mut Window, _cx: &mut App) -> impl IntoElement {
        Icon::build(self)
    }
}

#[derive(IntoElement)]
pub struct Icon {
    base: Svg,
    style: StyleRefinement,
    path: SharedString,
    image_source: Option<ImageSource>,
    text_color: Option<Hsla>,
    size: Option<Size>,
    color_mode: IconColorMode,
    rotation: Option<Radians>,
}

impl Default for Icon {
    fn default() -> Self {
        Self {
            base: svg().flex_none().size_4(),
            style: StyleRefinement::default(),
            path: "".into(),
            image_source: None,
            text_color: None,
            size: None,
            color_mode: IconColorMode::default(),
            rotation: None,
        }
    }
}

impl Clone for Icon {
    fn clone(&self) -> Self {
        let mut this = Self::default().path(self.path.clone());
        this.style = self.style.clone();
        this.rotation = self.rotation;
        this.size = self.size;
        this.text_color = self.text_color;
        this.color_mode = self.color_mode;
        this.image_source = self.image_source.clone();
        this
    }
}

impl Icon {
    pub fn new(icon: impl Into<Icon>) -> Self {
        icon.into()
    }

    fn build(name: impl IconNamed) -> Self {
        Self::default().path(name.path())
    }

    /// Set the icon path of the Assets bundle
    ///
    /// For example: `icons/foo.svg`
    pub fn path(mut self, path: impl Into<SharedString>) -> Self {
        self.path = path.into();
        self.image_source = None;
        self
    }

    /// Set the icon source to a filesystem path.
    ///
    /// This is used for external assets that are not embedded in the application asset bundle.
    pub fn file_path(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        self.path = path.display().to_string().into();
        self.image_source = Some(path.into());
        self
    }

    /// Create a new view for the icon
    pub fn view(self, cx: &mut App) -> Entity<Icon> {
        cx.new(|_| self)
    }

    pub fn transform(mut self, transformation: Transformation) -> Self {
        self.base = self.base.with_transformation(transformation);
        self
    }

    pub fn empty() -> Self {
        Self::default()
    }

    /// Set the icon color mode.
    pub fn color_mode(mut self, mode: IconColorMode) -> Self {
        self.color_mode = mode;
        self
    }

    /// Set the icon to color mode.
    pub fn color(mut self) -> Self {
        self.color_mode = IconColorMode::Color;
        self
    }

    /// Set the icon to mono mode.
    pub fn mono(mut self) -> Self {
        self.color_mode = IconColorMode::Mono;
        self
    }

    /// Rotate the icon by the given angle
    pub fn rotate(mut self, radians: impl Into<Radians>) -> Self {
        self.base = self
            .base
            .with_transformation(Transformation::rotate(radians));
        self
    }
}

impl Styled for Icon {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }

    fn text_color(mut self, color: impl IntoColor<Hsla>) -> Self {
        self.text_color = Some(color.into_color());
        self
    }
}

impl Sizable for Icon {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = Some(size.into());
        self
    }
}

impl RenderOnce for Icon {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let has_base_size = self.style.size.width.is_some() || self.style.size.height.is_some();
        let apply_resolved_size = should_apply_resolved_size(self.size, has_base_size);
        let resolved_size = resolve_icon_size(self.size);

        match self.color_mode {
            IconColorMode::Mono => {
                let text_color = self.text_color.unwrap_or_else(|| window.text_style().color);
                let mut base = self.base;
                *base.style() = self.style;

                base.flex_shrink_0()
                    .text_color(text_color)
                    .when(apply_resolved_size, |this| this.size(resolved_size))
                    .path(self.path)
                    .into_any_element()
            }
            IconColorMode::Color => {
                let mut base = div();
                *base.style() = self.style;

                base.flex_shrink_0()
                    .when(apply_resolved_size, |this| this.size(resolved_size))
                    .child(
                        img(self
                            .image_source
                            .unwrap_or_else(|| self.path.clone().into()))
                        .size_full(),
                    )
                    .into_any_element()
            }
        }
    }
}

impl From<Icon> for AnyElement {
    fn from(val: Icon) -> Self {
        val.into_any_element()
    }
}

impl Render for Icon {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_base_size = self.style.size.width.is_some() || self.style.size.height.is_some();
        let apply_resolved_size = should_apply_resolved_size(self.size, has_base_size);
        let resolved_size = resolve_icon_size(self.size);

        match self.color_mode {
            IconColorMode::Mono => {
                let text_color = self.text_color.unwrap_or_else(|| cx.theme().foreground);
                let mut base = svg().flex_none();
                *base.style() = self.style.clone();

                base.flex_shrink_0()
                    .text_color(text_color)
                    .when(apply_resolved_size, |this| this.size(resolved_size))
                    .path(self.path.clone())
                    .when_some(self.rotation, |this, rotation| {
                        this.with_transformation(Transformation::rotate(rotation))
                    })
                    .into_any_element()
            }
            IconColorMode::Color => {
                let mut base = div();
                *base.style() = self.style.clone();

                base.flex_shrink_0()
                    .when(apply_resolved_size, |this| this.size(resolved_size))
                    .child(
                        img(self
                            .image_source
                            .clone()
                            .unwrap_or_else(|| self.path.clone().into()))
                        .size_full(),
                    )
                    .into_any_element()
            }
        }
    }
}
