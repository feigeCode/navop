//! 在 GUI 进程里解析并启动外部命令行程序。
//!
//! macOS 上从 Finder / Dock 启动的应用只继承 launchd 的最小 PATH
//! （`/usr/bin:/bin:/usr/sbin:/sbin`），Homebrew、nvm、fnm 等目录都不在其中；
//! 直接 `Command::new("npx")` 只会得到 `No such file or directory (os error 2)`。
//! 这里先按进程 PATH 解析，再用 GUI 进程通常拿不到的工具链目录兜底，并把解析
//! 到的目录前置到子进程 PATH——`npx` 的 shebang 是 `#!/usr/bin/env node`，只解析
//! 出绝对路径而不补 PATH 仍会以 `env: node: No such file or directory` 失败。

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

/// macOS / Linux 上 GUI 进程通常看不到、但用户工具链常用的目录。
#[cfg(not(windows))]
const SYSTEM_PROGRAM_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
    "/usr/local/bin",
    "/usr/local/sbin",
    "/opt/local/bin",
    "/opt/local/sbin",
    "/snap/bin",
];

/// macOS / Linux 上任何进程都该有的基础目录：GUI 进程的 PATH 被裁剪到极短，
/// 甚至可能只剩工具链目录，`sh` / `env` 这类基础命令不能因此丢失。
#[cfg(not(windows))]
const BASE_SYSTEM_DIRS: &[&str] = &["/usr/bin", "/bin", "/usr/sbin", "/sbin"];

/// 用户目录下与版本管理器相关的 bin 目录（不含 nvm / fnm 的版本目录）。
#[cfg(not(windows))]
const HOME_PROGRAM_DIRS: &[&str] = &[
    ".local/bin",
    ".volta/bin",
    ".asdf/shims",
    ".bun/bin",
    "Library/pnpm",
    ".local/share/pnpm",
];

/// 自带安装脚本、把可执行文件放进 `$HOME/.<vendor>/bin` 的 CLI 目录。
///
/// 这类 CLI 既不进 Homebrew 也不进 nvm，只靠安装脚本往用户目录写一份可执行文件：
/// Kimi CLI 的安装脚本装出 `~/.kimi-code/bin/kimi`。GUI 进程被 launchd 裁剪成
/// `/usr/bin:/bin:/usr/sbin:/sbin` 的 PATH 里没有它，只靠 shell 的 PATH 兜底就会
/// 在 ACP 设置页报「找不到命令 `kimi`」。
#[cfg(not(windows))]
const VENDOR_PROGRAM_DIRS: &[&str] = &[".kimi-code/bin"];

/// 按 PATH 与 GUI 不可见目录解析外部命令。
///
/// `name` 可以是裸命令名（`npx`），也可以是绝对/相对路径（此时只校验该文件）。
#[must_use]
pub fn resolve_program(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH");
    resolve_program_in(name, path.as_deref(), &fallback_program_dirs())
}

/// 构造一个在 GUI 进程里尽量能跑起来的外部命令。
///
/// 找不到可执行文件时返回 `None`，由调用方给出可诊断的错误，而不是静默失败。
pub fn build_command<S: AsRef<OsStr>>(name: &str, args: &[S]) -> Option<Command> {
    let program = resolve_program(name)?;
    let mut command = program_command(&program);
    command.args(args);
    command.env("PATH", child_path_with_program_dir(&program));
    Some(command)
}

/// 把程序所在目录前置进子进程 PATH，其余部分保持当前进程的 PATH 顺序。
///
/// 除了当前进程的 PATH，还会补上 GUI 进程看不到的工具链目录（Homebrew、
/// `/usr/local/bin`、nvm / fnm / volta 的版本目录等）。原因不只是「找到自己」：
/// 拉起的外部命令常常再 `spawn` 一个兄弟 CLI——例如 ACP 的 `pi-acp` 适配器会去
/// `PATH` 里找 `pi`，而 `pi` 常常装在本机 nvm 的版本目录下。只把 npm 所在目录塞进
/// 子进程 PATH 时，适配器本身起得来，真正干活的 CLI 却报
/// `Could not start pi: executable not found (command: pi)`。
#[must_use]
pub fn child_path_with_program_dir(program: &Path) -> OsString {
    let path = std::env::var_os("PATH");
    child_path_in(program, path.as_deref(), &fallback_program_dirs())
}

