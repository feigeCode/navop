pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[must_use]
pub fn should_hide_background_child_console() -> bool {
    cfg!(windows)
}

#[must_use]
pub fn background_child_creation_flags(existing_flags: u32) -> u32 {
    existing_flags | CREATE_NO_WINDOW
}

pub fn configure_background_child(
    command: &mut std::process::Command,
) -> &mut std::process::Command {
    configure_std_command(command);
    command
}

pub fn configure_tokio_background_child(
    command: &mut tokio::process::Command,
) -> &mut tokio::process::Command {
    configure_tokio_command(command);
    command
}

#[cfg(windows)]
fn configure_std_command(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;

    command.creation_flags(background_child_creation_flags(0));
}

#[cfg(not(windows))]
fn configure_std_command(_command: &mut std::process::Command) {}

#[cfg(windows)]
fn configure_tokio_command(command: &mut tokio::process::Command) {
    command.creation_flags(background_child_creation_flags(0));
}

#[cfg(not(windows))]
fn configure_tokio_command(_command: &mut tokio::process::Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_child_flags_include_create_no_window() {
        assert_eq!(CREATE_NO_WINDOW, background_child_creation_flags(0));
        assert_eq!(
            CREATE_NO_WINDOW | 0x0000_0200,
            background_child_creation_flags(0x0000_0200)
        );
    }

    #[test]
    fn background_child_console_hiding_is_windows_only() {
        assert_eq!(cfg!(windows), should_hide_background_child_console());
    }
}
