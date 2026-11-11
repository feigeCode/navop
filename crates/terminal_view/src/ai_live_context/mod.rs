//! 终端实时状态 → AI 资源作用域。
//!
//! 侧边栏内置 AI 每轮发送前都会重建资源上下文（见 `ai_chat_view` 的
//! `ResourceContextRefresher`），这里负责把终端「此刻的现场」投影成资源作用域：
//! 当前目录、是否运行全屏程序、选中了哪些内容、窗口尺寸、进程是否已退出。
//!
//! 状态只以作用域形式呈现，模型无需调用工具即可感知；完整屏幕内容仍然通过
//! `terminal.read` 按需拉取，避免每轮把整屏塞进提示词。

mod context;

use agent_runtime::{ResourceRef, ResourceScope};
use ai_chat_view::ContextAttachment;
use alacritty_terminal::term::TermMode;
use terminal::terminal::Terminal;

pub(crate) use context::{LiveContextInputs, build_live_resource_context};

/// 作用域键：当前工作目录。与 `connections.list_sessions` 共用同一个键，
/// 模型不必区分「资源池里的 cwd」和「侧栏里的 cwd」。
const SCOPE_WORKING_DIR: &str = public_mcp::registry::SESSION_CWD_SCOPE;
/// 作用域键：屏幕形态（普通 shell / 全屏交互程序）。
const SCOPE_SCREEN: &str = "screen";
/// 作用域键：终端选中内容摘要。
const SCOPE_SELECTION: &str = "selection";
/// 作用域键：终端窗口尺寸。
const SCOPE_SIZE: &str = "size";
/// 作用域键：终端进程状态。
const SCOPE_PROCESS: &str = "process";
/// 作用域键：连接上挂着的活动终端 id（SSH 终端）。
const SCOPE_LIVE_TERMINAL: &str = "terminal";
/// 作用域键：终端所属连接的资源 id。
const SCOPE_CONNECTION: &str = "connection";

/// 选中内容摘要的最大字符数，避免把整段输出塞进提示词。
const SELECTION_PREVIEW_MAX_CHARS: usize = 120;
/// 选中正文随消息附送时的字符上限；超出部分截断，让模型按需再读。
const ATTACHMENT_MAX_CHARS: usize = 8_000;
/// 包裹选中正文的标签；选中内容里可能含围栏或角括号，用标签界定最稳。
const ATTACHMENT_TAG: &str = "terminal_selection";
/// 摘要里代替换行的占位符，保证摘要始终是单行。
const NEWLINE_PLACEHOLDER: &str = "⏎";

/// 终端此刻的现场，供作用域投影与测试使用。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TerminalLiveState {
    working_dir: Option<String>,
    title: String,
    alternate_screen: bool,
    selection: Option<String>,
    rows: usize,
    cols: usize,
    exited: Option<i32>,
}

impl TerminalLiveState {
    /// 从活动终端抓取一次现场状态。
    pub(crate) fn from_terminal(terminal: &Terminal) -> Self {
        Self {
            working_dir: terminal.current_working_dir().map(str::to_string),
            title: terminal.title().to_string(),
            alternate_screen: terminal.mode().contains(TermMode::ALT_SCREEN),
            selection: terminal.selection_text(),
            rows: terminal.rows(),
            cols: terminal.cols(),
            exited: terminal.child_exited(),
        }
    }

    /// 投影成资源作用域；没有信息的维度直接省略，避免空作用域占提示词。
    pub(crate) fn scopes(&self) -> Vec<ResourceScope> {
        let mut scopes = Vec::new();
        if let Some(dir) = self.working_dir.as_deref().filter(|dir| !dir.is_empty()) {
            scopes.push(ResourceScope::new(
                SCOPE_WORKING_DIR,
                "Working directory",
                dir,
            ));
        }
        scopes.push(ResourceScope::new(
            SCOPE_SCREEN,
            "Screen",
            self.screen_summary(),
        ));
        if let Some(selection) = self
            .selection
            .as_deref()
            .filter(|text| !text.trim().is_empty())
        {
            scopes.push(ResourceScope::new(
                SCOPE_SELECTION,
                "Selection",
                selection_summary(selection),
            ));
        }
        scopes.push(ResourceScope::new(
            SCOPE_SIZE,
            "Size",
            format!("{} rows x {} cols", self.rows, self.cols),
        ));
        if let Some(code) = self.exited {
            scopes.push(ResourceScope::new(
                SCOPE_PROCESS,
                "Process",
                format!("exited with code {code}"),
            ));
        }
        scopes
    }

