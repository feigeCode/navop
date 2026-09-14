//! ConPTY 真机集成测试（针对 issue #118 报告的现象：打开 WSL 后终端过一段时间卡死）。
//!
//! 风险背景：Windows 本地终端走 alacritty_terminal 的 ConPTY 后端。`Conpty::on_resize`
//! 用 `assert_eq!(result, S_OK)` 断言 `ResizePseudoConsole` 成功；一旦该调用失败，整个
//! PTY 读取线程会被 panic 掉：此后既不再读 PTY 也不再接受输入，进程却仍然存活 ——
//! 用户看到的就是「静默卡死」。
//!
//! **已实测的边界（Windows 10 19045 / 系统内置 ConPTY / 本机无第三方 `conpty.dll`）**：
//!   * `0x0`、`-1`、`i16::MIN` 等零/越界尺寸，`ResizePseudoConsole` 都返回 `S_OK`，
//!     而 `WindowSize -> COORD` 的 `as i16` 截断并不会让它失败 —— 这条链**不会**触发断言；
//!   * 唯一实测到的失败来源是**句柄已失效**（`ClosePseudoConsole` 之后）返回
//!     `E_HANDLE(0x80070006)`，即「会话拆除后仍到达一次 resize」的竞态。
//!
//! 因此本测试验证的是**加固行为**，而不是一个已被证实的固定根因：
//!   1. 无法安全提交的尺寸（0 / 超 i16 / 截断为负）不会杀死后端；
//!   2. 高频 resize 抖动后后端仍可继续读写；
//!   3. 多个目标实例并发互不影响；
//!   4. 主动 shutdown 属于正常结束，不会被误报为后端异常停止。
//!
//! ## 与真实 shell 交互的两条硬约定
//!
//! 1. **每行命令必须以 `\r` 结尾**。只写 `\n` 时 shell 不会执行命令：PowerShell 会停在
//!    `>>` 续行提示符上，cmd 也只是把输入原样回显 —— 早期版本因此整批出现**假阳性**
//!    （断言被「输入回显」满足，命令其实从未运行）。
//! 2. **断言用的 marker 不得出现在输入行里**，必须由 shell 变量拼接后在 stdout 输出
//!    （见 `marker_command`），否则「命令执行成功」与「输入被回显」无法区分。
//!
//! 覆盖两类真实 ConPTY 目标：
//!   * `wsl.exe --distribution <name>`：issue #118 的原始场景（需要本机已安装 WSL）；
//!   * 本机原生 shell（PowerShell / cmd）：**不需要管理员权限或 WSL**，可随时回归。
//!
//! 运行（全部 `#[ignore]`，需显式开启）：
//! ```text
//! # 仅本机 ConPTY（默认目标 powershell.exe，无需 WSL / 管理员）
//! cargo test -p terminal --test conpty_live -- --ignored --nocapture
//!
//! # 指定本机 shell
//! NAVOP_LIVE_CONPTY="powershell.exe,cmd.exe" \
//!   cargo test -p terminal --test conpty_live -- --ignored --nocapture
//!
//! # 追加 WSL 发行版（逗号分隔，需本机已安装 WSL）
//! NAVOP_LIVE_WSL="Ubuntu-24.04,Debian" \
//!   cargo test -p terminal --test conpty_live -- --ignored --nocapture
//! ```
//! 不可用的目标会打印跳过原因并被过滤掉；全部不可用时各用例直接返回，不视为失败。
//! 注意：本机内存只有 5.9GB，编译务必加 `CARGO_BUILD_JOBS=2`。

#![cfg(target_os = "windows")]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::tty::{Options as PtyOptions, Shell};
use terminal::pty_backend::{GpuiEventProxy, LocalPtyBackend, TerminalEvent};
use terminal::{
    LocalConfig, TerminalPerformanceMetrics, TerminalSize, list_wsl_distributions,
    local_config_for_wsl_distro,
};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// 未指定 `NAVOP_LIVE_CONPTY` 时使用的本机 shell（Windows 上始终存在）。
const DEFAULT_LOCAL_SHELL: &str = "powershell.exe";