/// 以调用方给的 PATH 为底，做和 [`child_path_with_program_dir`] 一样的补全。
///
/// 用于「配置里已经写了 PATH」的场景：用户显式配置的目录要排在兜底目录之前，
/// 不能因为补全逻辑把整条 PATH 换掉。
#[must_use]
pub fn child_path_with_program_dir_and_path(program: &Path, path: Option<&OsStr>) -> OsString {
    child_path_in(program, path, &fallback_program_dirs())
}

/// `child_path_with_program_dir` 的显式版本：顺序 = 程序目录 → 当前 PATH →
/// 兜底工具链目录 → 基础系统目录，并去重。参数显式传入，便于不依赖宿主环境测试。
fn child_path_in(program: &Path, path: Option<&OsStr>, fallback_dirs: &[PathBuf]) -> OsString {
    let mut parts: Vec<PathBuf> = Vec::new();
    if let Some(dir) = program.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        parts.push(dir.to_path_buf());
    }
    if let Some(path) = path {
        parts.extend(std::env::split_paths(path).filter(|part| !part.as_os_str().is_empty()));
    }
    parts.extend(fallback_dirs.iter().cloned());
    #[cfg(not(windows))]
    parts.extend(BASE_SYSTEM_DIRS.iter().map(PathBuf::from));
    let mut unique: Vec<PathBuf> = Vec::with_capacity(parts.len());
    for part in parts {
        if !unique.contains(&part) {
            unique.push(part);
        }
    }
    std::env::join_paths(unique).unwrap_or_else(|_| path.unwrap_or_default().to_os_string())
}

fn resolve_program_in(
    name: &str,
    path: Option<&OsStr>,
    fallback_dirs: &[PathBuf],
) -> Option<PathBuf> {
    if name.contains('/') || name.contains('\\') {
        let program = PathBuf::from(name);
        return executable_file(&program).then_some(program);
    }

    let dirs: Vec<PathBuf> = path
        .into_iter()
        .flat_map(std::env::split_paths)
        .chain(fallback_dirs.iter().cloned())
        .collect();
    let names = executable_names(name);
    dirs.into_iter().find_map(|dir| {
        names
            .iter()
            .map(|name| dir.join(name))
            .find(|candidate| executable_file(candidate))
    })
}

/// Windows 上 npm 提供的 `npx` 是 `npx.cmd`，`CreateProcessW` 不会像 shell 那样
/// 按 PATHEXT 依次尝试，所以这里显式给出候选文件名。
fn executable_names(name: &str) -> Vec<String> {
    if cfg!(windows) {
        if Path::new(name).extension().is_some() {
            return vec![name.to_string()];
        }
        return vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            format!("{name}.bat"),
            name.to_string(),
        ];
    }
    vec![name.to_string()]
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    metadata.is_file() && is_executable(&metadata)
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    true
}

/// `.cmd` / `.bat` 不能直接由 `CreateProcessW` 执行，必须经由 `cmd /C`。
fn program_command(program: &Path) -> Command {
    if needs_shell_wrapper(program) {
        let mut command = Command::new("cmd");
        command.arg("/C").arg(program);
        command
    } else {
        Command::new(program)
    }
}

fn needs_shell_wrapper(program: &Path) -> bool {
    if !cfg!(windows) {
        return false;
    }
    program
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "cmd" | "bat"))
}

#[cfg(not(windows))]
fn fallback_program_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = SYSTEM_PROGRAM_DIRS.iter().map(PathBuf::from).collect();
    let Some(home) = dirs::home_dir() else {
        return dirs;
    };
    dirs.extend(HOME_PROGRAM_DIRS.iter().map(|rel| home.join(rel)));
    dirs.extend(VENDOR_PROGRAM_DIRS.iter().map(|rel| home.join(rel)));
    dirs.extend(node_version_dirs(&home));
    dirs
}

#[cfg(windows)]
fn fallback_program_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(value) = std::env::var_os("ProgramFiles") {
        dirs.push(PathBuf::from(value).join("nodejs"));
    }
    if let Some(value) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(value).join("npm"));
    }
    if let Some(value) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(value).join("Volta/bin"));
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".volta/bin"));
    }
    dirs
}

