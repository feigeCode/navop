//! 终端会话级设置覆盖。
//!
//! 侧边栏里的终端设置只影响当前这个终端标签：改动写进 [`TerminalSessionOverrides`]，
//! 既不写 `AppSettings`，也不落盘到 `terminal-settings.json`。
//! 全局默认值由「设置 → 终端」维护，新开的标签按全局默认值初始化。
//!
//! 这样同一窗口里两个终端标签可以各自有不同的字号、主题和粘贴行为，
//! 关掉标签后覆盖随会话一起消失。

use crate::settings::TerminalSettings;
use one_core::settings::AppSettings;

/// 侧边栏可改、且只对当前会话生效的终端设置。
///
/// 每个字段都是 `Option<T>`：`None` 表示跟随全局默认值，`Some` 表示本会话被改过。
/// 用 `Option` 而不是直接存整份快照，是为了在全局默认值变化时，
/// 没被改过的项能继续跟着变（见 [`Self::merged_into`]）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TerminalSessionOverrides {
    pub font_size: Option<f32>,
    pub font_family: Option<String>,
    pub theme: Option<String>,
    pub scrollback_lines: Option<usize>,
    pub auto_copy: Option<bool>,
    pub enable_autocomplete: Option<bool>,
    pub show_suggestion_popup: Option<bool>,
    pub suggestion_popup_opacity: Option<f32>,
    pub middle_click_paste: Option<bool>,
    pub right_click_paste: Option<bool>,
    pub paste_image_upload: Option<bool>,
    pub sync_path_with_terminal: Option<bool>,
    pub cursor_blink: Option<bool>,
    pub confirm_multiline_paste: Option<bool>,
    pub confirm_high_risk_command: Option<bool>,
    pub selection_highlight: Option<bool>,
    pub show_line_timestamps: Option<bool>,
    pub show_line_numbers: Option<bool>,
    pub vim_scroll_to_arrow_keys: Option<bool>,
}

impl TerminalSessionOverrides {
    /// 当前会话是否被改过任何一项。
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// 已经脱离全局默认值的项数，用于侧边栏展示「N 项会话覆盖」。
    pub fn overridden_count(&self) -> usize {
        usize::from(self.font_size.is_some())
            + usize::from(self.font_family.is_some())
            + usize::from(self.theme.is_some())
            + usize::from(self.scrollback_lines.is_some())
            + usize::from(self.auto_copy.is_some())
            + usize::from(self.enable_autocomplete.is_some())
            + usize::from(self.show_suggestion_popup.is_some())
            + usize::from(self.suggestion_popup_opacity.is_some())
            + usize::from(self.middle_click_paste.is_some())
            + usize::from(self.right_click_paste.is_some())
            + usize::from(self.paste_image_upload.is_some())
            + usize::from(self.sync_path_with_terminal.is_some())
            + usize::from(self.cursor_blink.is_some())
            + usize::from(self.confirm_multiline_paste.is_some())
            + usize::from(self.confirm_high_risk_command.is_some())
            + usize::from(self.selection_highlight.is_some())
            + usize::from(self.show_line_timestamps.is_some())
            + usize::from(self.show_line_numbers.is_some())
            + usize::from(self.vim_scroll_to_arrow_keys.is_some())
    }

    /// 放弃全部会话覆盖，回到全局默认值。
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// 把覆盖叠加到全局默认设置上，得到本次会话实际生效的设置。
    pub fn merged_into(&self, base: &TerminalSettings) -> TerminalSettings {
        let mut settings = base.clone();
        if let Some(value) = self.font_size {
            settings.font_size = value;
        }
        if let Some(value) = self.font_family.as_ref() {
            settings.font_family = value.clone();
        }
        if let Some(value) = self.theme.as_ref() {
            settings.theme = value.clone();
        }
        if let Some(value) = self.scrollback_lines {
            settings.scrollback_lines = AppSettings::normalize_terminal_scrollback_lines(value);
        }
        if let Some(value) = self.auto_copy {
            settings.auto_copy = value;
        }
        if let Some(value) = self.enable_autocomplete {
            settings.enable_autocomplete = value;
        }
        if let Some(value) = self.show_suggestion_popup {
            settings.show_suggestion_popup = value;
        }
        if let Some(value) = self.suggestion_popup_opacity {
            settings.suggestion_popup_opacity =
                AppSettings::normalize_terminal_suggestion_popup_opacity(value);
        }
        if let Some(value) = self.middle_click_paste {
            settings.middle_click_paste = value;
        }
        if let Some(value) = self.right_click_paste {
            settings.right_click_paste = value;
        }
        if let Some(value) = self.paste_image_upload {
            settings.paste_image_upload = value;
        }
        if let Some(value) = self.sync_path_with_terminal {
            settings.sync_path_with_terminal = value;
        }
        if let Some(value) = self.cursor_blink {
            settings.cursor_blink = value;
        }
        if let Some(value) = self.confirm_multiline_paste {
            settings.confirm_multiline_paste = value;
        }
        if let Some(value) = self.confirm_high_risk_command {
            settings.confirm_high_risk_command = value;
        }
        if let Some(value) = self.selection_highlight {
            settings.selection_highlight = value;
        }
        if let Some(value) = self.show_line_timestamps {
            settings.show_line_timestamps = value;
        }
        if let Some(value) = self.show_line_numbers {
            settings.show_line_numbers = value;
        }
        if let Some(value) = self.vim_scroll_to_arrow_keys {
            settings.vim_scroll_to_arrow_keys = value;
        }
        settings
    }
}

