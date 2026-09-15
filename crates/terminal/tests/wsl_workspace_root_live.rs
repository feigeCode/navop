//! WSL 文件树根目录真机集成测试。
//!
//! 背景：WSL 终端在模型层只是一个跑 `wsl.exe --distribution <发行版>` 的本地终端，
//! 没有 Windows 工作目录。若文件树沿用普通本地会话的回落逻辑（`dirs::home_dir()`），
//! 它就会显示本机磁盘而不是发行版里的文件。
//!
//! 本测试覆盖「会话配置 → 发行版 → 本机可访问路径 → 真的能读出文件」这条链，
//! 并且排除「其实是本机 Windows 目录」这一失败形态：
//!   1. `resolve_local_workspace_root` 必须给出 `\\wsl$\<发行版>`；
//!   2. 该根目录下必须看到 Linux 目录（`etc`/`usr`/`home`），且不含 `Users`/`Windows`；
//!   3. 能真的读出 `\\wsl$\<发行版>\etc\passwd` 的内容；
//!   4. 文件树加载时会先 `canonicalize`，该形式也必须仍然可列目录；
//!   5. 终端上报的 Linux cwd（`/etc`）必须被映射回同一文件系统。
//!
//! 运行（`#[ignore]`，需显式开启；需本机已安装 WSL）：
//! ```text
//! cargo test -p terminal --test wsl_workspace_root_live -- --ignored --nocapture
//!
//! NAVOP_LIVE_WSL="Ubuntu-24.04,Debian" \
//!   cargo test -p terminal --test wsl_workspace_root_live -- --ignored --nocapture
//! ```
//! 未安装的发行版会被跳过并打印原因；全部不可用时直接返回，不视为失败。
//! 注意：本机内存只有 5.9GB，编译务必加 `CARGO_BUILD_JOBS=2`。

#![cfg(target_os = "windows")]

use std::path::Path;
use terminal::terminal::resolve_local_workspace_root;
use terminal::{
    list_wsl_distributions, local_config_for_wsl_distro, resolve_reported_working_dir, wsl_unc_root,
};

/// 未指定 `NAVOP_LIVE_WSL` 时尝试使用的发行版。
const DEFAULT_DISTRO: &str = "Ubuntu-24.04";

/// 发行版根目录下必须存在的 Linux 顶层目录。
const REQUIRED_LINUX_DIRS: [&str; 3] = ["etc", "usr", "home"];

/// 一旦出现这些名字，说明文件树其实指向了本机 Windows 目录。
const WINDOWS_ONLY_DIRS: [&str; 3] = ["Users", "Windows", "Program Files"];

#[test]
#[ignore = "真机 WSL 集成测试，需显式开启（见文件头注释）"]
fn wsl_workspace_root_points_at_the_distribution_filesystem() {
    for_each_distro(check_workspace_root);
}

#[test]
#[ignore = "真机 WSL 集成测试，需显式开启（见文件头注释）"]
fn wsl_reported_working_dir_stays_inside_the_distribution() {
    for_each_distro(check_reported_working_dir);
}

