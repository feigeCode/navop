rust_i18n::i18n!("locales", fallback = "en");

mod close_guard;
#[cfg(feature = "ui")]
mod editor_window;
mod file_policy;
mod language;

#[cfg(feature = "ui")]
pub use editor_window::open_remote_file_editor;

pub use close_guard::{CloseIntercept, decide_close_intercept};
pub use file_policy::{
    EditorMode, FilePolicy, LARGE_FILE_PLAIN_TEXT_THRESHOLD, MAX_EDITABLE_FILE_SIZE,
    decode_text_content, determine_file_policy,
};
pub use language::language_for_path;
