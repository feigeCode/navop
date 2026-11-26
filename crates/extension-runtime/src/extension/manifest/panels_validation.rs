use std::collections::HashSet;

use super::{Manifest, PanelContrib};

/// 面板可以挂载的宿主视图锚点。
///
/// 每加一个锚点，都要有宿主视图实现对应 trait 并注册：
/// `terminal-sidebar` → `terminal_view::sidebar::extension_panels`，
/// `db-sidebar` → `db_view::extension_panels`。
pub(crate) const SUPPORTED_ANCHORS: &[&str] = &["terminal-sidebar", "db-sidebar"];

/// `contributes.panels` 安装期校验。
///
/// 路径风格与 `shell_validation.rs` / `tools_validation.rs` 一致：错误带
/// `/contributes/panels/<id>/<field>` 风格的字段路径，安装期直接拒绝。
pub(crate) fn validate_panels(manifest: &Manifest) -> Result<(), PanelsValidationError> {
    if manifest.contributes.panels.is_empty() {
        return Ok(());
    }
    let Some(_app) = manifest.contributes.shell_app.as_ref() else {
        return Err(error(
            "/contributes/panels",
            "声明面板的扩展必须同时声明 shellApp（面板是 app 的附加 surface，\
             入口复用 shellApp.entry）",
        ));
    };
    let mut seen_ids = HashSet::new();
    for panel in &manifest.contributes.panels {
        validate_panel(panel, &mut seen_ids)?;
    }
    Ok(())
}

fn validate_panel(
    panel: &PanelContrib,
    seen_ids: &mut HashSet<String>,
) -> Result<(), PanelsValidationError> {
    let field = |name: &str| format!("/contributes/panels/{}/{}", panel.id, name);

    if !is_valid_panel_id(&panel.id) {
        return Err(error(
            field("id"),
            "id 只能包含小写字母、数字、点、下划线、连字符，长度 1–64，\
             且不能以 .-_ 开头或结尾",
        ));
    }
    if !seen_ids.insert(panel.id.clone()) {
        return Err(error(field("id"), "同一扩展内的面板 id 不能重复"));
    }
    if let Some(title) = panel.title.as_deref() {
        if title.trim().is_empty() {
            return Err(error(field("title"), "title 声明了就不能为空"));
        }
        if title.chars().count() > 40 {
            return Err(error(field("title"), "title 长度不能超过 40"));
        }
    }
    if let Some(icon) = panel.icon.as_deref() {
        if icon.trim().is_empty() {
            return Err(error(field("icon"), "icon 声明了就不能为空"));
        }
    }
    if !SUPPORTED_ANCHORS.contains(&panel.anchor.as_str()) {
        return Err(error(
            field("anchor"),
            format!(
                "anchor `{}` 不受支持；当前支持 {}",
                panel.anchor,
                SUPPORTED_ANCHORS.join("、")
            ),
        ));
    }
    if !(-1000..=1000).contains(&panel.order) {
        return Err(error(field("order"), "order 必须在 -1000–1000 之间"));
    }
    if let Some(when) = panel.when.as_deref() {
        if when.chars().count() > 256 {
            return Err(error(field("when"), "when 长度不能超过 256"));
        }
        // 语法交给 one_core::when_clause 在求值时解析；这里只挡明显超长的。
    }
    Ok(())
}

fn is_valid_panel_id(id: &str) -> bool {
    id.len() <= 64
        && !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".-_".contains(&b))
        && !id.starts_with(['.', '-', '_'])
        && !id.ends_with(['.', '-', '_'])
}

