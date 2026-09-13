#[test]
fn local_terminal_activation_does_not_reborrow_home() {
    let source = include_str!("../../home/home_tabs.rs");
    let opener = source
        .split("    fn add_local_terminal_tab(")
        .nth(1)
        .and_then(|source| {
            source
                .split("    pub(crate) fn add_item_to_tab_with_mode(")
                .next()
        })
        .expect("local terminal tab opener");

    // Activating a terminal deactivates the pinned HomePage via Entity::update.
    // Deferring is not enough if the callback leases HomePage again.
    assert!(opener.contains("window.defer(cx, move |window, cx|"));
    assert!(opener.contains("tc.add_and_activate_tab_with_focus(tab, window, cx)"));
    assert!(!opener.contains("home.update("));
    assert!(!opener.contains("cx.entity()"));
    assert!(!opener.contains("cx.defer_in("));
}

#[test]
fn all_local_terminal_profiles_use_the_same_tab_opener() {
    let source = include_str!("../../home/home_tabs.rs");
    let openers = source
        .split("    pub(crate) fn add_terminal_tab(")
        .nth(1)
        .and_then(|source| source.split("    fn add_local_terminal_tab(").next())
        .expect("local terminal profile openers");

    assert!(openers.contains("self.add_terminal_tab_from_profile(None, window, cx)"));
    assert!(openers.contains("self.add_terminal_tab_from_profile(Some(profile_kind), window, cx)"));
    assert_eq!(
        openers
            .matches("self.add_local_terminal_tab(config, window, cx)")
            .count(),
        3,
        "custom, built-in and WSL distro profiles must use the shared activation path",
    );
}

#[test]
fn wsl_distro_menu_items_reuse_the_local_terminal_activation_path() {
    let source = include_str!("../../home/home_tabs.rs");
    let opener = source
        .split("    pub(crate) fn add_terminal_tab_with_wsl_distro(")
        .nth(1)
        .and_then(|source| source.split("    fn add_terminal_tab_from_profile(").next())
        .expect("WSL distro tab opener");

    assert!(opener.contains("local_config_for_wsl_distro(&distro)"));
    assert!(opener.contains("self.add_local_terminal_tab(config, window, cx)"));

    let menu_source = include_str!("../../home_tab/local_terminal.rs");
    // WSL 发行版区段仅在 Windows 上渲染，其余平台保持原菜单。
    assert!(menu_source.contains("#[cfg(target_os = \"windows\")]\nfn append_wsl_distributions"));
    assert!(menu_source.contains("wsl_distributions_section"));
    assert!(menu_source.contains("add_terminal_tab_with_wsl_distro"));
    // 区段入口仅在识别到发行版后出现，并提供重新识别操作。
    assert!(menu_source.contains("wsl_distributions_refresh"));
    assert!(menu_source.contains("load_wsl_distributions"));

    let data_source = include_str!("../data.rs");
    assert!(data_source.contains(
        "#[cfg(target_os = \"windows\")]\n    pub(super) fn load_wsl_distributions"
    ));
    assert!(data_source.contains("terminal::list_wsl_distributions()"));
}
