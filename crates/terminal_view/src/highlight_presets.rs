use crate::settings::TerminalHighlightRule;
use rust_i18n::t;

#[derive(Clone, Debug)]
pub struct HighlightPreset {
    pub id: &'static str,
    pub title_key: &'static str,
    pub description_key: &'static str,
    pub rules: Vec<HighlightPresetRule>,
}

#[derive(Clone, Debug)]
pub struct HighlightPresetRule {
    pub id: String,
    pub pattern: &'static str,
    pub foreground: Option<&'static str>,
    pub background: Option<&'static str>,
    pub priority: u8,
    pub note_key: &'static str,
}

impl HighlightPresetRule {
    fn to_terminal_rule(&self) -> TerminalHighlightRule {
        TerminalHighlightRule {
            id: self.id.clone(),
            enabled: true,
            pattern: self.pattern.to_string(),
            foreground: self.foreground.map(str::to_string),
            background: self.background.map(str::to_string),
            priority: self.priority,
            note: t!(self.note_key).to_string(),
        }
    }
}

pub fn builtin_highlight_rules() -> Vec<TerminalHighlightRule> {
    builtin_highlight_presets()
        .into_iter()
        .flat_map(|preset| {
            preset
                .rules
                .into_iter()
                .map(|rule| rule.to_terminal_rule())
                .collect::<Vec<_>>()
        })
        .collect()
}

pub fn builtin_highlight_presets() -> Vec<HighlightPreset> {
    vec![
        HighlightPreset {
            id: "linux_permissions",
            title_key: "CustomHighlight.presets.linux_permissions.title",
            description_key: "CustomHighlight.presets.linux_permissions.description",
            rules: vec![
                preset_rule(
                    "linux_permissions",
                    "root",
                    r"\b(root|sudo|su)\b",
                    Some("#f87171"),
                    None,
                    54,
                    "CustomHighlight.presets.linux_permissions.rules.root",
                ),
                preset_rule(
                    "linux_permissions",
                    "owner",
                    r"\b(chmod|chown|chgrp)\b",
                    Some("#fbbf24"),
                    None,
                    48,
                    "CustomHighlight.presets.linux_permissions.rules.owner",
                ),
                preset_rule(
                    "linux_permissions",
                    "mode",
                    r"[dlbcps\-][rwx\-sStT]{9}",
                    Some("#60a5fa"),
                    None,
                    44,
                    "CustomHighlight.presets.linux_permissions.rules.mode",
                ),
            ],
        },
        HighlightPreset {
            id: "status_signals",
            title_key: "CustomHighlight.presets.status_signals.title",
            description_key: "CustomHighlight.presets.status_signals.description",
            rules: vec![
                preset_rule(
                    "status_signals",
                    "success",
                    r"\b(SUCCESS|OK|PASSED|DONE|READY|HEALTHY)\b",
                    Some("#4ade80"),
                    None,
                    50,
                    "CustomHighlight.presets.status_signals.rules.success",
                ),
                preset_rule(
                    "status_signals",
                    "warning",
                    r"\b(WARN|WARNING|CAUTION|RETRY|DEPRECATED)\b",
                    Some("#facc15"),
                    None,
                    52,
                    "CustomHighlight.presets.status_signals.rules.warning",
                ),
                preset_rule(
                    "status_signals",
                    "error",
                    r"\b(ERROR|FAILED|FATAL|PANIC|DENIED|TIMEOUT)\b",
                    Some("#f87171"),
                    None,
                    56,
                    "CustomHighlight.presets.status_signals.rules.error",
                ),
            ],
        },
        HighlightPreset {
            id: "ip_addresses",
            title_key: "CustomHighlight.presets.ip_addresses.title",
            description_key: "CustomHighlight.presets.ip_addresses.description",
            rules: vec![
                preset_rule(
                    "ip_addresses",
                    "ipv4",
                    r"\b(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\b",
                    Some("#64c8ff"),
                    None,
                    40,
                    "CustomHighlight.presets.ip_addresses.rules.ipv4",
                ),
                preset_rule(
                    "ip_addresses",
                    "ipv6",
                    r"\b(?:[0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}\b|\b(?:[0-9a-fA-F]{1,4}:){1,7}:\b|\b(?:[0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}\b",
                    Some("#64c8ff"),
                    None,
                    39,
                    "CustomHighlight.presets.ip_addresses.rules.ipv6",
                ),
            ],
        },
        HighlightPreset {
            id: "network_markers",
            title_key: "CustomHighlight.presets.network_markers.title",
            description_key: "CustomHighlight.presets.network_markers.description",
            rules: vec![
                preset_rule(
                    "network_markers",
                    "url",
                    r"(?i)\b(?:https?|ssh|ftp|file)://[^\s]+",
                    Some("#38bdf8"),
                    None,
                    46,
                    "CustomHighlight.presets.network_markers.rules.url",
                ),
                preset_rule(
                    "network_markers",
                    "port",
                    r"(?i)\b(?:port|listen|listening)\b|\:\d{2,5}\b",
                    Some("#fb7185"),
                    None,
                    42,
                    "CustomHighlight.presets.network_markers.rules.port",
                ),
            ],
        },
        HighlightPreset {
            id: "time_and_numbers",
            title_key: "CustomHighlight.presets.time_and_numbers.title",
            description_key: "CustomHighlight.presets.time_and_numbers.description",
            rules: vec![
                preset_rule(
                    "time_and_numbers",
                    "datetime",
                    r"\b\d{4}-\d{2}-\d{2}(?:[ T]\d{2}:\d{2}(?::\d{2})?)?\b",
                    Some("#34d399"),
                    None,
                    40,
                    "CustomHighlight.presets.time_and_numbers.rules.datetime",
                ),
                preset_rule(
                    "time_and_numbers",
                    "clock",
                    r"\b\d{2}:\d{2}:\d{2}\b",
                    Some("#22d3ee"),
                    None,
                    38,
                    "CustomHighlight.presets.time_and_numbers.rules.clock",
                ),
                preset_rule(
                    "time_and_numbers",
                    "quantity",
                    r"\b\d+(?:\.\d+)?(?:ms|s|m|h|KB|MB|GB|TB|%)\b",
                    Some("#f59e0b"),
                    None,
                    36,
                    "CustomHighlight.presets.time_and_numbers.rules.quantity",
                ),
            ],
        },
    ]
}