    fn screen_summary(&self) -> String {
        if !self.alternate_screen {
            return "normal shell".to_string();
        }
        let title = self.title.trim();
        if title.is_empty() {
            "full-screen program running (vi/less/htop); keys go to the app".to_string()
        } else {
            format!("full-screen program running ({title}); keys go to the app")
        }
    }
}

/// 把终端此刻的选中内容包成「随本条用户消息一起发送」的正文。
///
/// 只有存在非空选中时才返回；正文用标签包住并在标签上标明来源、行数与是否
/// 截断，模型既知道自己在看什么，也知道有没有看到全部。标签外的内容仍由
/// `terminal.read` 按需拉取，这里只带用户明确选中的部分。
pub(crate) fn selection_attachment(
    terminal: &Terminal,
    source_label: &str,
) -> Option<ContextAttachment> {
    let selection = terminal.selection_text()?;
    if selection.trim().is_empty() {
        return None;
    }
    let lines = selection.lines().count().max(1);
    let (text, truncated) = truncate_chars(&selection, ATTACHMENT_MAX_CHARS);
    let label = format!("{lines} lines of terminal selection");
    Some(ContextAttachment::new(
        label,
        wrap_selection(text, lines, truncated, source_label),
    ))
}

fn wrap_selection(text: &str, lines: usize, truncated: bool, source_label: &str) -> String {
    format!(
        "<{ATTACHMENT_TAG} source=\"{source}\" lines=\"{lines}\" truncated=\"{truncated}\">\n{body}\n</{ATTACHMENT_TAG}>",
        source = sanitize_attribute(source_label),
        body = text.trim_end(),
    )
}

/// 属性值只保留单行安全字符，避免标签被来源名里的引号截断。
fn sanitize_attribute(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '"' | '\'' | '<' | '>' | '\n' | '\r' => ' ',
            other => other,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// 按字符数截断，返回截断后的切片与是否发生了截断。
fn truncate_chars(text: &str, max_chars: usize) -> (&str, bool) {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => (&text[..index], true),
        None => (text, false),
    }
}

/// 抓取终端现场并投影成作用域。
pub(crate) fn live_resource_scopes(terminal: &Terminal) -> Vec<ResourceScope> {
    TerminalLiveState::from_terminal(terminal).scopes()
}

/// 把实时作用域写到资源上，替换上一次的实时作用域。
///
/// 只替换本模块负责的键，宿主自己写入的作用域（如连接信息）保持不变。
pub(crate) fn apply_live_scopes(resource: &mut ResourceRef, scopes: Vec<ResourceScope>) {
    let live_keys = [
        SCOPE_WORKING_DIR,
        SCOPE_SCREEN,
        SCOPE_SELECTION,
        SCOPE_SIZE,
        SCOPE_PROCESS,
    ];
    resource
        .scopes
        .retain(|scope| !live_keys.contains(&scope.key.as_str()));
    for scope in scopes {
        resource.set_scope(scope);
    }
}

fn selection_summary(selection: &str) -> String {
    let lines = selection.lines().count().max(1);
    let chars = selection.chars().count();
    let preview = selection_preview(selection);
    format!("{lines} lines / {chars} chars: {preview}; full text is attached to this message")
}

