use tool_runtime::{
    ResourceCapability, ResourceId, ResourceKind, ResourcePool, ResourceRef, ResourceScope,
    TargetResolutionError, ToolTargetSpec,
};

#[test]
fn first_resource_becomes_default_target() {
    let pool = ResourcePool::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
        .with_resource(ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"));

    assert_eq!(
        Some(&ResourceId::new("ssh-a")),
        pool.default_target.as_ref()
    );
    assert_eq!("prod-b", pool.resolve_target("prod-b").unwrap().label);
}

#[test]
fn default_target_is_not_a_resource_boundary() {
    let pool = ResourcePool::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
        .with_resource(ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"));

    let target = pool
        .resolve_target("ssh-b")
        .expect("non-default resource should resolve");

    assert_eq!(ResourceId::new("ssh-b"), target.id);
}

#[test]
fn target_matches_id_label_or_alias() {
    let pool = ResourcePool::new().with_resource(
        ResourceRef::new("db-prod", ResourceKind::Mysql, "primary database")
            .with_alias("prod-db")
            .with_alias("production database")
            .with_scope(ResourceScope::new("schema", "Schema", "public"))
            .with_capability(ResourceCapability::Query),
    );

    assert_eq!(
        ResourceId::new("db-prod"),
        pool.resolve_target("db-prod").unwrap().id
    );
    assert_eq!(
        ResourceId::new("db-prod"),
        pool.resolve_target("primary database").unwrap().id
    );
    assert_eq!(
        ResourceId::new("db-prod"),
        pool.resolve_target("prod-db").unwrap().id
    );
}

#[test]
fn target_matches_prompt_like_or_ssh_like_alias() {
    let pool = ResourcePool::new().with_resource(
        ResourceRef::new("terminal-prod", ResourceKind::Terminal, "prod terminal")
            .with_alias("zn-54")
            .with_alias("10.2.4.54"),
    );

    for target in [
        "root@zn-54:~",
        "root@zn-54:/var/log",
        "ssh://root@zn-54",
        "root@10.2.4.54:~",
        "zn-54:~",
    ] {
        assert_eq!(
            ResourceId::new("terminal-prod"),
            pool.resolve_target(target).unwrap().id,
            "{target} should resolve"
        );
    }
}

#[test]
fn kind_target_can_resolve_through_linked_saved_connection() {
    let pool = ResourcePool::new()
        .with_resource(
            ResourceRef::new("21", ResourceKind::Ssh, "prod-a")
                .with_alias("10.2.4.54")
                .with_alias("zn-54"),
        )
        .with_resource(
            ResourceRef::new("ssh-terminal-prod-a", ResourceKind::Terminal, "prod-a")
                .with_alias("21"),
        );

    let target = pool
        .resolve_target_for_kinds("root@zn-54:~", &[ResourceKind::Terminal])
        .expect("prompt-like target should resolve through saved SSH connection");

    assert_eq!(ResourceId::new("ssh-terminal-prod-a"), target.id);
}

#[test]
fn target_spec_filters_linked_targets_by_required_capability() {
    let pool = ResourcePool::new()
        .with_resource(
            ResourceRef::new("21", ResourceKind::Ssh, "prod-a")
                .with_alias("10.2.4.54")
                .with_alias("zn-54"),
        )
        .with_resource(
            ResourceRef::new("terminal-input", ResourceKind::Terminal, "prod-a")
                .with_alias("21")
                .with_capability(ResourceCapability::TerminalExec),
        )
        .with_resource(
            ResourceRef::new("terminal-remote", ResourceKind::Terminal, "prod-a")
                .with_alias("21")
                .with_capability(ResourceCapability::RemoteExec),
        );

    let target = pool
        .resolve_target_for_spec(
            "root@zn-54:~",
            &ToolTargetSpec::required_with_capabilities(
                vec![ResourceKind::Terminal],
                vec![ResourceCapability::TerminalExec],
            ),
        )
        .expect("terminal.exec target should resolve to terminal exec capable resource");

    assert_eq!(ResourceId::new("terminal-input"), target.id);
}

#[test]
fn terminal_control_target_requires_control_capability() {
    let pool = ResourcePool::new()
        .with_resource(
            ResourceRef::new("terminal-exec", ResourceKind::Terminal, "prod-a")
                .with_alias("21")
                .with_capability(ResourceCapability::TerminalExec),
        )
        .with_resource(
            ResourceRef::new("terminal-control", ResourceKind::Terminal, "prod-a")
                .with_alias("21")
                .with_capability(ResourceCapability::TerminalControl),
        );

    let target = pool
        .resolve_target_for_spec(
            "21",
            &ToolTargetSpec::required_with_capabilities(
                vec![ResourceKind::Terminal],
                vec![ResourceCapability::TerminalControl],
            ),
        )
        .expect("terminal.control should resolve to control-capable terminal");

    assert_eq!(ResourceId::new("terminal-control"), target.id);
}

#[test]
fn target_spec_reports_matched_target_without_required_capability() {
    let pool = ResourcePool::new()
        .with_resource(
            ResourceRef::new("21", ResourceKind::Ssh, "prod-a")
                .with_alias("10.2.4.54")
                .with_alias("zn-54"),
        )
        .with_resource(
            ResourceRef::new("terminal-visible", ResourceKind::Terminal, "prod-a").with_alias("21"),
        );

    let error = pool
        .resolve_target_for_spec(
            "root@zn-54:~",
            &ToolTargetSpec::required_with_capabilities(
                vec![ResourceKind::Terminal],
                vec![ResourceCapability::TerminalExec],
            ),
        )
        .expect_err("visible terminal without terminal exec capability should be rejected");

    assert!(matches!(
        error,
        TargetResolutionError::TargetLacksCapabilities { .. }
    ));
}

#[test]
fn ambiguous_target_is_rejected() {
    let pool = ResourcePool::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod").with_alias("prod"))
        .with_resource(ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod").with_alias("prod"));

    let error = pool
        .resolve_target("prod")
        .expect_err("ambiguous label should fail");

    assert!(matches!(
        error,
        TargetResolutionError::AmbiguousTarget { .. }
    ));
}

#[test]
fn default_target_resolution_returns_default_resource() {
    let pool = ResourcePool::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
        .with_resource(ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"));

    let target = pool.resolve_resource_target(None).unwrap();

    assert_eq!(ResourceId::new("ssh-a"), target.id);
}

#[test]
fn target_outside_pool_is_rejected() {
    let pool =
        ResourcePool::new().with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"));

    let error = pool
        .resolve_target("ssh-z")
        .expect_err("unknown target should fail");

    assert!(matches!(
        error,
        TargetResolutionError::TargetNotInPool { .. }
    ));
}
