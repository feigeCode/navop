#[cfg(debug_assertions)]
use std::path::{Path, PathBuf};

/// 在应用读取配置之前加载本地环境文件。
///
/// 已由启动进程显式设置的环境变量不会被覆盖；文件优先级为
/// `.env.local` 高于 `.env`，目录优先级为当前工作目录、开发工作区。
/// Release 构建不会读取任何环境文件。
#[cfg(debug_assertions)]
pub fn load_env_files() {
    let mut directories = Vec::new();

    if let Ok(current_dir) = std::env::current_dir() {
        push_unique(&mut directories, current_dir);
    }

    if let Some(workspace_dir) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() {
        push_unique(&mut directories, workspace_dir.to_path_buf());
    }

    for path in env_file_candidates(&directories) {
        if path.is_file()
            && let Err(error) = dotenvy::from_path(&path)
        {
            eprintln!("无法加载环境文件 {}: {error}", path.display());
        }
    }
}

#[cfg(not(debug_assertions))]
pub fn load_env_files() {}

#[cfg(debug_assertions)]
fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

#[cfg(debug_assertions)]
fn env_file_candidates(directories: &[PathBuf]) -> Vec<PathBuf> {
    [".env.local", ".env"]
        .into_iter()
        .flat_map(|file_name| {
            directories
                .iter()
                .map(move |directory| directory.join(file_name))
        })
        .collect()
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    #[test]
    fn local_environment_files_have_priority_over_fallback_files() {
        let first = PathBuf::from("/first");
        let second = PathBuf::from("/second");

        assert_eq!(
            env_file_candidates(&[first.clone(), second.clone()]),
            vec![
                first.join(".env.local"),
                second.join(".env.local"),
                first.join(".env"),
                second.join(".env"),
            ]
        );
    }

    #[test]
    fn duplicate_search_directories_are_ignored() {
        let mut directories = Vec::new();
        push_unique(&mut directories, PathBuf::from("/workspace"));
        push_unique(&mut directories, PathBuf::from("/workspace"));

        assert_eq!(directories, vec![PathBuf::from("/workspace")]);
    }
}
