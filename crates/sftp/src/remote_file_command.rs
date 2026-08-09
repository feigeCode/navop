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

    items
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
        .map(|commands| commands.join(" && "))
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
    use crate::ServerCopyItem;

    fn item(source: &str, target: &str, is_dir: bool) -> ServerCopyItem {
        ServerCopyItem {
            source_path: source.to_string(),
            target_path: target.to_string(),
            is_dir,
            size: 0,
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
            "cp -- '/src/a.txt' '/dst/a.txt' && cp -R -- '/src/folder' '/dst/folder'"
        );
    }

    #[test]
    fn builds_move_commands() {
        let command = build_remote_file_command(
            RemoteFileOperation::Move,
            &[item("/src/a.txt", "/dst/a.txt", false)],
        )
        .expect("move command");

        assert_eq!(command, "mv -- '/src/a.txt' '/dst/a.txt'");
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
            "cp -- '/src/it'\"'\"'s ready' '/dst/-still it'\"'\"'s ready'"
        );
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
