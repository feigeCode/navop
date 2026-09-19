#[cfg(any(test, target_os = "windows"))]
use anyhow::Context;
use anyhow::Result;
use std::path::{Component, Path, PathBuf, Prefix};

use crate::LocalConfig;

/// `wsl.exe --list --verbose` 报告的一个 WSL 发行版。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WslDistribution {
    /// 注册名（如 `Ubuntu-22.04`），用于 `wsl.exe -d <name>` 启动。
    pub name: String,
    /// 运行状态（如 `Running`/`Stopped`）；仅用于展示与测试，允许缺失。
    pub state: Option<String>,
    /// WSL 版本（1/2）；旧版 wsl.exe 输出可能缺失。
    pub version: Option<u8>,
    /// 是否为默认发行版（输出行首的 `*` 标记）。
    pub is_default: bool,
}

#[cfg(any(test, target_os = "windows"))]
impl WslDistribution {
    fn new(name: String, state: Option<String>, version: Option<u8>, is_default: bool) -> Self {
        Self {
            name,
            state,
            version,
            is_default,
        }
    }
}

/// 识别已安装的 WSL 发行版（`wsl.exe --list --verbose`）。
///
/// 仅在 Windows 上可用；未安装 WSL、无发行版或 wsl.exe 不可用时返回错误。
pub fn list_wsl_distributions() -> Result<Vec<WslDistribution>> {
    #[cfg(target_os = "windows")]
    {
        list_wsl_distributions_with(&crate::local_shell::resolve_wsl())
    }
    #[cfg(not(target_os = "windows"))]
    {
        anyhow::bail!("WSL is only available on Windows")
    }
}

#[cfg(any(test, target_os = "windows"))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn list_wsl_distributions_with(wsl: &str) -> Result<Vec<WslDistribution>> {
    let mut command = std::process::Command::new(wsl);
    command.args(["--list", "--verbose"]);
    // `wsl.exe` 是控制台程序：无控制台的 GUI 进程直接 spawn 会新建一个控制台
    // 窗口（应用启动时表现为闪一下黑框），即使 stdout 已被重定向到管道也一样。
    // 统一走后台子进程约定隐藏控制台（Windows 上设置 CREATE_NO_WINDOW）。
    process_util::configure_background_child(&mut command);
    let output = command
        .output()
        .with_context(|| format!("failed to run {wsl} --list --verbose"))?;
    if !output.status.success() {
        // 未安装 WSL 或没有发行版时 wsl.exe 以非零码退出并输出本地化错误文本，
        // 这里原样透出，由调用方决定按空结果还是提示处理。
        anyhow::bail!("{}", decode_wsl_output(&output.stdout));
    }
    Ok(parse_wsl_list_output(&output.stdout))
}

/// 构造以指定发行版启动本地终端的配置（`wsl.exe -d <name>`）。
pub fn local_config_for_wsl_distro(distro: &str) -> Result<LocalConfig> {
    #[cfg(target_os = "windows")]
    {
        local_config_for_wsl_distro_with(crate::local_shell::resolve_wsl(), distro)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = distro;
        anyhow::bail!("WSL is only available on Windows")
    }
}

#[cfg(any(test, target_os = "windows"))]
pub(crate) fn local_config_for_wsl_distro_with(wsl: String, distro: &str) -> Result<LocalConfig> {
    let distro = distro.trim();
    anyhow::ensure!(!distro.is_empty(), "WSL distribution name is required");
    Ok(LocalConfig {
        shell: Some(wsl),
        args: vec!["--distribution".into(), distro.into()],
        working_dir: None,
        ..LocalConfig::default()
    })
}

// ---------------------------------------------------------------------------
// 会话 → 发行版文件系统
//
// WSL 终端在模型层只是一个跑 `wsl.exe` 的本地终端，没有 Windows 工作目录。
// 如果文件树按普通本地会话回落，就会显示本机磁盘而不是发行版里的文件；下面
// 这几个纯函数把「本地配置 → 发行版 → UNC 路径」的映射单独固化下来。
// ---------------------------------------------------------------------------

/// 生成 UNC 路径时使用的 WSL 服务器名。
///
/// `\\wsl$` 是随 WSL 提供的兼容别名，Windows 10 / 11 都可用；较新的
/// `\\wsl.localhost` 在部分系统上只暴露运行中的发行版，因此统一用前者生成。
const WSL_UNC_SERVER: &str = "wsl$";

