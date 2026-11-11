//! 结构契约：笔记模块的 Dialog 必须显式启用默认 footer。
//!
//! gpui-component 的 `Dialog::new()` 默认 `default_footer = false`，只有
//! `.confirm()` / `.alert()` 或自定义 `.footer(...)` 才渲染确认按钮。
//! 只写 `.on_ok(...)` 的弹窗没有确定按钮，用户选完内容只能点 × 关窗，
//! 表现就是「设置完路径/名称后没有任何变化」（2026-08 迁移 gpui-kit 时
//! 丢了 4 处 `.confirm()`）。

/// 返回缺少 footer 配置却绑定了 `on_ok` 的弹窗数量。
fn dialogs_without_footer(source: &str) -> usize {
    let body = source
        .split("#[cfg(test)]")
        .next()
        .expect("source has a body before the test module");
    body.split("open_dialog(")
        .filter(|chunk| chunk.contains(".on_ok("))
        .filter(|chunk| {
            !chunk.contains(".confirm()")
                && !chunk.contains(".alert()")
                && !chunk.contains(".footer(")
        })
        .count()
}

#[test]
fn notes_dialogs_render_their_confirm_buttons() {
    let setup = include_str!("notes_setup.rs");
    let actions = include_str!("notes_actions.rs");

    assert!(
        setup.contains("open_dialog(") && setup.contains(".on_ok("),
        "location dialog should still be scanned"
    );
    assert!(
        actions.contains("open_dialog(") && actions.contains(".on_ok("),
        "rename/delete/create dialogs should still be scanned"
    );
    assert_eq!(0, dialogs_without_footer(setup), "notes_setup.rs");
    assert_eq!(0, dialogs_without_footer(actions), "notes_actions.rs");
}
