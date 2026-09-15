//! 文件树后端策略:文件系统操作从「本机 std::fs」抽象出来。
//!
//! `WorkspaceExplorer` 只依赖 [`WorkspaceBackend`],因此同一套树 UI 可以驱动
//! 两种后端:
//!
//! - [`LocalBackend`]:本机文件系统(含 WSL 的 UNC 根,在 Windows 上就是本机路径)。
//! - [`ContainerBackend`]:容器文件系统,经 `docker exec` 执行 `ls/stat/cat/...`。
//!
//! 后端方法都是同步阻塞调用,执行方(`WorkspaceExplorer` / `WorkspaceEditor`)
//! 已经在 `background_spawn` 任务里调用它们,不占用 UI 线程。

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use ignore::gitignore::Gitignore;
use remote_file_editor::{decode_text_content, determine_file_policy, load_language_for_path};

use crate::file_system::{self, LoadedFile};
use crate::model::{ExplorerEntry, sort_entries};

/// 文件树依赖的文件系统能力。
///
/// `supports_git` 为 false 的后端(容器)不参与仓库发现、gitignore 与变更视图。
pub trait WorkspaceBackend: Send + Sync {
    fn canonical_root(&self, root: PathBuf) -> Result<PathBuf>;
    fn read_directory(
        &self,
        path: &Path,
        ignore: Option<&Gitignore>,
        show_hidden: bool,
        show_ignored: bool,
    ) -> Result<Vec<ExplorerEntry>>;
    fn root_ignore_matcher(&self, root: &Path) -> Option<Arc<Gitignore>>;
    fn load_file(&self, path: &Path) -> Result<LoadedFile>;
    fn save_file(&self, path: &Path, text: &str) -> Result<()>;
    fn create_file(&self, parent: &Path, name: &str) -> Result<PathBuf>;
    fn create_directory(&self, parent: &Path, name: &str) -> Result<PathBuf>;
    fn rename_entry(&self, path: &Path, new_name: &str) -> Result<PathBuf>;
    fn delete_entry(&self, path: &Path) -> Result<()>;
    fn copy_entry(&self, source: &Path, destination_dir: &Path) -> Result<PathBuf>;
    fn move_entry(&self, source: &Path, destination_dir: &Path) -> Result<PathBuf>;
    fn supports_git(&self) -> bool;
}

/// 本机文件系统后端。
pub struct LocalBackend;

impl WorkspaceBackend for LocalBackend {
    fn canonical_root(&self, root: PathBuf) -> Result<PathBuf> {
        file_system::canonical_workspace_root(root)
    }

    fn read_directory(
        &self,
        path: &Path,
        ignore: Option<&Gitignore>,
        show_hidden: bool,
        show_ignored: bool,
    ) -> Result<Vec<ExplorerEntry>> {
        file_system::read_directory(path, ignore, show_hidden, show_ignored)
    }

    fn root_ignore_matcher(&self, root: &Path) -> Option<Arc<Gitignore>> {
        file_system::root_ignore_matcher(root)
    }

    fn load_file(&self, path: &Path) -> Result<LoadedFile> {
        file_system::load_file(path)
    }

    fn save_file(&self, path: &Path, text: &str) -> Result<()> {
        file_system::save_file(path, text)
    }

    fn create_file(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        file_system::create_file(parent, name)
    }

    fn create_directory(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        file_system::create_directory(parent, name)
    }

    fn rename_entry(&self, path: &Path, new_name: &str) -> Result<PathBuf> {
        file_system::rename_entry(path, new_name)
    }

    fn delete_entry(&self, path: &Path) -> Result<()> {
        file_system::delete_entry(path)
    }

    fn copy_entry(&self, source: &Path, destination_dir: &Path) -> Result<PathBuf> {
        file_system::copy_entry(source, destination_dir)
    }

    fn move_entry(&self, source: &Path, destination_dir: &Path) -> Result<PathBuf> {
        file_system::move_entry(source, destination_dir)
    }

    fn supports_git(&self) -> bool {
        true
    }
}

/// 容器文件系统后端:通过 `docker exec` 在容器内执行文件操作。
///
/// 路径都是容器内绝对路径,只交给 `docker exec`,不经过本机 `std::fs`。
pub struct ContainerBackend {
    program: String,
    global_args: Vec<String>,
    container: String,
}