pub fn merge_highlight_preset_rules(
    existing: &[TerminalHighlightRule],
    preset: &HighlightPreset,
) -> Vec<TerminalHighlightRule> {
    let mut merged = existing.to_vec();

    for preset_rule in preset
        .rules
        .iter()
        .map(HighlightPresetRule::to_terminal_rule)
    {
        if let Some(existing_rule) = merged.iter_mut().find(|rule| rule.id == preset_rule.id) {
            *existing_rule = preset_rule;
        } else {
            merged.push(preset_rule);
        }
    }

    merged
}

pub fn merge_builtin_highlight_rules(
    existing: &[TerminalHighlightRule],
) -> Vec<TerminalHighlightRule> {
    let mut merged = existing.to_vec();
    for preset in builtin_highlight_presets() {
        merged = merge_highlight_preset_rules(&merged, &preset);
    }
    merged
}

fn preset_rule(
    preset_id: &str,
    rule_name: &str,
    pattern: &'static str,
    foreground: Option<&'static str>,
    background: Option<&'static str>,
    priority: u8,
    note_key: &'static str,
) -> HighlightPresetRule {
    HighlightPresetRule {
        id: format!("preset:{preset_id}:{rule_name}"),
        pattern,
        foreground,
        background,
        priority,
        note_key,
    }
}

#[cfg(test)]
mod tests {
    use super::{builtin_highlight_presets, merge_highlight_preset_rules};
    use crate::settings::TerminalHighlightRule;

    #[test]
    fn highlight_preset_builtin_groups_are_exposed() {
        let presets = builtin_highlight_presets();

        assert_eq!(presets.len(), 5);
        assert_eq!(presets[0].id, "linux_permissions");
        assert_eq!(presets[1].id, "status_signals");
        assert_eq!(presets[2].id, "ip_addresses");
        assert_eq!(presets[3].id, "network_markers");
        assert_eq!(presets[4].id, "time_and_numbers");
        assert!(presets.iter().all(|preset| !preset.rules.is_empty()));
    }

    #[test]
    fn ip_highlight_preset_exposes_ipv4_and_ipv6_rules() {
        let preset = builtin_highlight_presets()
            .into_iter()
            .find(|preset| preset.id == "ip_addresses")
            .expect("应找到 IP 地址预设");

        assert!(preset
            .rules
            .iter()
            .any(|rule| rule.id == "preset:ip_addresses:ipv4"));
        assert!(preset
            .rules
            .iter()
            .any(|rule| rule.id == "preset:ip_addresses:ipv6"));
    }

    #[test]
    fn highlight_preset_merge_replaces_same_rule_ids_and_keeps_custom_rules() {
        let existing = vec![
            TerminalHighlightRule {
                id: "preset:status_signals:error".into(),
                enabled: false,
                pattern: "OLD_ERROR".into(),
                foreground: Some("#111111".into()),
                background: None,
                priority: 10,
                note: "old".into(),
            },
            TerminalHighlightRule {
                id: "custom:user-rule".into(),
                enabled: true,
                pattern: "hello".into(),
                foreground: Some("#222222".into()),
                background: None,
                priority: 20,
                note: "custom".into(),
            },
        ];
        let presets = builtin_highlight_presets();
        let preset = presets
            .into_iter()
            .find(|preset| preset.id == "status_signals")
            .expect("应找到状态词预设");

        let merged = merge_highlight_preset_rules(&existing, &preset);

        assert!(merged.iter().any(|rule| rule.id == "custom:user-rule"));
        let preset_rule = merged
            .iter()
            .find(|rule| rule.id == "preset:status_signals:error")
            .expect("应替换同 id 的预设规则");
        assert_ne!(preset_rule.pattern, "OLD_ERROR");
        assert!(merged.len() >= preset.rules.len());
    }
}
