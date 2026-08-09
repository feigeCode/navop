use crate::ServerCopyItem;
use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteFileOperation {
    Copy,
    Move,
}

pub fn build_remote_file_command(
    operation: RemoteFileOperation,
    items: &[ServerCopyItem],
) -> Result<String> {
    if items.is_empty() {
        bail!("remote file command requires at least one item");
    }

    let commands = items
        .iter()
        .map(|item| {
            let source = shell_quote_path(&item.source_path)?;
            let target = shell_quote_path(&item.target_path)?;
            Ok(match operation {
                RemoteFileOperation::Copy if item.is_dir => {
                    format!("cp -R -- {source} {target}")
                }
                RemoteFileOperation::Copy => format!("cp -- {source} {target}"),
                RemoteFileOperation::Move => format!("mv -- {source} {target}"),
            })
        })
        .collect::<Result<Vec<_>>>()
        .map(|commands| commands.join(" && "))?;
    let required_command = match operation {
        RemoteFileOperation::Copy => "cp",
        RemoteFileOperation::Move => "mv",
    };
    Ok(format!(
        "{} {commands}",
        required_command_guard(required_command)
    ))
}

fn required_command_guard(command: &str) -> String {
    format!(
        "command -v {command} >/dev/null 2>&1 || {{ printf '%s\\n' \
'required remote command not found: {command}' >&2; exit 127; }};"
    )
}

fn shell_quote_path(path: &str) -> Result<String> {
    if path.contains('\0') {
        bail!("remote path contains a NUL byte");
    }
    Ok(format!("'{}'", path.replace('\'', "'\"'\"'")))
}

#[cfg(test)]
mod tests {
    use super::{RemoteFileOperation, build_remote_file_command};
    use crate::{DirectoryConflictPolicy, ServerCopyItem};

    fn item(source: &str, target: &str, is_dir: bool) -> ServerCopyItem {
        ServerCopyItem {
            source_path: source.to_string(),
            target_path: target.to_string(),
            is_dir,
            size: 0,
            directory_conflict_policy: DirectoryConflictPolicy::Merge,
        }
    }

    #[test]
    fn builds_copy_commands_for_files_and_directories() {
        let command = build_remote_file_command(
            RemoteFileOperation::Copy,
            &[
                item("/src/a.txt", "/dst/a.txt", false),
                item("/src/folder", "/dst/folder", true),
            ],
        )
        .expect("copy command");

        assert_eq!(
            command,
            "command -v cp >/dev/null 2>&1 || { printf '%s\\n' \
'required remote command not found: cp' >&2; exit 127; }; \
cp -- '/src/a.txt' '/dst/a.txt' && cp -R -- '/src/folder' '/dst/folder'"
        );
    }

    #[test]
    fn builds_move_commands() {
        let command = build_remote_file_command(
            RemoteFileOperation::Move,
            &[item("/src/a.txt", "/dst/a.txt", false)],
        )
        .expect("move command");

        assert_eq!(
            command,
            "command -v mv >/dev/null 2>&1 || { printf '%s\\n' \
'required remote command not found: mv' >&2; exit 127; }; \
mv -- '/src/a.txt' '/dst/a.txt'"
        );
    }

    #[test]
    fn safely_quotes_shell_paths() {
        let command = build_remote_file_command(
            RemoteFileOperation::Copy,
            &[item("/src/it's ready", "/dst/-still it's ready", false)],
        )
        .expect("quoted command");

        assert_eq!(
            command,
            "command -v cp >/dev/null 2>&1 || { printf '%s\\n' \
'required remote command not found: cp' >&2; exit 127; }; \
cp -- '/src/it'\"'\"'s ready' '/dst/-still it'\"'\"'s ready'"
        );
    }

    #[test]
    fn copy_command_checks_cp_before_execution() {
        let command = build_remote_file_command(
            RemoteFileOperation::Copy,
            &[item("/src/a.txt", "/dst/a.txt", false)],
        )
        .expect("copy command");

        assert!(command.starts_with(
            "command -v cp >/dev/null 2>&1 || { printf '%s\\n' \
'required remote command not found: cp' >&2; exit 127; }; "
        ));
    }

    #[test]
    fn move_command_checks_mv_before_execution() {
        let command = build_remote_file_command(
            RemoteFileOperation::Move,
            &[item("/src/a.txt", "/dst/a.txt", false)],
        )
        .expect("move command");

        assert!(command.starts_with(
            "command -v mv >/dev/null 2>&1 || { printf '%s\\n' \
'required remote command not found: mv' >&2; exit 127; }; "
        ));
    }

    #[test]
    fn command_availability_is_checked_once_for_multiple_items() {
        let command = build_remote_file_command(
            RemoteFileOperation::Copy,
            &[
                item("/src/a.txt", "/dst/a.txt", false),
                item("/src/b.txt", "/dst/b.txt", false),
            ],
        )
        .expect("copy command");

        assert_eq!(1, command.matches("command -v cp").count());
    }

    #[test]
    fn rejects_empty_items_and_nul_bytes() {
        assert!(build_remote_file_command(RemoteFileOperation::Copy, &[]).is_err());
        assert!(
            build_remote_file_command(
                RemoteFileOperation::Move,
                &[item("/src/\0bad", "/dst/good", false)],
            )
            .is_err()
        );
    }
}
