#[test]
fn local_terminal_tabs_open_through_the_domain_registry() {
    let source = include_str!("../../home/home_tabs.rs");
    let opener = source
        .split("    pub(crate) fn open_local_terminal_tab(")
        .nth(1)
        .and_then(|source| {
            source
                .split("    #[cfg(feature = \"database\")]\n    pub(crate) fn add_item_to_tab_with_mode(")
                .next()
        })
        .expect("local terminal tab opener");

    // 公共壳不再自己构造终端视图：本地终端由 shell 领域注册的 TabOpener 打开。
    assert!(opener.contains("let intent = TabIntent::Terminal { profile };"));
    assert!(opener.contains("app_kit::registry().open_tab(&intent, TabOpenMode::Activate, self, window, cx)"));
    assert!(!opener.contains("TerminalWorkspace::new_with_index"));
    assert!(!opener.contains("TerminalView::new"));
}

#[test]
fn local_terminal_menu_passes_profile_descriptors() {
    let menu_source = include_str!("../../home_tab/local_terminal.rs");

    // 内置 profile 与自定义 profile 都通过领域描述传递，公共壳不解析启动参数。
    assert!(menu_source.contains("kind: domain_terminal_kind(*kind)"));
    assert!(menu_source.contains("kind: TerminalProfileKind::Custom"));
    assert!(menu_source.contains("id: Some(profile.id.clone())"));
    assert!(menu_source.contains("open_local_terminal_tab("));
    assert!(!menu_source.contains("add_terminal_tab_with_profile"));
    assert!(!menu_source.contains("add_terminal_tab_with_custom_profile"));
}

#[test]
fn wsl_distro_menu_uses_shell_extras_snapshot() {
    let menu_source = include_str!("../../home_tab/local_terminal.rs");

    // WSL 发行版由 shell 领域的 ShellExtras 暴露，公共壳只渲染已有快照。
    assert!(menu_source.contains("app_kit::registry().shell_extras()"));
    assert!(menu_source.contains("extras.wsl_distributions(cx)"));
    assert!(menu_source.contains("refresh_wsl_distributions(cx)"));
    assert!(menu_source.contains("kind: TerminalProfileKind::Wsl"));
    assert!(menu_source.contains("wsl_distributions_section"));
    assert!(menu_source.contains("wsl_distributions_refresh"));

    let data_source = include_str!("../data.rs");
    assert!(!data_source.contains("load_wsl_distributions"));
    assert!(!data_source.contains("terminal::list_wsl_distributions"));
}
