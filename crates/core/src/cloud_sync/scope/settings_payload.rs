//! 设置类 Singleton 的载荷抽取、应用与字段级三方合并
//!
//! 设置类数据没有 `updated_at`、没有逐条 ID，因此整组只对应一条同步记录（Singleton），
//! 变更检测靠 checksum，冲突处理靠**字段级三方合并**（Q8）。
//!
//! ## 为什么用字段白名单
//! `AppSettings` 里混着大量**不该跨设备同步**的字段：窗口尺寸、主密钥相关开关、
//! 同步自身的配置（`sync_enabled` / `sync_provider` / `sync_scope` / `personal_sync`）、
//! 日志路径、更新跳过版本等。整包序列化会带来两类问题：
//! - 设备相关项被强行拉平（窗口尺寸在多显示器下互相打架）
//! - 同步配置自身被同步造成回环（见设计文档 R7 / Q9）
//! 因此必须显式白名单，逐字段审定。

use serde_json::{Map, Value};

use crate::settings::AppSettings;

/// 可同步的设置分组
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsGroup {
    General,
    Database,
    Terminal,
    Agent,
    Mcp,
    Keybindings,
}

impl SettingsGroup {
    pub const ALL: &'static [SettingsGroup] = &[
        SettingsGroup::General,
        SettingsGroup::Database,
        SettingsGroup::Terminal,
        SettingsGroup::Agent,
        SettingsGroup::Mcp,
        SettingsGroup::Keybindings,
    ];

    pub fn data_type(self) -> &'static str {
        match self {
            Self::General => crate::cloud_sync::data_type::SETTINGS_GENERAL,
            Self::Database => crate::cloud_sync::data_type::SETTINGS_DATABASE,
            Self::Terminal => crate::cloud_sync::data_type::SETTINGS_TERMINAL,
            Self::Agent => crate::cloud_sync::data_type::SETTINGS_AGENT,
            Self::Mcp => crate::cloud_sync::data_type::SETTINGS_MCP,
            Self::Keybindings => crate::cloud_sync::data_type::SETTINGS_KEYBINDINGS,
        }
    }

    pub fn from_data_type(data_type: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|g| g.data_type() == data_type)
    }

    /// 载荷里记录的分组名（与 `data_type` 一一对应，用于反序列化时反查）
    pub fn type_name(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Database => "database",
            Self::Terminal => "terminal",
            Self::Agent => "agent",
            Self::Mcp => "mcp",
            Self::Keybindings => "keybindings",
        }
    }

    pub fn from_type_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|g| g.type_name() == name)
    }

    /// 该分组参与同步的 `AppSettings` 字段名白名单
    pub fn fields(self) -> &'static [&'static str] {
        match self {
            Self::General => &[
                "locale",
                "theme_mode",
                "auto_switch_theme",
                "light_theme",
                "dark_theme",
                "window_opacity",
                "custom_accent_enabled",
                "custom_accent_color",
                "font_family",
                "font_size",
                "ui_scale_percent",
                "custom_fonts",
                "auto_update",
                "startup_default_page",
                "home_connection_layout",
                "connection_sort_order",
                "connection_sidebar_expanded",
                "close_button_behavior",
            ],
            Self::Database => &[
                "database_open_mode",
                "large_text_cell_editor_open_mode",
                "table_row_height",
                "table_view_mode",
                "table_show_column_comment",
                "sql_query_max_rows",
                "sql_export_rows_per_statement",
                "sql_format",
                "enable_sql_auto_save",
                "sql_auto_save_interval",
                "sql_editor_font_family",
                "sql_editor_font_size",
                "sql_editor_hover_enabled",
                "table_preview_font_family",
            ],
            // 终端侧仅同步 settings.json 里的 terminal_* 字段。
            // `terminal-settings.json` 的 custom_highlights 需要
            // terminal_view 的内置规则清单才能剥离，且 one-core 不能反向依赖
            // terminal_view，此处暂不纳入（已知缺口）。
            Self::Terminal => &[
                "terminal_font_family",
                "terminal_font_size",
                "terminal_scrollback_lines",
                "terminal_auto_copy",
                "terminal_enable_autocomplete",
                "terminal_show_suggestion_popup",
                "terminal_suggestion_popup_opacity",
                "terminal_middle_click_paste",
                "terminal_right_click_paste",
                "terminal_paste_image_upload",
                "terminal_sync_path_with_terminal",
                "terminal_theme",
                "terminal_cursor_blink",
                "terminal_confirm_multiline_paste",
                "terminal_confirm_high_risk_command",
                "terminal_auto_session_logging",
                "terminal_selection_highlight",
                "terminal_show_timestamps",
                "terminal_show_line_numbers",
            ],
            // Q7：Agent 设置包含 Agent 侧工具暴露
            Self::Agent => &["ai_chat", "tool_exposure"],
            // Q7：MCP 设置包含 MCP 侧服务选项
            Self::Mcp => &["mcp"],
            Self::Keybindings => &["custom_keybindings"],
        }
    }

    /// 是否按「子键」展开后逐项合并。
    ///
    /// 快捷键是 `HashMap<action_id, Vec<String>>`：A 机改了 A 快捷键、B 机改了 B 快捷键
    /// 时，整体二选一会丢一边；按 action_id 展开后能自动合并，收益最大。
    /// 其余分组的字段（如 `sql_format`）作为原子值处理，避免半截合并出非法组合。
    pub fn merge_per_subkey(self) -> bool {
        matches!(self, Self::Keybindings)
    }
}

