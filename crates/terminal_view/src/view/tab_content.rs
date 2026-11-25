use super::*;

const DEFAULT_RECORDING_NAME: &str = "recording.cast";

pub(super) fn recording_playback_display_name(value: &str) -> SharedString {
    let basename = value.rsplit(['/', '\\']).next().unwrap_or_default();
    let sanitized = basename
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    let sanitized = sanitized.trim();

    if sanitized.is_empty() {
        DEFAULT_RECORDING_NAME.into()
    } else {
        sanitized.to_string().into()
    }
}

pub(super) fn recording_playback_tab_title(name: &str) -> SharedString {
    t!("TerminalRecordingPlayback.tab_title", name = name)
        .to_string()
        .into()
}

pub(super) fn session_log_tab_title(name: &str) -> SharedString {
    t!("TerminalSessionLog.tab_title", name = name)
        .to_string()
        .into()
}

/// 将本地终端原始标题格式化为简洁可读的标签名。
///
/// Windows ConPTY 会推送完整可执行文件路径（如 `C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe`
/// 或 `C:\Program Files\PowerShell\7\pwsh.exe`），直接作为标签名既冗长又不雅观。
/// 本函数识别常见 shell 可执行路径并返回简洁名称；对非路径的普通标题保持原样。
pub(super) fn concise_local_terminal_title(
    preferred_title: Option<&str>,
    raw_title: &str,
    shell_path: Option<&str>,
) -> String {
    if let Some(preferred) = preferred_title.map(str::trim).filter(|s| !s.is_empty()) {
        return preferred.to_string();
    }

    let trimmed = raw_title.trim();
    if !trimmed.is_empty() {
        if let Some(friendly) = friendly_shell_name_from_path(trimmed) {
            return friendly;
        }
        if looks_like_filesystem_path(trimmed) {
            if let Some(leaf) = path_leaf_name(trimmed) {
                return leaf.to_string();
            }
        }
        return trimmed.to_string();
    }

    if let Some(shell) = shell_path.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(friendly) = friendly_shell_name_from_path(shell) {
            return friendly;
        }
        if let Some(leaf) = path_leaf_name(shell) {
            return leaf.to_string();
        }
    }

    "Terminal".to_string()
}

/// 生成本地终端标签悬停提示：展示完整终端标题和/或当前工作目录。
pub(super) fn local_terminal_tooltip(
    display_title: &str,
    raw_title: &str,
    current_working_dir: Option<&str>,
) -> Option<SharedString> {
    let mut lines: Vec<&str> = Vec::new();
    let raw_trimmed = raw_title.trim();
    if !raw_trimmed.is_empty() && raw_trimmed != display_title.trim() {
        lines.push(raw_trimmed);
    }
    if let Some(cwd) = current_working_dir.map(str::trim).filter(|s| !s.is_empty()) {
        if cwd != display_title.trim() && !lines.contains(&cwd) {
            lines.push(cwd);
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(SharedString::from(lines.join("\n")))
    }
}

fn looks_like_filesystem_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    let windows_drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    let unc = value.starts_with(r"\\");
    let unix_abs = value.starts_with('/') && value.len() > 1;
    windows_drive || unc || unix_abs
}

fn path_leaf_name(path: &str) -> Option<&str> {
    let trimmed = path.trim().trim_end_matches(['/', '\\']);
    let leaf = trimmed.rsplit(['/', '\\']).next()?;
    let leaf = leaf.trim();
    (!leaf.is_empty()).then_some(leaf)
}

