rust_i18n::i18n!("locales", fallback = "en");

mod close_guard;
mod editor_window;
mod external_edit_controller;
mod external_editor;
mod external_editor_confirmation;
mod external_launcher;
mod external_rules;
mod external_session;
mod file_policy;
mod language;

pub use editor_window::{open_remote_file_editor, refresh_keybindings};
pub use external_editor::{
    ExternalEditorOpenRequest, external_editor_menu_label, external_editors_for_file,
    open_remote_file_external_editor,
};
pub use external_launcher::{
    LaunchTemplateContext, launch_external_editor, render_args, resolve_editor_program,
    resolve_program_with_env, validate_program,
};
pub use external_rules::{
    editor_matches_file, editor_supports_current_platform, matches_file_mask, matching_editors,
};
pub use external_session::{
    ExternalEditSession, RemoteFileSnapshot, UploadDecision, decide_upload, sanitized_file_name,
    session_temp_file,
};

pub use close_guard::{
    CloseIntercept, active_index_after_close, active_index_after_open, decide_close_intercept,
    find_tab_index, has_dirty_tabs,
};
pub use file_policy::{
    EditorMode, FilePolicy, LARGE_FILE_PLAIN_TEXT_THRESHOLD, MAX_EDITABLE_FILE_SIZE,
    decode_text_content, determine_file_policy,
};
pub use language::language_for_path;