/// 从 `AppSettings` 抽取某分组的白名单字段
pub fn extract_payload(group: SettingsGroup, settings: &AppSettings) -> serde_json::Result<Value> {
    let all = serde_json::to_value(settings)?;
    let mut payload = Map::new();
    for field in group.fields() {
        if let Some(value) = all.get(*field) {
            payload.insert((*field).to_string(), value.clone());
        }
    }
    Ok(Value::Object(payload))
}

/// 把远端载荷合并回 `AppSettings`
///
/// 只写白名单内的字段，其余保持本地不变。
pub fn apply_payload(
    group: SettingsGroup,
    settings: &mut AppSettings,
    payload: &Value,
) -> serde_json::Result<()> {
    let mut all = serde_json::to_value(&*settings)?;
    if let (Some(target), Some(source)) = (all.as_object_mut(), payload.as_object()) {
        for field in group.fields() {
            if let Some(value) = source.get(*field) {
                target.insert((*field).to_string(), value.clone());
            }
        }
    }
    *settings = serde_json::from_value(all)?;
    Ok(())
}

/// 单个字段的冲突
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsFieldConflict {
    pub field: String,
    pub local: Value,
    pub remote: Value,
}

/// 三方合并结果
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsMerge {
    /// 合并后的完整载荷（冲突字段取本地值，交由用户最终裁决前先保持可用）
    pub merged: Value,
    /// 冲突字段；为空表示可以静默自动合并
    pub conflicts: Vec<SettingsFieldConflict>,
}