fn for_each_distro(check: fn(&str) -> Result<(), String>) {
    let distros = live_distros();
    if distros.is_empty() {
        eprintln!("跳过：本机没有可用的 WSL 发行版（可用 NAVOP_LIVE_WSL 指定）");
        return;
    }
    let mut failures = Vec::new();
    for distro in &distros {
        match check(distro) {
            Ok(()) => eprintln!("OK  {distro}"),
            Err(error) => failures.push(error),
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// 解析 `NAVOP_LIVE_WSL`，并过滤掉本机未安装的发行版。
fn live_distros() -> Vec<String> {
    let requested: Vec<String> = std::env::var("NAVOP_LIVE_WSL")
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| vec![DEFAULT_DISTRO.to_string()]);

    let installed = list_wsl_distributions()
        .map(|distributions| {
            distributions
                .into_iter()
                .map(|distro| distro.name)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    requested
        .into_iter()
        .filter(|name| {
            let available = installed.iter().any(|installed| installed == name);
            if !available {
                eprintln!("跳过 {name}：本机未安装该发行版（已安装：{installed:?}）");
            }
            available
        })
        .collect()
}

fn check_workspace_root(distro: &str) -> Result<(), String> {
    let root = workspace_root_for(distro)?;

    let names = entry_names(&root)?;
    for required in REQUIRED_LINUX_DIRS {
        if !names.iter().any(|name| name == required) {
            return Err(format!(
                "{distro}: {} 下缺少 Linux 目录 {required}，实际目录项 {names:?}",
                root.display()
            ));
        }
    }
    for forbidden in WINDOWS_ONLY_DIRS {
        if names.iter().any(|name| name == forbidden) {
            return Err(format!(
                "{distro}: {} 看起来是本机 Windows 目录（含 {forbidden}）",
                root.display()
            ));
        }
    }

    // 文件树加载时会先 canonicalize，规范化后的形式必须仍然可列目录
    let canonical = std::fs::canonicalize(&root)
        .map_err(|error| format!("{distro}: canonicalize {} 失败：{error}", root.display()))?;
    let canonical_names = entry_names(&canonical)?;
    if canonical_names.len() != names.len() {
        return Err(format!(
            "{distro}: {} 规范化后（{}）目录项数量从 {} 变为 {}",
            root.display(),
            canonical.display(),
            names.len(),
            canonical_names.len()
        ));
    }

    // 真的读出内容：证明该路径可被普通文件 API 使用，而不只是能列目录
    let passwd = root.join("etc").join("passwd");
    let contents = std::fs::read_to_string(&passwd)
        .map_err(|error| format!("{distro}: 读取 {} 失败：{error}", passwd.display()))?;
    if !contents.contains("root:") {
        return Err(format!(
            "{distro}: {} 内容不像 /etc/passwd：{}",
            passwd.display(),
            contents.chars().take(80).collect::<String>()
        ));
    }

    Ok(())
}

fn check_reported_working_dir(distro: &str) -> Result<(), String> {
    let root = workspace_root_for(distro)?;

    let mapped = resolve_reported_working_dir(&root, "/etc")
        .ok_or_else(|| format!("{distro}: 发行版内的 /etc 未被映射"))?;
    let expected = root.join("etc");
    if mapped != expected {
        return Err(format!(
            "{distro}: /etc 应映射到 {}，实际为 {}",
            expected.display(),
            mapped.display()
        ));
    }
    entry_names(&mapped)?;

    // 相对路径与向上越界都无法确定落在发行版内，必须拒绝映射
    for rejected in ["etc", "/etc/../usr", "..", "/.."] {
        if resolve_reported_working_dir(&root, rejected).is_some() {
            return Err(format!("{distro}: 不应把 {rejected:?} 映射成文件树根目录"));
        }
    }

    Ok(())
}

/// 构造该发行版的会话配置，并解析出文件树根目录。
fn workspace_root_for(distro: &str) -> Result<std::path::PathBuf, String> {
    let config = local_config_for_wsl_distro(distro)
        .map_err(|error| format!("{distro}: 构造 WSL 会话配置失败：{error}"))?;
    let expected =
        wsl_unc_root(distro).ok_or_else(|| format!("{distro}: 无法构造发行版 UNC 根目录"))?;
    let root = resolve_local_workspace_root(&config)
        .ok_or_else(|| format!("{distro}: 未解析出文件树根目录"))?;
    if root != expected {
        return Err(format!(
            "{distro}: 文件树根目录应为 {}，实际为 {}",
            expected.display(),
            root.display()
        ));
    }
    Ok(root)
}

fn entry_names(path: &Path) -> Result<Vec<String>, String> {
    let entries = std::fs::read_dir(path)
        .map_err(|error| format!("读取目录 {} 失败：{error}", path.display()))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("读取 {} 的目录项失败：{error}", path.display()))?;
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}
