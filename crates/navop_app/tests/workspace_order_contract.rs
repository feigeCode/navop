#[test]
fn workspace_reload_honors_persisted_order_before_names() {
    let source = include_str!("../src/home_tab/data.rs");
    let load = source
        .split("pub(super) fn load_connections")
        .next()
        .unwrap();
    assert!(load.contains("sort_workspaces(&mut workspaces)"));
    let sort = source.split("fn sort_workspaces(").nth(1).unwrap();
    let sort = sort.split("#[cfg(test)]").next().unwrap();
    let order = sort
        .find(".sort_order")
        .expect("persisted order must be used");
    let name = sort.find("connection_name_cmp").unwrap();
    assert!(order < name, "names must only break equal-order ties");
    assert!(sort.contains(".then_with("));
}
