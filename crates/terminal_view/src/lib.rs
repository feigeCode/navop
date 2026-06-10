rust_i18n::i18n!("locales", fallback = "en");

pub mod addon;
pub mod cd_completion;
pub mod highlight_presets;
pub mod history_prompt;
pub mod keys;
pub mod serial_form_window;
pub mod settings;
pub mod sidebar;
mod ssh_form_mfa;
pub mod ssh_form_window;
pub mod terminal_element;
pub mod theme;
pub mod view;

pub use addon::{AddonManager, HoveredLink, SearchAddon, TerminalAddon, WebLinksAddon};
pub use one_core::layout::{
    SIDEBAR_DEFAULT_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH, TOOLBAR_WIDTH,
};
pub use serial_form_window::{SerialFormWindow, SerialFormWindowConfig};
pub use settings::{
    TerminalHighlightRule, TerminalSettings, current_settings, init_settings, update_settings,
};
pub use sidebar::{SettingsPanel, SidebarPanel, TerminalSidebar, TerminalSidebarEvent};
pub use ssh_form_window::{SshFormWindow, SshFormWindowConfig};
pub use terminal::terminal::{
    ConnectionState, SshTerminalConfig, Terminal, TerminalConnectionKind, TerminalModelEvent,
};
pub use theme::{
    DEFAULT_LINE_HEIGHT_SCALE, MAX_FONT_SIZE, MIN_FONT_SIZE, TerminalTheme, default_font_fallbacks,
};
pub use view::{TerminalView, init, refresh_keybindings};