/// 识别 UNC 根目录时接受的全部 WSL 服务器名（`\\wsl.localhost` 是新形式，
/// 用户手动选择根目录时可能命中）。
const WSL_UNC_SERVERS: [&str; 2] = [WSL_UNC_SERVER, "wsl.localhost"];

/// 若该本地配置以 `wsl.exe [--distribution|-d] <发行版>` 启动，返回发行版名。
///
/// 目标发行版只体现在启动参数里（模型层把它当作普通本地终端），这里把它解析
/// 出来供文件树定位发行版文件系统使用。
#[cfg(any(test, target_os = "windows"))]
pub(crate) fn wsl_distribution_for_config(config: &LocalConfig) -> Option<&str> {
    let shell = config.shell.as_deref()?;
    if !is_wsl_program(shell) {
        return None;
    }
    distribution_from_args(&config.args)
}

/// 发行版文件系统根目录在 Windows 侧的本机路径（`\\wsl$\<发行版>`）。
///
/// 发行版名必须是单个路径片段，否则会拼出指向别处的路径；因此非法名返回
/// `None` 而不是构造一个可疑路径。
pub fn wsl_unc_root(distro: &str) -> Option<PathBuf> {
    let distro = distro.trim();
    if !is_valid_distribution_name(distro) {
        return None;
    }
    Some(PathBuf::from(format!(r"\\{WSL_UNC_SERVER}\{distro}")))
}

/// 把发行版内的 Linux 绝对路径映射为本机可访问的 UNC 路径。
///
/// 只接受以 `/` 开头的发行版内绝对路径；相对路径与含 `..` 的路径无法确定最终
/// 落在发行版内，一律返回 `None`。
pub fn wsl_unc_path(distro: &str, linux_path: &str) -> Option<PathBuf> {
    let distro = distro.trim();
    if !is_valid_distribution_name(distro) {
        return None;
    }
    let relative = linux_path.trim().strip_prefix('/')?;
    if relative.split('/').any(|segment| segment == "..") {
        return None;
    }
    let mut mapped = format!(r"\\{WSL_UNC_SERVER}\{distro}");
    for segment in relative
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
    {
        mapped.push('\\');
        mapped.push_str(segment);
    }
    Some(PathBuf::from(mapped))
}

/// 从工作区根目录反查它指向的 WSL 发行版；非 WSL 根目录返回 `None`。
///
/// 同时识别 `\\wsl$\<发行版>` / `\\wsl.localhost\<发行版>` 与 `canonicalize()`
/// 产出的 `\\?\UNC\wsl$\<发行版>`。非 Windows 平台上 `\\wsl$\x` 只是普通文件名，
/// 不会被误判为 UNC 路径。
pub(crate) fn wsl_distribution_from_root(root: &Path) -> Option<&str> {
    let mut components = root.components();
    let Component::Prefix(prefix) = components.next()? else {
        return None;
    };
    let (server, share) = match prefix.kind() {
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => (server, share),
        _ => return None,
    };
    let server = server.to_str()?;
    if !WSL_UNC_SERVERS
        .iter()
        .any(|known| server.eq_ignore_ascii_case(known))
    {
        return None;
    }
    let share = share.to_str()?;
    (!share.is_empty()).then_some(share)
}

/// 把终端上报的工作目录映射成文件树可用的根目录。
///
/// WSL 会话里 shell 上报的是发行版内的 Linux 绝对路径（如 `/home/navop`）。
/// 若直接当成本机路径使用，它会落到当前盘符下的 `/home/navop` 而指向错误的
/// 目录；只有经 `\\wsl$\<发行版>` 映射才回到同一个文件系统。
pub fn resolve_reported_working_dir(current_root: &Path, reported: &str) -> Option<PathBuf> {
    match wsl_distribution_from_root(current_root) {
        Some(distro) => wsl_unc_path(distro, reported),
        None => Some(PathBuf::from(reported)),
    }
}

/// 配置里的 shell 是否指向 `wsl.exe`；只比较文件名，允许写成完整路径或命令名。
#[cfg(any(test, target_os = "windows"))]
fn is_wsl_program(program: &str) -> bool {
    let file_name = program
        .rsplit(|ch| ch == '\\' || ch == '/')
        .next()
        .unwrap_or(program);
    file_name.eq_ignore_ascii_case("wsl") || file_name.eq_ignore_ascii_case("wsl.exe")
}

