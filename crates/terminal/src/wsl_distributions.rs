use anyhow::Result;
#[cfg(any(test, target_os = "windows"))]
use anyhow::Context;

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
    fn new(
        name: String,
        state: Option<String>,
        version: Option<u8>,
        is_default: bool,
    ) -> Self {
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
pub(crate) fn list_wsl_distributions_with(wsl: &str) -> Result<Vec<WslDistribution>> {
    let output = std::process::Command::new(wsl)
        .args(["--list", "--verbose"])
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
pub(crate) fn local_config_for_wsl_distro_with(
    wsl: String,
    distro: &str,
) -> Result<LocalConfig> {
    let distro = distro.trim();
    anyhow::ensure!(!distro.is_empty(), "WSL distribution name is required");
    Ok(LocalConfig {
        shell: Some(wsl),
        args: vec!["--distribution".into(), distro.into()],
        working_dir: None,
        ..LocalConfig::default()
    })
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
    let version = columns.get(2).and_then(|column| column.trim().parse::<u8>().ok());
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
        (None, true) => Some(WslDistribution::new(name.to_string(), state, None, is_default)),
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