fn friendly_shell_name_from_path(value: &str) -> Option<String> {
    if !looks_like_filesystem_path(value) && !value.to_ascii_lowercase().ends_with(".exe") {
        return None;
    }
    let leaf = path_leaf_name(value)?;
    let lower = leaf.to_ascii_lowercase();
    let friendly = match lower.as_str() {
        "pwsh" | "pwsh.exe" | "powershell" | "powershell.exe" => "PowerShell",
        "cmd" | "cmd.exe" => "CMD",
        "wsl" | "wsl.exe" => "WSL",
        "bash" | "bash.exe" => {
            if value.to_ascii_lowercase().contains("git") {
                "Git Bash"
            } else {
                "Bash"
            }
        }
        "zsh" | "zsh.exe" => "Zsh",
        "fish" => "Fish",
        "nu" | "nu.exe" => "Nushell",
        "sh" => "sh",
        _ => {
            if let Some(stem) = leaf.strip_suffix(".exe").or_else(|| leaf.strip_suffix(".EXE")) {
                if !stem.is_empty() {
                    return Some(stem.to_string());
                }
            }
            return None;
        }
    };
    Some(friendly.to_string())
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for TerminalView {}

impl TabContent for TerminalView {
    fn content_key(&self) -> &'static str {
        "Terminal"
    }

    fn title(&self, cx: &App) -> SharedString {
        if let Some(name) = &self.recording_playback_name {
            return recording_playback_tab_title(name.as_ref());
        }
        if let Some(name) = &self.session_log_name {
            return session_log_tab_title(name.as_ref());
        }

        let terminal = self.terminal.read(cx);
        let base_title = if let Some(name) = terminal.connection_name() {
            name.to_string()
        } else if terminal.live_connection_kind() == Some(TerminalConnectionKind::Local) {
            let shell_path = match self.duplicate_source.as_ref() {
                Some(TerminalDuplicateSource::Local(config)) => config.shell.as_deref(),
                _ => None,
            };
            concise_local_terminal_title(
                self.local_profile_title.as_deref(),
                terminal.title(),
                shell_path,
            )
        } else if !terminal.title().is_empty() {
            terminal.title().to_string()
        } else {
            "Terminal".to_string()
        };

        // 如果有序号，添加到标题后
        if let Some(index) = self.tab_index {
            SharedString::from(format!("{}({})", base_title, index))
        } else {
            SharedString::from(base_title)
        }
    }

    fn title_tooltip(&self, cx: &App) -> Option<SharedString> {
        if self.recording_playback_name.is_some() || self.session_log_name.is_some() {
            return None;
        }
        let terminal = self.terminal.read(cx);
        if terminal.live_connection_kind() != Some(TerminalConnectionKind::Local) {
            return None;
        }
        let display_title = self.title(cx);
        let cwd = terminal.current_working_dir().or_else(|| {
            self.local_working_dir
                .as_deref()
                .and_then(|path| path.to_str())
        });
        local_terminal_tooltip(display_title.as_ref(), terminal.title(), cwd)
    }

    fn icon(&self, cx: &App) -> Option<Icon> {
        // 连接设置里选了图标(SSH 自定义 / 扩展 Icon 字段)时优先展示它。
        if let Some(icon) = self.connection_icon.clone() {
            return Some(icon);
        }
        match self.connection_kind(cx) {
            TerminalConnectionKind::Serial => Some(IconName::SerialPort.color()),
            TerminalConnectionKind::Telnet => Some(IconName::SquareTerminalColor.color()),
            _ => Some(IconName::TerminalColor.color()),
        }
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn can_duplicate(&self, cx: &App) -> bool {
        self.duplicate_supported(cx)
    }

    fn on_activate(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.set_performance_tab_active(true);
        self.set_performance_pane_active(true);
        self.on_host_activated(cx);
    }

    fn on_deactivate(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.set_performance_tab_active(false);
        self.set_performance_pane_active(false);
    }

    fn duplicate(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Arc<dyn TabContentView>> {
        if !self.duplicate_supported(cx) {
            return None;
        }
        let source = self.duplicate_source_snapshot(cx)?;
        let profile_title = self.local_profile_title();
        let duplicate =
            cx.new(|cx| Self::new_from_duplicate_source(source, profile_title, window, cx));
        Some(Arc::new(duplicate))
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if self.requires_close_confirmation(cx) {
            return self.confirm_local_terminal_close(window, cx);
        }

        self.close_terminal_now(cx);
        Task::ready(true)
    }

    fn apply_title(&mut self, title: &str, _window: &mut Window, cx: &mut Context<Self>) {
        self.sync_broadcast_label(title, cx);
    }

    fn sidebar_contributions(&self, _cx: &App) -> Vec<SidebarContribution> {
        Vec::new()
    }
}