impl ContainerBackend {
    pub fn new(
        program: impl Into<String>,
        global_args: Vec<String>,
        container: impl Into<String>,
    ) -> Self {
        Self {
            program: program.into(),
            global_args,
            container: container.into(),
        }
    }

    /// 运行 `docker <global...> exec -i <容器> <命令...>`,可选写入 stdin。
    fn run(&self, command: &[String], stdin: Option<&[u8]>) -> Result<std::process::Output> {
        let mut child = Command::new(&self.program)
            .args(&self.global_args)
            .arg("exec")
            .arg("-i")
            .arg(&self.container)
            .args(command)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("Unable to run {}", self.program))?;
        if let Some(bytes) = stdin {
            if let Some(mut handle) = child.stdin.take() {
                handle
                    .write_all(bytes)
                    .context("Unable to write to container stdin")?;
            }
        }
        let output = child
            .wait_with_output()
            .with_context(|| format!("Unable to query container {}", self.container))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "docker exec failed in {}: {}",
                self.container,
                stderr.trim()
            );
        }
        Ok(output)
    }

    /// 在容器内以 `sh -c` 运行脚本,额外参数经 `$1` 起可用。
    fn sh(
        &self,
        script: &str,
        args: &[String],
        stdin: Option<&[u8]>,
    ) -> Result<std::process::Output> {
        let mut command = vec![
            "sh".to_string(),
            "-c".to_string(),
            script.to_string(),
            "sh".to_string(),
        ];
        command.extend(args.iter().cloned());
        self.run(&command, stdin)
    }

    fn path_text(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }
}

impl WorkspaceBackend for ContainerBackend {
    fn canonical_root(&self, root: PathBuf) -> Result<PathBuf> {
        // 容器路径已是绝对路径;本机 canonicalize 不适用。
        Ok(root)
    }

    fn read_directory(
        &self,
        path: &Path,
        _ignore: Option<&Gitignore>,
        show_hidden: bool,
        _show_ignored: bool,
    ) -> Result<Vec<ExplorerEntry>> {
        // `-p` 给目录名追加 `/`,据此判定类型且不受文件名空格影响。
        let output = self.sh("ls -1Ap -- \"$1\"", &[Self::path_text(path)], None)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(parse_ls_entries(path, &stdout, show_hidden))
    }

    fn root_ignore_matcher(&self, _root: &Path) -> Option<Arc<Gitignore>> {
        None
    }

    fn load_file(&self, path: &Path) -> Result<LoadedFile> {
        let path_str = Self::path_text(path);
        let size_output = self.run(
            &[
                "stat".to_string(),
                "-c".to_string(),
                "%s".to_string(),
                "--".to_string(),
                path_str.clone(),
            ],
            None,
        )?;
        let size: usize = String::from_utf8_lossy(&size_output.stdout)
            .trim()
            .parse()
            .with_context(|| format!("Unable to read size of {path_str}"))?;
        let policy = determine_file_policy(size)?;
        let content = self.run(
            &["cat".to_string(), "--".to_string(), path_str.clone()],
            None,
        )?;
        let bytes = content.stdout;
        let text = decode_text_content(&bytes)?;
        let language = load_language_for_path(&path_str, policy.is_large_file)?;
        Ok(LoadedFile {
            text,
            policy,
            file_size: size,
            language,
        })
    }

    fn save_file(&self, path: &Path, text: &str) -> Result<()> {
        self.sh(
            "cat > \"$1\"",
            &[Self::path_text(path)],
            Some(text.as_bytes()),
        )?;
        Ok(())
    }

    fn create_file(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        let target = file_system::validated_child_path(parent, name)?;
        // `set -C` = noclobber:目标已存在时失败,与本地 create_new 语义一致。
        self.sh("set -C; : > \"$1\"", &[Self::path_text(&target)], None)?;
        Ok(target)
    }

    fn create_directory(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        let target = file_system::validated_child_path(parent, name)?;
        self.sh("mkdir -- \"$1\"", &[Self::path_text(&target)], None)?;
        Ok(target)
    }

