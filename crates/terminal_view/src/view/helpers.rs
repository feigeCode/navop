use super::*;

pub(super) const REMOTE_CLIPBOARD_IMAGE_DIR: &str = "/tmp";
pub(super) const REMOTE_CLIPBOARD_IMAGE_PREFIX: &str = "onetcli-paste";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WrappedLineSegment {
    pub(super) text: String,
    pub(super) wraps_to_next: bool,
}

impl WrappedLineSegment {
    pub(super) fn new(text: impl Into<String>, wraps_to_next: bool) -> Self {
        Self {
            text: text.into(),
            wraps_to_next,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AddonLineText {
    pub(super) text: String,
    pub(super) column: usize,
    pub(super) screen_line: usize,
}

pub(super) const DEFAULT_CELL_WIDTH: Pixels = px(8.0);
pub(super) const DEFAULT_COLS: usize = 80;
pub(super) const DEFAULT_ROWS: usize = 24;
pub(super) const TERMINAL_RESET_FONT_SIZE: f32 = 15.0;
pub(super) const HISTORY_SUGGESTION_LIMIT: usize = 6;
pub(super) fn shell_escape(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_alphanumeric() || c == '/' || c == '.' || c == '-' || c == '_')
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

pub(super) fn first_wrapped_grid_line(
    current: i32,
    min_line: i32,
    wraps_to_next: impl Fn(i32) -> bool,
) -> i32 {
    let mut line = current;
    while line > min_line && wraps_to_next(line - 1) {
        line -= 1;
    }
    line
}

pub(super) fn last_wrapped_grid_line(
    current: i32,
    max_line: i32,
    wraps_to_next: impl Fn(i32) -> bool,
) -> i32 {
    let mut line = current;
    while line < max_line && wraps_to_next(line) {
        line += 1;
    }
    line
}

pub(super) fn wrapped_addon_line_text(
    lines: &[WrappedLineSegment],
    current_line: usize,
    column: usize,
    first_screen_line: usize,
) -> AddonLineText {
    debug_assert!(
        lines
            .iter()
            .take(lines.len().saturating_sub(1))
            .all(|line| line.wraps_to_next)
    );
    let prefix_width = lines
        .iter()
        .take(current_line)
        .map(|line| line.text.chars().count())
        .sum::<usize>();
    let text = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<String>();

    AddonLineText {
        text,
        column: prefix_width + column,
        screen_line: first_screen_line,
    }
}

pub(super) fn normalize_paste_line_endings(text: &str) -> Cow<'_, str> {
    if text.contains('\r') {
        Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

pub(super) fn terminal_paste_bytes(text: &str, mode: TermMode) -> Vec<u8> {
    let text = normalize_paste_line_endings(text);
    if mode.contains(TermMode::BRACKETED_PASTE) {
        format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', "")).into_bytes()
    } else {
        match text {
            Cow::Borrowed(text) => text.as_bytes().to_vec(),
            Cow::Owned(text) => text.into_bytes(),
        }
    }
}

pub(super) fn should_direct_paste_on_right_click(enabled: bool, button: MouseButton) -> bool {
    enabled && button == MouseButton::Right
}

pub(super) fn remote_clipboard_image_path(format: ImageFormat, timestamp_millis: u128) -> String {
    format!(
        "{REMOTE_CLIPBOARD_IMAGE_DIR}/{REMOTE_CLIPBOARD_IMAGE_PREFIX}-{timestamp_millis}.{}",
        image_format_extension(format)
    )
}

pub(super) fn current_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub(super) fn image_format_extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
        ImageFormat::Gif => "gif",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Tiff => "tiff",
        ImageFormat::Ico => "ico",
        ImageFormat::Svg => "svg",
        ImageFormat::Pnm => "pnm",
    }
}

pub(super) fn clipboard_image_from_item(item: &ClipboardItem) -> Option<Image> {
    item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::Image(image) => Some(image.clone()),
        ClipboardEntry::ExternalPaths(paths) => paths
            .paths()
            .iter()
            .find_map(|path| image_from_local_path(path)),
        ClipboardEntry::String(_) => None,
    })
}

pub(super) fn should_upload_clipboard_image_to_remote_cli(
    paste_image_upload_enabled: bool,
    connection_kind: TerminalConnectionKind,
    _mode: TermMode,
) -> bool {
    paste_image_upload_enabled && connection_kind == TerminalConnectionKind::Ssh
}
