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
        2,
        "both custom and built-in profiles must use the shared activation path",
    );
}