/// 记一条候选覆盖后是否真的发生了变化；没变化就不必重新应用整份设置。
pub fn apply_override<T>(
    overrides: &mut TerminalSessionOverrides,
    updater: impl FnOnce(&mut TerminalSessionOverrides) -> Option<T>,
) -> bool {
    let mut next = overrides.clone();
    if updater(&mut next).is_none() || next == *overrides {
        return false;
    }
    *overrides = next;
    true
}

/// 说明：自定义高亮规则（`TerminalHighlightRule`）属于用户维护的内容，
/// 不走会话覆盖，仍按原链路写入 `terminal-settings.json`。

#[cfg(test)]
mod tests {
    use super::*;

    fn base_settings() -> TerminalSettings {
        TerminalSettings::default()
    }

    #[test]
    fn empty_overrides_keep_global_values() {
        let base = base_settings();
        let overrides = TerminalSessionOverrides::default();
        assert!(overrides.is_empty());
        assert_eq!(0, overrides.overridden_count());
        assert_eq!(base, overrides.merged_into(&base));
    }

    #[test]
    fn override_only_replaces_the_touched_field() {
        let base = base_settings();
        let mut overrides = TerminalSessionOverrides::default();
        overrides.font_size = Some(base.font_size + 4.0);
        overrides.cursor_blink = Some(!base.cursor_blink);

        let merged = overrides.merged_into(&base);
        assert_eq!(base.font_size + 4.0, merged.font_size);
        assert_eq!(!base.cursor_blink, merged.cursor_blink);
        // 其余字段仍然跟随全局默认值。
        assert_eq!(base.theme, merged.theme);
        assert_eq!(base.scrollback_lines, merged.scrollback_lines);
        assert_eq!(2, overrides.overridden_count());
        assert!(!overrides.is_empty());
    }

    #[test]
    fn clear_returns_to_global_values() {
        let base = base_settings();
        let mut overrides = TerminalSessionOverrides::default();
        overrides.theme = Some("One Dark".to_string());
        overrides.auto_copy = Some(!base.auto_copy);
        overrides.clear();

        assert!(overrides.is_empty());
        assert_eq!(base, overrides.merged_into(&base));
    }

    #[test]
    fn merged_settings_stay_independent_per_session() {
        let base = base_settings();
        let mut first = TerminalSessionOverrides::default();
        first.font_size = Some(18.0);
        let second = TerminalSessionOverrides::default();

        assert_eq!(18.0, first.merged_into(&base).font_size);
        assert_eq!(base.font_size, second.merged_into(&base).font_size);
        // 覆盖不会回写到全局默认值。
        assert_eq!(base_settings().font_size, base.font_size);
    }

    #[test]
    fn overrides_are_normalized_when_merged() {
        let base = base_settings();
        let mut overrides = TerminalSessionOverrides::default();
        overrides.scrollback_lines = Some(usize::MAX);
        overrides.suggestion_popup_opacity = Some(-1.0);

        let merged = overrides.merged_into(&base);
        assert_eq!(
            AppSettings::normalize_terminal_scrollback_lines(usize::MAX),
            merged.scrollback_lines
        );
        assert_eq!(
            AppSettings::normalize_terminal_suggestion_popup_opacity(-1.0),
            merged.suggestion_popup_opacity
        );
    }

    #[test]
    fn apply_override_reports_actual_changes_only() {
        let mut overrides = TerminalSessionOverrides::default();
        assert!(apply_override(&mut overrides, |o| {
            o.font_size = Some(15.0);
            Some(())
        }));
        assert!(!apply_override(&mut overrides, |o| {
            o.font_size = Some(15.0);
            Some(())
        }));
        assert!(!apply_override(&mut overrides, |_| None::<()>));
        assert_eq!(Some(15.0), overrides.font_size);
    }
}