/// 从启动参数里取出 `--distribution` / `-d` 的取值。
#[cfg(any(test, target_os = "windows"))]
fn distribution_from_args(args: &[String]) -> Option<&str> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if !matches!(arg.as_str(), "--distribution" | "-d") {
            continue;
        }
        let name = args.next()?.trim();
        return (!name.is_empty()).then_some(name);
    }
    None
}

/// 发行版名必须是单个路径片段（它会被直接拼进 UNC 路径）。
fn is_valid_distribution_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\\')
}

/// 解析 `wsl.exe --list --verbose` 的输出。
///
/// 纯函数、跨平台可测：支持 wsl.exe 管道输出的 UTF-16LE（默认）与 UTF-8；
/// 表头、错误文本等非数据行一律过滤为空结果。
#[cfg(any(test, target_os = "windows"))]
pub(crate) fn parse_wsl_list_output(raw: &[u8]) -> Vec<WslDistribution> {
    decode_wsl_output(raw)
        .lines()
        .filter_map(parse_distribution_row)
        .collect()
}

/// 单行解析规则（issue feigeCode/navop#182）：
/// - 行首 `*` 标记默认发行版；
/// - 名称与第二列（状态）以连续两个以上空格分隔；
/// - 版本列必须能解析为数字，状态列是已知状态词时也接受缺失版本列——
///   版本/状态双判据用于把本地化表头与错误文本从数据行中稳定排除。
#[cfg(any(test, target_os = "windows"))]
fn parse_distribution_row(line: &str) -> Option<WslDistribution> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let (is_default, line) = match line.strip_prefix('*') {
        Some(rest) => (true, rest.trim()),
        None => (false, line),
    };
    let columns = split_columns(line);
    let name = columns.first()?.trim();
    if name.is_empty() {
        return None;
    }
    let state = columns.get(1).map(|column| column.trim().to_string());
    let version = columns
        .get(2)
        .and_then(|column| column.trim().parse::<u8>().ok());
    let recognized_state = state
        .as_deref()
        .is_some_and(|state| KNOWN_STATES.contains(&state));
    match (version, recognized_state) {
        (Some(version), _) => Some(WslDistribution::new(
            name.to_string(),
            state,
            Some(version),
            is_default,
        )),
        (None, true) => Some(WslDistribution::new(
            name.to_string(),
            state,
            None,
            is_default,
        )),
        // 表头（版本列为 "VERSION" 等非数字）或错误文本行
        (None, false) => None,
    }
}

/// wsl.exe 表格状态列的已知词（状态词不参与本地化，仅作旧输出回退判据）。
#[cfg(any(test, target_os = "windows"))]
const KNOWN_STATES: [&str; 6] = [
    "Running",
    "Stopped",
    "Installing",
    "Uninstalling",
    "Converting",
    "Stopping",
];

/// 按连续两个以上空格拆分表格列；名称中的单个空格不受影响。
#[cfg(any(test, target_os = "windows"))]
fn split_columns(line: &str) -> Vec<&str> {
    line.split("  ")
        .map(str::trim)
        .filter(|column| !column.is_empty())
        .collect()
}

/// wsl.exe 在 stdout 被重定向时输出 UTF-16LE（可能带 BOM）；否则按 UTF-8 处理。
#[cfg(any(test, target_os = "windows"))]
pub(crate) fn decode_wsl_output(raw: &[u8]) -> String {
    if looks_like_utf16le(raw) {
        let units = raw
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(raw).into_owned()
    }
    .chars()
    .filter(|ch| *ch != '\u{feff}' && *ch != '\0')
    .collect()
}

/// 无 BOM 时以零字节占比判定：UTF-16LE 编码的文本在奇数位大量为零字节。
#[cfg(any(test, target_os = "windows"))]
fn looks_like_utf16le(raw: &[u8]) -> bool {
    if raw.starts_with(&[0xFF, 0xFE]) {
        return true;
    }
    let odd_zero_bytes = raw
        .iter()
        .enumerate()
        .filter(|(index, byte)| index % 2 == 1 && **byte == 0)
        .count();
    raw.len() >= 2 && odd_zero_bytes > raw.len() / 4
}

#[cfg(test)]
#[path = "wsl_distributions_tests.rs"]
mod tests;
