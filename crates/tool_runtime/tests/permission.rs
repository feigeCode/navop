use tool_runtime::{
    OperationPolicy, PermissionDecision, PermissionPolicy, PermissionProfile, ResourceId,
    RiskLevel, ToolAnnotations, ToolId,
};

#[test]
fn safe_profile_allows_read_and_denies_write() {
    let policy = PermissionPolicy::for_profile(PermissionProfile::Safe);

    assert_eq!(
        PermissionDecision::Allow,
        policy.decide(
            &ToolId::new("db.query"),
            None,
            &ToolAnnotations::read_only("Query")
        )
    );
    assert_eq!(
        PermissionDecision::Deny,
        policy.decide(
            &ToolId::new("db.exec"),
            None,
            &ToolAnnotations::mutating("Exec")
        )
    );
}

#[test]
fn confirm_profile_asks_for_mutating_and_high_risk_tools() {
    let policy = PermissionPolicy::for_profile(PermissionProfile::Confirm);

    assert_eq!(
        PermissionDecision::Allow,
        policy.decide(
            &ToolId::new("db.query"),
            None,
            &ToolAnnotations::read_only("Query")
        )
    );
    assert_eq!(
        PermissionDecision::Ask,
        policy.decide(
            &ToolId::new("db.exec"),
            None,
            &ToolAnnotations::mutating("Exec")
        )
    );
    assert_eq!(
        PermissionDecision::Ask,
        policy.decide(
            &ToolId::new("ssh.exec"),
            None,
            &ToolAnnotations::read_only("Exec").with_risk(RiskLevel::High)
        )
    );
}

#[test]
fn auto_profile_allows_low_and_medium_but_asks_for_high_or_open_world() {
    let policy = PermissionPolicy::for_profile(PermissionProfile::Auto);

    assert_eq!(
        PermissionDecision::Allow,
        policy.decide(
            &ToolId::new("redis.get"),
            None,
            &ToolAnnotations::read_only("Get").with_risk(RiskLevel::Low)
        )
    );
    assert_eq!(
        PermissionDecision::Allow,
        policy.decide(
            &ToolId::new("redis.keys"),
            None,
            &ToolAnnotations::read_only("Keys").with_risk(RiskLevel::Medium)
        )
    );
    assert_eq!(
        PermissionDecision::Ask,
        policy.decide(
            &ToolId::new("ssh.exec"),
            None,
            &ToolAnnotations::mutating("Exec").with_risk(RiskLevel::High)
        )
    );
}

#[test]
fn unrestricted_profile_allows_by_default() {
    let policy = PermissionPolicy::for_profile(PermissionProfile::Unrestricted);

    assert_eq!(
        PermissionDecision::Allow,
        policy.decide(
            &ToolId::new("sftp.write"),
            None,
            &ToolAnnotations::mutating("Write").with_risk(RiskLevel::Critical)
        )
    );
}

#[test]
fn per_tool_and_resource_overrides_win() {
    let mut policy = PermissionPolicy::for_profile(PermissionProfile::Auto);
    policy
        .per_tool_overrides
        .insert(ToolId::new("db.exec"), OperationPolicy::Deny);
    policy
        .per_resource_overrides
        .insert(ResourceId::new("prod-db"), OperationPolicy::Ask);

    assert_eq!(
        PermissionDecision::Deny,
        policy.decide(
            &ToolId::new("db.exec"),
            Some(&ResourceId::new("staging-db")),
            &ToolAnnotations::mutating("Exec")
        )
    );
    assert_eq!(
        PermissionDecision::Ask,
        policy.decide(
            &ToolId::new("db.query"),
            Some(&ResourceId::new("prod-db")),
            &ToolAnnotations::read_only("Query")
        )
    );
}
