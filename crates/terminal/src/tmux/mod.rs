mod command;
mod control;
mod discovery;
mod protocol;
mod state;
mod transport;
mod version;

pub use command::{
    TmuxSplitPlacement, quote_shell_arg, send_enter_command, send_keys_command,
    split_window_command,
};
pub use control::{
    TmuxControlError, TmuxControlEvent, TmuxControlTransport, TmuxControllerHandle,
    spawn_tmux_control,
};
pub use discovery::{DiscoveredTmux, TmuxTarget, discover_tmux_client, open_control_transport};
pub use protocol::{TmuxControlLine, parse_control_line, unescape_output};
pub use state::{
    LIST_PANES_COMMAND_FORMAT, LIST_PANES_FORMAT, LIST_WINDOWS_FORMAT, TmuxPane, TmuxSessionState,
    TmuxWindow, parse_list_panes, parse_list_panes_with_command_state, parse_list_windows,
};
pub use version::TmuxVersion;