    fn rename_entry(&self, path: &Path, new_name: &str) -> Result<PathBuf> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("Cannot rename workspace root"))?;
        let target = file_system::validated_child_path(parent, new_name)?;
        if target == path {
            return Ok(path.to_path_buf());
        }
        self.sh(
            "if [ -e \"$2\" ]; then echo 'target exists' >&2; exit 1; fi; mv -- \"$1\" \"$2\"",
            &[Self::path_text(path), Self::path_text(&target)],
            None,
        )?;
        Ok(target)
    }

    fn delete_entry(&self, path: &Path) -> Result<()> {
        self.sh("rm -rf -- \"$1\"", &[Self::path_text(path)], None)?;
        Ok(())
    }

    fn copy_entry(&self, source: &Path, destination_dir: &Path) -> Result<PathBuf> {
        let name = source
            .file_name()
            .ok_or_else(|| anyhow!("Cannot transfer workspace root"))?;
        let destination = destination_dir.join(name);
        self.sh(
            "if [ -e \"$2\" ]; then echo 'target exists' >&2; exit 1; fi; cp -r -- \"$1\" \"$2\"",
            &[Self::path_text(source), Self::path_text(&destination)],
            None,
        )?;
        Ok(destination)
    }

    fn move_entry(&self, source: &Path, destination_dir: &Path) -> Result<PathBuf> {
        let name = source
            .file_name()
            .ok_or_else(|| anyhow!("Cannot transfer workspace root"))?;
        let destination = destination_dir.join(name);
        self.sh(
            "if [ -e \"$2\" ]; then echo 'target exists' >&2; exit 1; fi; mv -- \"$1\" \"$2\"",
            &[Self::path_text(source), Self::path_text(&destination)],
            None,
        )?;
        Ok(destination)
    }

    fn supports_git(&self) -> bool {
        false
    }
}

/// 把 `ls -1Ap` 的输出解析成条目。
///
/// `-p` 让目录名带 `/` 后缀,因此名称里的空格不受影响;`.git` 与(未开启
/// `show_hidden` 时的)点文件被过滤,与本机后端保持一致。
fn parse_ls_entries(path: &Path, output: &str, show_hidden: bool) -> Vec<ExplorerEntry> {
    let mut entries = Vec::new();
    for line in output.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let (name, is_dir) = match line.strip_suffix('/') {
            Some(name) => (name, true),
            None => (line, false),
        };
        if name.is_empty() || name == ".git" {
            continue;
        }
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        entries.push(ExplorerEntry {
            path: path.join(name),
            name: name.to_string(),
            is_dir,
        });
    }
    sort_entries(&mut entries);
    entries
}

/// 本机后端(默认)。
pub fn local_backend() -> Arc<dyn WorkspaceBackend> {
    Arc::new(LocalBackend)
}

/// 容器后端。
pub fn container_backend(
    program: impl Into<String>,
    global_args: Vec<String>,
    container: impl Into<String>,
) -> Arc<dyn WorkspaceBackend> {
    Arc::new(ContainerBackend::new(program, global_args, container))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ls_entries_with_directories_and_plain_names() {
        let output = "bin/\netc/\nREADME.md\n";
        let entries = parse_ls_entries(Path::new("/root"), output, false);
        let names = entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.is_dir))
            .collect::<Vec<_>>();
        // 目录排在文件前,且带 is_dir 标记
        assert_eq!(
            vec![("bin", true), ("etc", true), ("README.md", false)],
            names
        );
        assert_eq!(Path::new("/root/bin"), entries[0].path);
    }

    #[test]
    fn keeps_names_with_spaces_and_filters_hidden_entries() {
        let output = "my file.txt\n.hidden\n.git\nvisible\n";
        let entries = parse_ls_entries(Path::new("/app"), output, false);
        assert_eq!(
            vec!["my file.txt", "visible"],
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>()
        );

        let with_hidden = parse_ls_entries(Path::new("/app"), output, true);
        assert_eq!(
            vec![".hidden", "my file.txt", "visible"],
            with_hidden
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn local_backend_has_git_container_does_not() {
        assert!(LocalBackend.supports_git());
        let container =
            ContainerBackend::new("docker", vec!["--context".into(), "prod".into()], "web");
        assert!(!container.supports_git());
        // 容器路径不做本机 canonicalize。
        assert_eq!(
            PathBuf::from("/etc"),
            container.canonical_root(PathBuf::from("/etc")).unwrap()
        );
    }
}