impl SettingsMerge {
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// 字段级三方合并（Q8）
///
/// - 只有一边变 → 采用该边
/// - 两边都变且值相同 → 采用该值
/// - 两边都变且值不同 → 记为冲突字段
/// - base 缺失（清库 / 首次 / 跨通道切换）→ **降级为整体采用远端**，绝不静默丢数据（R14）
pub fn merge_three_way(
    group: SettingsGroup,
    base: Option<&Value>,
    local: &Value,
    remote: &Value,
) -> SettingsMerge {
    let Some(base) = base else {
        return SettingsMerge {
            merged: remote.clone(),
            conflicts: Vec::new(),
        };
    };

    let base_map = flatten(group, base);
    let local_map = flatten(group, local);
    let remote_map = flatten(group, remote);

    let mut merged = Map::new();
    let mut conflicts = Vec::new();

    let mut keys: Vec<&String> = base_map
        .keys()
        .chain(local_map.keys())
        .chain(remote_map.keys())
        .collect();
    keys.sort();
    keys.dedup();

    for key in keys {
        let base_value = base_map.get(key).cloned().unwrap_or(Value::Null);
        let local_value = local_map.get(key).cloned().unwrap_or(Value::Null);
        let remote_value = remote_map.get(key).cloned().unwrap_or(Value::Null);

        let local_changed = local_value != base_value;
        let remote_changed = remote_value != base_value;

        match (local_changed, remote_changed) {
            (true, true) if local_value == remote_value => {
                merged.insert(key.clone(), local_value);
            }
            (true, true) => {
                merged.insert(key.clone(), local_value.clone());
                conflicts.push(SettingsFieldConflict {
                    field: key.clone(),
                    local: local_value,
                    remote: remote_value,
                });
            }
            (true, false) => {
                merged.insert(key.clone(), local_value);
            }
            (false, true) => {
                merged.insert(key.clone(), remote_value);
            }
            (false, false) => {
                merged.insert(key.clone(), base_value);
            }
        }
    }

    SettingsMerge {
        merged: unflatten(group, merged),
        conflicts,
    }
}

/// 展平成 `字段名` 或 `字段名::子键` 的可合并单元
fn flatten(group: SettingsGroup, value: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(object) = value.as_object() else {
        return out;
    };
    for (field, field_value) in object {
        if group.merge_per_subkey() {
            if let Some(nested) = field_value.as_object() {
                for (sub, sub_value) in nested {
                    out.insert(format!("{field}::{sub}"), sub_value.clone());
                }
                continue;
            }
        }
        out.insert(field.clone(), field_value.clone());
    }
    out
}

/// `flatten` 的逆运算
fn unflatten(group: SettingsGroup, flat: Map<String, Value>) -> Value {
    if !group.merge_per_subkey() {
        return Value::Object(flat);
    }
    let mut out = Map::new();
    for (key, value) in flat {
        match key.split_once("::") {
            Some((field, sub)) => {
                let entry = out
                    .entry(field.to_string())
                    .or_insert_with(|| Value::Object(Map::new()));
                if let Some(object) = entry.as_object_mut() {
                    object.insert(sub.to_string(), value);
                }
            }
            None => {
                out.insert(key, value);
            }
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn general() -> SettingsGroup {
        SettingsGroup::General
    }

    #[test]
    fn disjoint_field_changes_merge_silently() {
        let base = json!({"locale": "zh-CN", "font_size": 14});
        let local = json!({"locale": "en", "font_size": 14});
        let remote = json!({"locale": "zh-CN", "font_size": 18});

        let merge = merge_three_way(general(), Some(&base), &local, &remote);

        assert!(merge.is_clean());
        assert_eq!(json!({"locale": "en", "font_size": 18}), merge.merged);
    }

    #[test]
    fn same_field_changed_on_both_sides_is_a_conflict() {
        let base = json!({"font_size": 14});
        let local = json!({"font_size": 16});
        let remote = json!({"font_size": 20});

        let merge = merge_three_way(general(), Some(&base), &local, &remote);

        assert_eq!(1, merge.conflicts.len());
        assert_eq!("font_size", merge.conflicts[0].field);
    }

    #[test]
    fn identical_changes_are_not_conflicts() {
        let base = json!({"font_size": 14});
        let local = json!({"font_size": 16});
        let remote = json!({"font_size": 16});

        let merge = merge_three_way(general(), Some(&base), &local, &remote);

        assert!(merge.is_clean());
        assert_eq!(json!({"font_size": 16}), merge.merged);
    }

    #[test]
    fn missing_base_falls_back_to_remote_without_dropping_data() {
        let local = json!({"font_size": 16});
        let remote = json!({"font_size": 20, "locale": "en"});

        let merge = merge_three_way(general(), None, &local, &remote);

        assert!(merge.is_clean());
        assert_eq!(remote, merge.merged);
    }

    #[test]
    fn keybindings_merge_per_action_id() {
        let base =
            json!({"custom_keybindings": {"app.quit": ["ctrl-q"], "sql.run_query": ["ctrl-r"]}});
        let local =
            json!({"custom_keybindings": {"app.quit": ["ctrl-w"], "sql.run_query": ["ctrl-r"]}});
        let remote =
            json!({"custom_keybindings": {"app.quit": ["ctrl-q"], "sql.run_query": ["ctrl-e"]}});

        let merge = merge_three_way(SettingsGroup::Keybindings, Some(&base), &local, &remote);

        assert!(merge.is_clean(), "不同 action 的修改应自动合并");
        assert_eq!(
            json!({"custom_keybindings": {"app.quit": ["ctrl-w"], "sql.run_query": ["ctrl-e"]}}),
            merge.merged
        );
    }

    #[test]
    fn payload_round_trip_only_touches_whitelisted_fields() {
        use crate::settings::AppSettings;

        let settings = AppSettings::default();
        let payload = extract_payload(general(), &settings).expect("extract");
        assert!(payload.get("font_size").is_some());
        // 同步配置自身绝不能出现在载荷里（R7 / Q9）
        assert!(payload.get("sync_scope").is_none());
        assert!(payload.get("sync_provider").is_none());
        assert!(payload.get("personal_sync").is_none());

        let mut applied = settings.clone();
        let mut patched = payload.clone();
        patched["font_size"] = json!(21);
        apply_payload(general(), &mut applied, &patched).expect("apply");
        assert_eq!(21.0, applied.font_size);
    }

    #[test]
    fn every_settings_data_type_round_trips() {
        for group in SettingsGroup::ALL {
            let parsed = SettingsGroup::from_data_type(group.data_type())
                .unwrap_or_else(|| panic!("data_type 未注册: {}", group.data_type()));
            assert_eq!(*group, parsed);
            assert!(!group.fields().is_empty());
        }
    }
}