/// 首屏输出等待上限：WSL2 冷启动（首启 + VM 拉起）可能较慢。
const BOOT_TIMEOUT: Duration = Duration::from_secs(120);
/// 单条命令回显等待上限。
const ECHO_TIMEOUT: Duration = Duration::from_secs(30);

type SharedTerm = Arc<FairMutex<Term<GpuiEventProxy>>>;

/// 一个待验证的 ConPTY 目标。
#[derive(Clone, Debug)]
enum Target {
    /// `wsl.exe --distribution <name>`。
    Wsl(String),
    /// 本机原生 shell。
    Local { program: String },
}

impl Target {
    /// 用于日志与断言消息的稳定标识。
    fn label(&self) -> String {
        match self {
            Self::Wsl(name) => format!("wsl:{name}"),
            Self::Local { program } => format!("local:{program}"),
        }
    }

    /// 本机是否具备运行该目标的条件；不满足时返回原因。
    fn availability(&self) -> Result<(), String> {
        match self {
            Self::Wsl(name) => {
                let distros = list_wsl_distributions()
                    .map_err(|error| format!("无法枚举 WSL 发行版（{error}）"))?;
                if distros.iter().any(|distro| &distro.name == name) {
                    Ok(())
                } else {
                    Err(format!(
                        "发行版 {name} 不在已安装列表中：{}",
                        distros
                            .iter()
                            .map(|distro| distro.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                }
            }
            Self::Local { program } => {
                let found = std::process::Command::new("where.exe")
                    .arg(program)
                    .output()
                    .map(|output| output.status.success())
                    .unwrap_or(false);
                if found {
                    Ok(())
                } else {
                    Err(format!("PATH 上找不到 {program}"))
                }
            }
        }
    }

    /// 构造启动该目标所需的本地终端配置。
    fn local_config(&self) -> Result<LocalConfig, String> {
        match self {
            Self::Wsl(name) => local_config_for_wsl_distro(name)
                .map_err(|error| format!("构造发行版 {name} 的本地终端配置失败：{error}")),
            Self::Local { program } => Ok(LocalConfig {
                shell: Some(program.clone()),
                args: Vec::new(),
                working_dir: None,
                env: Vec::new(),
            }),
        }
    }
}

/// 读取环境变量中的逗号分隔列表。
fn split_env(name: &str) -> Vec<String> {
    std::env::var(name)
        .unwrap_or_default()
        .split(',')
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

/// 解析本次要跑的目标；两个环境变量都没设置时退回本机 `powershell.exe`。
fn live_targets() -> Vec<Target> {
    let mut targets: Vec<Target> = split_env("NAVOP_LIVE_WSL")
        .into_iter()
        .map(Target::Wsl)
        .collect();
    targets.extend(
        split_env("NAVOP_LIVE_CONPTY")
            .into_iter()
            .map(|program| Target::Local { program }),
    );
    if targets.is_empty() {
        targets.push(Target::Local {
            program: DEFAULT_LOCAL_SHELL.to_string(),
        });
    }
    targets
}

/// 过滤出本机可用的目标；不可用的打印原因。
fn active_targets() -> Vec<Target> {
    let mut active = Vec::new();
    for target in live_targets() {
        match target.availability() {
            Ok(()) => active.push(target),
            Err(reason) => eprintln!("跳过目标 {}：{reason}", target.label()),
        }
    }
    active
}

/// 可调整的网格尺寸。
///
/// 测试必须把 alacritty 网格与提交给 ConPTY 的窗口尺寸**同步**：ConPTY 在窗口变化后
/// 会按新列宽重绘（发出带绝对坐标的序列），若网格仍按旧列宽渲染，屏幕内容会整体错位，
/// 断言随之变得不可靠。真实应用里 UI 层同样会 `term.resize`，这里只是显式对齐该行为。
struct SharedDimensions {
    rows: AtomicUsize,
    cols: AtomicUsize,
}

impl SharedDimensions {
    fn new(rows: u16, cols: u16) -> Self {
        Self {
            rows: AtomicUsize::new(rows as usize),
            cols: AtomicUsize::new(cols as usize),
        }
    }

    fn apply(&self, size: TerminalSize) {
        self.rows.store(size.rows as usize, Ordering::Relaxed);
        self.cols.store(size.cols as usize, Ordering::Relaxed);
    }

    /// `Term::resize` 按值接收尺寸，这里给出当时的快照即可（`Term` 只读取不持有它）。
    fn snapshot(&self) -> Self {
        Self {
            rows: AtomicUsize::new(self.rows.load(Ordering::Relaxed)),
            cols: AtomicUsize::new(self.cols.load(Ordering::Relaxed)),
        }
    }
}

impl Dimensions for SharedDimensions {
    fn total_lines(&self) -> usize {
        self.rows.load(Ordering::Relaxed)
    }

    fn screen_lines(&self) -> usize {
        self.rows.load(Ordering::Relaxed)
    }

    fn columns(&self) -> usize {
        self.cols.load(Ordering::Relaxed)
    }
}

/// 这些尺寸无法用 ConPTY 的 `COORD`（i16 对）表达，属于必须被加固挡住的输入。
fn is_unsafe_size(size: TerminalSize) -> bool {
    const MAX_COORD: u16 = i16::MAX as u16;
    size.rows == 0 || size.cols == 0 || size.rows > MAX_COORD || size.cols > MAX_COORD
}

struct Spawned {
    label: String,
    target: Target,
    backend: LocalPtyBackend,
    term: SharedTerm,
    dimensions: Arc<SharedDimensions>,
    events: UnboundedReceiver<TerminalEvent>,
}

impl Spawned {
    /// 模拟 UI 的尺寸调整：合法尺寸时同时调整网格与 PTY，非法尺寸只提交给后端
    /// （后端应当自行拒绝，网格不受影响）。
    fn resize(&self, size: TerminalSize) {
        if !is_unsafe_size(size) {
            self.dimensions.apply(size);
            self.term.lock().resize(self.dimensions.snapshot());
        }
        self.backend.resize(size);
    }
}

/// 用真实 ConPTY 起一个本地 PTY 后端。
fn spawn_target(target: &Target) -> Result<Spawned, String> {
    let label = target.label();
    let config = target.local_config()?;
    let shell = config
        .shell
        .clone()
        .ok_or_else(|| format!("{label}: 目标配置缺少 shell 路径"))?;

    let (event_tx, event_rx) = unbounded_channel();
    let metrics = Arc::new(TerminalPerformanceMetrics::enabled());
    let proxy = GpuiEventProxy::with_metrics(event_tx, metrics);

    let dimensions = Arc::new(SharedDimensions::new(24, 80));
    let term = Term::new(TermConfig::default(), &*dimensions, proxy.clone());
    let term = Arc::new(FairMutex::new(term));

    let options = PtyOptions {
        shell: Some(Shell::new(shell, config.args)),
        working_directory: None,
        env: config.env.into_iter().collect::<HashMap<_, _>>(),
        drain_on_exit: true,
        escape_args: true,
    };

    let backend = LocalPtyBackend::new(term.clone(), proxy, options)
        .map_err(|error| format!("{label}: 创建本地 PTY 后端失败：{error}"))?;

    Ok(Spawned {
        label,
        target: target.clone(),
        backend,
        term,
        dimensions,
        events: event_rx,
    })
}

/// 把当前屏幕（含回滚区）拼成文本，用于断言命令回显。
fn screen_text(term: &SharedTerm) -> String {
    let term = term.lock();
    let grid = term.grid();
    let columns = grid.columns();
    let mut text = String::new();
    for indexed in grid.display_iter() {
        if indexed.flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        text.push(indexed.c);
        if indexed.point.column.0 + 1 == columns {
            text.push('\n');
        }
    }
    text
}

/// 排空事件队列中已经到达的事件。
fn drain_events(events: &mut UnboundedReceiver<TerminalEvent>) -> Vec<TerminalEvent> {
    let mut collected = Vec::new();
    while let Ok(event) = events.try_recv() {
        collected.push(event);
    }
    collected
}

/// 统计事件队列中 `BackendStopped` 的数量。
fn backend_stopped_count(events: &mut UnboundedReceiver<TerminalEvent>) -> usize {
    drain_events(events)
        .into_iter()
        .filter(|event| matches!(event, TerminalEvent::BackendStopped))
        .count()
}

/// 轮询等待屏幕上出现指定文本。
fn wait_for_text(term: &SharedTerm, timeout: Duration, needle: &str) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if screen_text(term).contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// 等待屏幕出现任何非空白内容，作为「会话已起来」的信号。
fn wait_for_first_output(term: &SharedTerm, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !screen_text(term).trim().is_empty() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// 轮询等待后端进入「已停止」状态。
fn wait_for_stopped(backend: &LocalPtyBackend, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if backend.is_stopped() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// 向后端写入一条 marker 命令并等待其**输出**出现；返回会话是否仍然可读写。
fn round_trip_marker(spawned: &Spawned, marker: &str) -> bool {
    spawned.backend.write(marker_command(&spawned.target, marker));
    wait_for_text(&spawned.term, ECHO_TIMEOUT, marker)
}

/// 生成一条只在 stdout 上打印 marker 的命令。
///
/// 两个关键点（都曾让这套测试出现**假阳性**）：
/// 1. 每行必须以 `\r`（回车）结尾：只写 `\n` 不会让 shell 执行命令 ——
///    PowerShell 会停在 `>>` 续行提示符上，cmd 也只是把输入原样回显；
/// 2. 命令文本本身**不含完整 marker**（由 shell 变量拼接生成），
///    否则「命令真的执行了」会被「输入被回显」冒充，断言形同虚设。
fn marker_command(target: &Target, marker: &str) -> Vec<u8> {
    let (head, tail) = marker.split_at(marker.len() / 2);
    let lines: Vec<String> = match target {
        // cmd 的 `%VAR%` 在解析整行时就会展开，故赋值与使用必须分成不同行。
        Target::Local { program } if is_cmd_exe(program) => vec![
            format!("set \"NAVOP_A={head}\""),
            format!("set \"NAVOP_B={tail}\""),
            "echo %NAVOP_A%%NAVOP_B%".to_string(),
        ],
        Target::Local { .. } => vec![
            format!("$a='{head}'"),
            format!("$b='{tail}'"),
            "Write-Output ($a+$b)".to_string(),
        ],
        Target::Wsl(_) => vec![
            format!("a='{head}'"),
            format!("b='{tail}'"),
            "echo $a$b".to_string(),
        ],
    };
    let mut bytes = Vec::new();
    for line in lines {
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\r');
    }
    bytes
}

/// 目标程序是否为 Windows 命令解释器（其变量展开语法与 POSIX shell / PowerShell 不同）。
fn is_cmd_exe(program: &str) -> bool {
    std::path::Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("cmd.exe"))
}

/// 线性同余伪随机数，避免为测试引入额外依赖。
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_in(&mut self, low: u16, high: u16) -> u16 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let span = (high - low) as u64 + 1;
        low + ((self.0 >> 33) % span) as u16
    }
}

fn size(rows: u16, cols: u16) -> TerminalSize {
    TerminalSize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// 这些尺寸无法用 ConPTY 的 `COORD`（i16 对）表达，属于必须被加固挡住的输入。
fn unsafe_sizes() -> Vec<TerminalSize> {
    vec![
        size(0, 0),
        size(0, 80),
        size(24, 0),
        size(u16::MAX, u16::MAX),
        size(u16::MAX, 80),
        size(24, u16::MAX),
        size(i16::MAX as u16 + 1, 100),
        size(100, i16::MAX as u16 + 1),
    ]
}

/// 等待会话首屏输出，失败即返回可读的错误信息。
fn ensure_booted(spawned: &Spawned) -> Result<(), String> {
    if wait_for_first_output(&spawned.term, BOOT_TIMEOUT) {
        Ok(())
    } else {
        Err(format!(
            "{}: 等待首屏输出超时，会话未正常启动",
            spawned.label
        ))
    }
}

/// 场景一：非法尺寸不得杀死后端，且之后仍能正常读写。
fn check_unsafe_sizes(target: &Target) -> Result<(), String> {
    let spawned = spawn_target(target)?;
    ensure_booted(&spawned)?;

    for bad in unsafe_sizes() {
        spawned.resize(bad);
        std::thread::sleep(Duration::from_millis(30));
        if spawned.backend.is_stopped() {
            return Err(format!(
                "{}: 提交尺寸 {}x{} 后后端被杀死",
                spawned.label, bad.cols, bad.rows
            ));
        }
    }

    // 非法尺寸不应破坏后续合法尺寸与正常读写。
    spawned.resize(size(30, 100));
    if !round_trip_marker(&spawned, "__NAVOP_UNSAFE_OK__") {
        return Err(format!(
            "{}: 非法尺寸之后会话已无法回显（stopped={}）\n--- 屏幕快照 ---\n{}\n--- 快照结束 ---",
            spawned.label,
            spawned.backend.is_stopped(),
            screen_text(&spawned.term)
        ));
    }

    let mut events = spawned.events;
    let stopped = backend_stopped_count(&mut events);
    spawned.backend.shutdown();
    if stopped != 0 {
        return Err(format!(
            "{}: 不应上报后端异常停止（实际 {stopped} 次）",
            spawned.label
        ));
    }
    Ok(())
}

/// 场景二：高频 resize 抖动（含周期性非法尺寸）后会话仍可响应。
fn check_resize_storm(target: &Target) -> Result<(), String> {
    let spawned = spawn_target(target)?;
    ensure_booted(&spawned)?;

    // 大部分为合法尺寸，每 7 次夹一个非法尺寸，模拟 UI 抖动 + 极端窗口。
    let mut rng = Lcg::new(0x5eed_1181);
    let mut unsafe_hits = 0u32;
    for round in 0..240u32 {
        let next = if round % 7 == 6 {
            unsafe_hits += 1;
            match round % 3 {
                0 => size(0, rng.next_in(1, 200)),
                1 => size(rng.next_in(1, 200), 0),
                _ => size(u16::MAX, u16::MAX),
            }
        } else {
            size(rng.next_in(10, 60), rng.next_in(20, 240))
        };
        spawned.resize(next);
        std::thread::sleep(Duration::from_millis(4));
    }

    if unsafe_hits == 0 {
        return Err(format!("{}: 抖动序列必须包含非法尺寸", spawned.label));
    }
    if spawned.backend.is_stopped() {
        return Err(format!(
            "{}: {unsafe_hits} 次非法尺寸之后的 resize 抖动杀死了后端",
            spawned.label
        ));
    }
    if !round_trip_marker(&spawned, "__NAVOP_STORM_OK__") {
        return Err(format!("{}: resize 抖动后会话不再回显", spawned.label));
    }

    spawned.backend.shutdown();
    Ok(())
}

/// 场景三：多目标并发抖动互不影响。
fn check_concurrent_targets(targets: &[Target]) -> Result<(), String> {
    let mut sessions: Vec<Spawned> = Vec::new();
    for target in targets {
        sessions.push(spawn_target(target)?);
    }

    let mut failures = Vec::new();
    for spawned in &sessions {
        if let Err(message) = ensure_booted(spawned) {
            failures.push(message);
        }
    }

    // 并发抖动：所有实例同时连续提交非法尺寸。
    let rounds = AtomicU32::new(0);
    for round in 0..60u32 {
        for spawned in &sessions {
            let bad = match round % 3 {
                0 => size(0, 0),
                1 => size(u16::MAX, u16::MAX),
                _ => size(24, i16::MAX as u16 + 1),
            };
            spawned.resize(bad);
            rounds.fetch_add(1, Ordering::Relaxed);
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    for spawned in &sessions {
        if spawned.backend.is_stopped() {
            failures.push(format!("{}: 并发抖动后后端被杀死", spawned.label));
        }
    }
    for spawned in &sessions {
        if !round_trip_marker(spawned, "__NAVOP_CONCURRENT_OK__") {
            failures.push(format!("{}: 并发抖动后会话不再回显", spawned.label));
        }
    }

    for spawned in sessions {
        spawned.backend.shutdown();
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

/// 场景四：主动 shutdown 属于正常结束，不得误报为后端异常停止。
fn check_clean_shutdown(target: &Target) -> Result<(), String> {
    let mut spawned = spawn_target(target)?;
    ensure_booted(&spawned)?;

    spawned.backend.shutdown();
    if !wait_for_stopped(&spawned.backend, Duration::from_secs(30)) {
        return Err(format!(
            "{}: shutdown() 之后事件循环未在超时内结束",
            spawned.label
        ));
    }

    let stopped = backend_stopped_count(&mut spawned.events);
    if stopped != 0 {
        return Err(format!(
            "{}: 主动关闭属于正常结束，不得上报 BackendStopped（实际 {stopped} 次）",
            spawned.label
        ));
    }

    // 停止后再写入不应 panic，也不应让后端复活。
    spawned.backend.write(b"echo should_be_dropped\n".to_vec());
    if !spawned.backend.is_stopped() {
        return Err(format!("{}: 停止后的写入让后端复活", spawned.label));
    }
    Ok(())
}

/// 对每个可用目标跑同一个检查，汇总所有失败信息。
fn for_each_target<F>(scenario: F)
where
    F: Fn(&Target) -> Result<(), String>,
{
    let targets = active_targets();
    if targets.is_empty() {
        eprintln!("跳过：没有可用的 ConPTY 目标");
        return;
    }
    let mut failures = Vec::new();
    for target in &targets {
        eprintln!("运行目标 {}", target.label());
        if let Err(message) = scenario(target) {
            failures.push(message);
        }
    }
    assert!(failures.is_empty(), "失败项：\n{}", failures.join("\n"));
}

#[test]
#[ignore = "真机 ConPTY 集成测试，需显式开启（见文件头注释）"]
fn conpty_unsafe_sizes_do_not_kill_backend() {
    for_each_target(check_unsafe_sizes);
}

#[test]
#[ignore = "真机 ConPTY 集成测试，需显式开启（见文件头注释）"]
fn conpty_resize_storm_keeps_session_responsive() {
    for_each_target(check_resize_storm);
}

#[test]
#[ignore = "真机 ConPTY 集成测试，需显式开启（见文件头注释）"]
fn conpty_concurrent_targets_are_independent() {
    let targets = active_targets();
    if targets.is_empty() {
        eprintln!("跳过：没有可用的 ConPTY 目标");
        return;
    }
    if let Err(message) = check_concurrent_targets(&targets) {
        panic!("失败项：\n{message}");
    }
}

#[test]
#[ignore = "真机 ConPTY 集成测试，需显式开启（见文件头注释）"]
fn conpty_clean_shutdown_is_not_reported_as_stopped() {
    for_each_target(check_clean_shutdown);
}