fn error(field: impl Into<String>, reason: impl Into<String>) -> PanelsValidationError {
    PanelsValidationError {
        field: field.into(),
        reason: reason.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelsValidationError {
    pub field: String,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::extension::manifest::PanelPlacement;

    fn manifest_with(panels: serde_json::Value) -> Manifest {
        serde_json::from_value(json!({
            "schema_version": 1,
            "id": "com.example.tools",
            "name": "测试扩展",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.1.0", "nsh": "0.3.0" },
            "permissions": ["shell:exec"],
            "runtime": {
                "ipc": [{ "id": "provider", "entry": { "command": "./bin/provider" } }]
            },
            "contributes": {
                "shellApp": {
                    "title": "测试 app",
                    "entry": "ui/dist/bundle.js",
                    "modules": ["resource"],
                    "backends": { "main": "provider" }
                },
                "panels": panels
            }
        }))
        .expect("测试清单必须能被反序列化")
    }

    fn valid_panel() -> serde_json::Value {
        json!({
            "id": "inspector",
            "title": "检查器",
            "anchor": "terminal-sidebar",
            "order": 100
        })
    }

    fn validate_error(manifest: &Manifest) -> PanelsValidationError {
        validate_panels(manifest).expect_err("应当被安装期校验拒绝")
    }

    #[test]
    fn valid_panels_pass() {
        let manifest = manifest_with(json!([valid_panel(), {
            "id": "logs",
            "title": "日志",
            "anchor": "terminal-sidebar",
            "order": 200,
            "when": "connection.protocol == 'ssh'"
        }]));
        validate_panels(&manifest).expect("合法声明应通过");
    }

    #[test]
    fn panels_without_shell_app_are_rejected() {
        let mut manifest = manifest_with(json!([valid_panel()]));
        manifest.contributes.shell_app = None;
        let error = validate_error(&manifest);
        assert_eq!("/contributes/panels", error.field);
        assert!(error.reason.contains("shellApp"), "{}", error.reason);
    }

    #[test]
    fn duplicate_panel_ids_are_rejected() {
        let manifest = manifest_with(json!([valid_panel(), valid_panel()]));
        let error = validate_error(&manifest);
        assert_eq!("/contributes/panels/inspector/id", error.field);
        assert!(error.reason.contains("重复"), "{}", error.reason);
    }

    #[test]
    fn invalid_id_characters_are_rejected() {
        let mut panel = valid_panel();
        panel["id"] = json!("Bad Id");
        let manifest = manifest_with(json!([panel]));
        let error = validate_error(&manifest);
        assert_eq!("/contributes/panels/Bad Id/id", error.field);
    }

    #[test]
    fn db_sidebar_anchor_is_accepted() {
        let mut panel = valid_panel();
        panel["anchor"] = json!("db-sidebar");
        let manifest = manifest_with(json!([panel]));
        validate_panels(&manifest).expect("db-sidebar 应当被接受");
    }

    #[test]
    fn unknown_anchor_is_rejected() {
        let mut panel = valid_panel();
        panel["anchor"] = json!("main-sidebar");
        let manifest = manifest_with(json!([panel]));
        let error = validate_error(&manifest);
        assert_eq!("/contributes/panels/inspector/anchor", error.field);
        assert!(error.reason.contains("main-sidebar"), "{}", error.reason);
    }

    #[test]
    fn empty_title_is_rejected() {
        let mut panel = valid_panel();
        panel["title"] = json!("  ");
        let manifest = manifest_with(json!([panel]));
        let error = validate_error(&manifest);
        assert_eq!("/contributes/panels/inspector/title", error.field);
    }

    #[test]
    fn order_bounds_are_enforced() {
        let mut panel = valid_panel();
        panel["order"] = json!(1001);
        let manifest = manifest_with(json!([panel]));
        assert_eq!(
            validate_error(&manifest).field,
            "/contributes/panels/inspector/order"
        );
    }

    #[test]
    fn default_anchor_is_terminal_sidebar() {
        let manifest = manifest_with(json!([{
            "id": "inspector",
            "title": "检查器"
        }]));
        validate_panels(&manifest).expect("anchor 缺省即 terminal-sidebar");
        assert_eq!(manifest.contributes.panels[0].anchor, "terminal-sidebar");
    }

    #[test]
    fn placement_defaults_to_right_and_accepts_all_three_edges() {
        let manifest = manifest_with(json!([
            { "id": "a", "title": "默认" },
            { "id": "b", "title": "左", "placement": "left" },
            { "id": "c", "title": "下", "placement": "bottom" }
        ]));
        validate_panels(&manifest).expect("三种停靠位置都合法");
        assert_eq!(PanelPlacement::Right, manifest.contributes.panels[0].placement);
        assert_eq!(PanelPlacement::Left, manifest.contributes.panels[1].placement);
        assert_eq!(PanelPlacement::Bottom, manifest.contributes.panels[2].placement);
    }

    #[test]
    fn unknown_placement_is_rejected_at_parse_time() {
        let result = serde_json::from_value::<Manifest>(json!({
            "schema_version": 1,
            "id": "com.example.tools",
            "name": "测试扩展",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.1.0", "nsh": "0.3.0" },
            "permissions": ["shell:exec"],
            "contributes": {
                "shellApp": { "title": "app", "entry": "ui/dist/bundle.js" },
                "panels": [{ "id": "a", "placement": "top" }]
            }
        }));
        assert!(result.is_err(), "未知 placement 必须在安装期被拒绝");
    }

    #[test]
    fn empty_panels_section_is_fine() {
        let manifest = manifest_with(json!([]));
        validate_panels(&manifest).expect("没有面板就没有校验");
    }
}