fn selection_preview(selection: &str) -> String {
    let mut preview = String::new();
    let mut truncated = false;
    for ch in selection.chars() {
        if preview.chars().count() >= SELECTION_PREVIEW_MAX_CHARS {
            truncated = true;
            break;
        }
        if ch == '\n' {
            preview.push_str(NEWLINE_PLACEHOLDER);
        } else if ch.is_whitespace() && ch != ' ' {
            preview.push(' ');
        } else {
            preview.push(ch);
        }
    }
    let preview = preview.trim().to_string();
    if truncated {
        format!("{preview}…")
    } else {
        preview
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> TerminalLiveState {
        TerminalLiveState {
            working_dir: Some("/Users/demo/project".to_string()),
            title: String::new(),
            alternate_screen: false,
            selection: None,
            rows: 40,
            cols: 120,
            exited: None,
        }
    }

    fn scope_value(scopes: &[ResourceScope], key: &str) -> Option<String> {
        scopes
            .iter()
            .find(|scope| scope.key == key)
            .map(|scope| scope.value.clone())
    }

    #[test]
    fn normal_shell_reports_directory_size_and_screen() {
        let scopes = state().scopes();

        assert_eq!(
            scope_value(&scopes, SCOPE_WORKING_DIR).as_deref(),
            Some("/Users/demo/project")
        );
        assert_eq!(
            scope_value(&scopes, SCOPE_SCREEN).as_deref(),
            Some("normal shell")
        );
        assert_eq!(
            scope_value(&scopes, SCOPE_SIZE).as_deref(),
            Some("40 rows x 120 cols")
        );
        assert!(scope_value(&scopes, SCOPE_SELECTION).is_none());
        assert!(scope_value(&scopes, SCOPE_PROCESS).is_none());
    }

    #[test]
    fn alternate_screen_reports_title_and_hints_keys_are_consumed() {
        let mut state = state();
        state.alternate_screen = true;
        state.title = "vim: src/main.rs".to_string();

        let screen = scope_value(&state.scopes(), SCOPE_SCREEN).expect("screen scope");
        assert!(screen.contains("vim: src/main.rs"));
        assert!(screen.contains("keys go to the app"));
    }

    #[test]
    fn selection_is_summarized_into_a_single_line() {
        let mut state = state();
        state.selection = Some("fn main() {\n    println!(\"hi\");\n}".to_string());

        let selection = scope_value(&state.scopes(), SCOPE_SELECTION).expect("selection scope");
        assert!(selection.starts_with("3 lines / "));
        assert!(selection.contains(NEWLINE_PLACEHOLDER));
        assert!(!selection.contains('\n'));
    }

    #[test]
    fn long_selection_is_truncated() {
        let mut state = state();
        state.selection = Some("x".repeat(SELECTION_PREVIEW_MAX_CHARS + 50));

        let selection = scope_value(&state.scopes(), SCOPE_SELECTION).expect("selection scope");
        assert!(selection.contains('…'));
        // 预览截断后仍要留出「正文已随消息附送」这句话的位置。
        assert!(selection.chars().count() < SELECTION_PREVIEW_MAX_CHARS + 90);
    }

    #[test]
    fn exited_process_is_reported() {
        let mut state = state();
        state.exited = Some(1);

        assert_eq!(
            scope_value(&state.scopes(), SCOPE_PROCESS).as_deref(),
            Some("exited with code 1")
        );
    }

    #[test]
    fn apply_live_scopes_replaces_only_live_keys() {
        let mut resource =
            ResourceRef::new("terminal-1", agent_runtime::ResourceKind::Terminal, "demo")
                .with_scope(ResourceScope::new("screen", "Screen", "stale"))
                .with_scope(ResourceScope::new("host", "Host", "10.0.0.1"));

        apply_live_scopes(&mut resource, state().scopes());

        assert_eq!(
            scope_value(&resource.scopes, "host").as_deref(),
            Some("10.0.0.1")
        );
        assert_eq!(
            scope_value(&resource.scopes, SCOPE_SCREEN).as_deref(),
            Some("normal shell")
        );
    }

    #[test]
    fn selection_summary_points_at_attached_body() {
        let mut live = state();
        live.selection = Some("echo one\necho two".to_string());

        let summary = scope_value(&live.scopes(), SCOPE_SELECTION).expect("selection scope");

        assert!(summary.contains("2 lines"), "{summary}");
        assert!(summary.contains("attached to this message"), "{summary}");
    }

    #[test]
    fn wrapped_selection_keeps_metadata_on_the_tag() {
        let body = wrap_selection("fn main() {}", 3, true, "本地终端 \"demo\"");

        assert!(
            body.starts_with("<terminal_selection source=\"本地终端  demo\""),
            "{body}"
        );
        assert!(body.contains("lines=\"3\""), "{body}");
        assert!(body.contains("truncated=\"true\""), "{body}");
        assert!(body.ends_with("</terminal_selection>"), "{body}");
    }

    #[test]
    fn truncation_marks_only_when_over_limit() {
        assert_eq!(truncate_chars("abc", 4), ("abc", false));

        let (text, truncated) = truncate_chars("abcdef", 3);

        assert_eq!(text, "abc");
        assert!(truncated);
    }

    #[test]
    fn truncation_does_not_split_multibyte_chars() {
        let (text, truncated) = truncate_chars("中文选区", 2);

        assert_eq!(text, "中文");
        assert!(truncated);
    }
}