#[cfg(not(windows))]
fn node_version_dirs(home: &Path) -> Vec<PathBuf> {
    let mut versions: Vec<(Vec<u64>, PathBuf)> = Vec::new();
    collect_version_dirs(&home.join(".nvm/versions/node"), "bin", &mut versions);
    collect_version_dirs(
        &home.join("Library/Application Support/fnm/node-versions"),
        "installation/bin",
        &mut versions,
    );
    collect_version_dirs(
        &home.join(".local/share/fnm/node-versions"),
        "installation/bin",
        &mut versions,
    );
    // 新版本优先：用户通常希望 `npx` 来自最新的 Node.js。
    versions.sort_by(|left, right| right.0.cmp(&left.0));
    versions.into_iter().map(|(_, path)| path).collect()
}

#[cfg(not(windows))]
fn collect_version_dirs(root: &Path, bin: &str, out: &mut Vec<(Vec<u64>, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let bin = entry.path().join(bin);
        if bin.is_dir() {
            out.push((version_key(&entry.file_name()), bin));
        }
    }
}

#[cfg(not(windows))]
fn version_key(name: &OsStr) -> Vec<u64> {
    name.to_string_lossy()
        .split(|character: char| !character.is_ascii_digit())
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "process-util-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_program(path: &Path) {
        std::fs::write(path, "#!/bin/sh\n").expect("write fake program");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                .expect("mark fake program executable");
        }
    }

    #[test]
    fn resolution_prefers_path_over_fallback_dirs() {
        let on_path = temp_dir("path-first");
        let fallback = temp_dir("fallback-second");
        let program_name = executable_names("navop-test-program")[0].clone();
        write_program(&on_path.join(&program_name));
        write_program(&fallback.join(&program_name));

        let path = std::env::join_paths([on_path.as_path()]).expect("join path");
        let resolved = resolve_program_in(
            "navop-test-program",
            Some(path.as_os_str()),
            std::slice::from_ref(&fallback),
        );

        assert_eq!(Some(on_path.join(&program_name)), resolved);
        let _ = std::fs::remove_dir_all(&on_path);
        let _ = std::fs::remove_dir_all(&fallback);
    }

    #[test]
    fn resolution_falls_back_to_gui_invisible_dirs() {
        let fallback = temp_dir("fallback-only");
        let program_name = executable_names("navop-test-program")[0].clone();
        write_program(&fallback.join(&program_name));

        let resolved =
            resolve_program_in("navop-test-program", None, std::slice::from_ref(&fallback));

        assert_eq!(Some(fallback.join(&program_name)), resolved);
        let _ = std::fs::remove_dir_all(&fallback);
    }

    /// Windows 没有可执行位，可执行性由扩展名决定（见
    /// `windows_cmd_shims_are_resolved_and_run_through_cmd`），这里只验证 unix。
    #[cfg(unix)]
    #[test]
    fn resolution_skips_non_executable_files() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("not-executable");
        let program_name = executable_names("navop-test-program")[0].clone();
        std::fs::write(dir.join(&program_name), "#!/bin/sh\n").expect("write fake program");
        std::fs::set_permissions(
            dir.join(&program_name),
            std::fs::Permissions::from_mode(0o644),
        )
        .expect("drop execute bit");

        assert_eq!(
            None,
            resolve_program_in("navop-test-program", None, std::slice::from_ref(&dir))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn absolute_paths_are_used_without_searching() {
        let dir = temp_dir("absolute");
        let program_name = executable_names("navop-test-program")[0].clone();
        let program = dir.join(&program_name);
        write_program(&program);

        let resolved = resolve_program_in(&program.to_string_lossy(), None, &[]);

        assert_eq!(Some(program), resolved);
        assert_eq!(
            None,
            resolve_program_in("/definitely/missing/navop-test-program", None, &[]),
            "缺失的绝对路径必须解析失败，而不是回落到 PATH 搜索"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn child_path_prepends_the_program_directory_once() {
        let program = Path::new("/opt/navop-toolchain/bin/npx");
        let path = child_path_with_program_dir(program);
        let parts: Vec<PathBuf> = std::env::split_paths(&path).collect();

        assert_eq!(
            Some(Path::new("/opt/navop-toolchain/bin")),
            parts.first().map(PathBuf::as_path)
        );
        assert_eq!(
            1,
            parts
                .iter()
                .filter(|part| part.as_path() == Path::new("/opt/navop-toolchain/bin"))
                .count()
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_cmd_shims_are_resolved_and_run_through_cmd() {
        assert_eq!(
            ["npx.exe", "npx.cmd", "npx.bat", "npx"],
            executable_names("npx").as_slice()
        );
        assert!(needs_shell_wrapper(Path::new("C:/node/npx.cmd")));
        assert!(!needs_shell_wrapper(Path::new("C:/node/npx.exe")));
    }

    /// 子进程 PATH 必须带上 GUI 进程看不到的工具链目录，否则被拉起的 CLI
    /// 找不到同机安装的兄弟命令（`pi-acp` 找 `pi` 就是这么失败的）。
    #[test]
    fn child_path_appends_gui_invisible_toolchain_dirs() {
        let fallback = temp_dir("child-path-fallback");
        let program = Path::new("/opt/navop-toolchain/bin/npx");
        let process_path = std::env::join_paths([Path::new("/usr/bin"), Path::new("/bin")])
            .expect("join process path");

        let path = child_path_in(
            program,
            Some(process_path.as_os_str()),
            std::slice::from_ref(&fallback),
        );
        let parts: Vec<PathBuf> = std::env::split_paths(&path).collect();

        assert_eq!(
            Some(Path::new("/opt/navop-toolchain/bin")),
            parts.first().map(PathBuf::as_path),
            "程序目录必须排在最前，npx 才能找到同目录的 node"
        );
        assert!(
            parts.contains(&PathBuf::from("/usr/bin")),
            "当前进程的 PATH 不能被丢掉：{parts:?}"
        );
        assert_eq!(
            Some(fallback.as_path()),
            parts
                .iter()
                .find(|part| part.as_path() == fallback)
                .map(PathBuf::as_path),
            "兜底工具链目录必须补进去：{parts:?}"
        );
        assert_eq!(
            1,
            parts.iter().filter(|part| *part == &fallback).count(),
            "兜底目录只出现一次"
        );
        let _ = std::fs::remove_dir_all(&fallback);
    }

    /// PATH 被裁剪到只剩工具链目录时，`sh` / `env` 这类基础命令也不能丢。
    #[cfg(not(windows))]
    #[test]
    fn child_path_always_keeps_base_system_dirs() {
        let path = child_path_in(Path::new("/opt/navop-toolchain/bin/npx"), None, &[]);
        let parts: Vec<PathBuf> = std::env::split_paths(&path).collect();

        for dir in BASE_SYSTEM_DIRS {
            assert!(
                parts.contains(&PathBuf::from(dir)),
                "缺少基础目录 {dir}: {parts:?}"
            );
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_uses_the_bare_program_name() {
        assert_eq!(vec!["npx".to_string()], executable_names("npx"));
        assert!(!needs_shell_wrapper(Path::new("/opt/homebrew/bin/npx")));
    }

    /// Kimi CLI 的安装脚本只写 `~/.kimi-code/bin`，兜底目录少了这条，
    /// GUI（launchd PATH）里就会报「找不到命令 `kimi`」。
    #[cfg(not(windows))]
    #[test]
    fn fallback_dirs_cover_vendor_self_installed_cli() {
        let home = dirs::home_dir().expect("home dir");
        let dirs = fallback_program_dirs();

        for dir in VENDOR_PROGRAM_DIRS {
            assert!(
                dirs.contains(&home.join(dir)),
                "兜底目录缺少 {dir}: {dirs:?}"
            );
        }
    }

    /// 进程 PATH 里没有用户目录时，仍然要从兜底目录解析出厂商自装的 CLI。
    #[cfg(not(windows))]
    #[test]
    fn vendor_installed_program_resolves_from_fallback_dirs() {
        let home = temp_dir("vendor-home");
        let vendor = home.join(VENDOR_PROGRAM_DIRS[0]);
        std::fs::create_dir_all(&vendor).expect("create vendor dir");
        write_program(&vendor.join("kimi"));

        let resolved = resolve_program_in(
            "kimi",
            Some(OsStr::new("/usr/bin:/bin:/usr/sbin:/sbin")),
            std::slice::from_ref(&vendor),
        );

        assert_eq!(Some(vendor.join("kimi")), resolved);
        let _ = std::fs::remove_dir_all(&home);
    }
}
