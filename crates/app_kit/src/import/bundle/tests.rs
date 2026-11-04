use super::plan::{ImportReport, PlanInput, canonical_kind, plan_import};
use super::*;

fn connection(name: &str, kind: &str) -> BundleConnection {
    BundleConnection {
        name: name.to_string(),
        connection_type: kind.to_string(),
        params: "{}".to_string(),
        workspace: None,
        remark: None,
        selected_databases: None,
        sync_enabled: true,
    }
}

fn empty_input() -> PlanInput<'static> {
    PlanInput {
        existing_names: &[],
        existing_identities: &[],
        existing_workspaces: &[],
    }
}

fn full_bundle() -> ConnectionBundle {
    ConnectionBundle {
        version: BUNDLE_VERSION,
        exported_at: 1_760_000_000,
        app_id: Some("navop".to_string()),
        workspaces: vec![BundleWorkspace {
            id: Some("w1".to_string()),
            parent: None,
            name: "生产".to_string(),
            color: Some("#ff0000".to_string()),
            icon: Some("database".to_string()),
        }],
        connections: vec![BundleConnection {
            name: "主库".to_string(),
            connection_type: "Database".to_string(),
            params: r#"{"host":"db.internal","port":5432}"#.to_string(),
            workspace: Some("生产".to_string()),
            remark: Some("只读".to_string()),
            selected_databases: Some(r#"["app"]"#.to_string()),
            sync_enabled: false,
        }],
    }
}

#[test]
fn bundle_round_trips_through_json() {
    let bundle = full_bundle();
    let json = serde_json::to_string(&bundle).expect("序列化失败");
    let decoded: ConnectionBundle = serde_json::from_str(&json).expect("反序列化失败");
    assert_eq!(decoded, bundle);
    assert!(json.contains(r#""version":2"#));
}

#[test]
fn minimal_bundle_uses_defaults() {
    let json = r#"{"version":1,"connections":[{"name":"a","connection_type":"Ftp"}]}"#;
    let decoded: ConnectionBundle = serde_json::from_str(json).expect("反序列化失败");
    assert_eq!(decoded.exported_at, 0);
    assert_eq!(decoded.app_id, None);
    assert!(decoded.workspaces.is_empty());
    let connection = &decoded.connections[0];
    assert_eq!(connection.params, "");
    assert!(connection.sync_enabled);
    assert_eq!(connection.workspace, None);
}

#[test]
fn duplicate_identity_is_skipped() {
    let mut bundle = full_bundle();
    bundle.connections.push(connection("主库", "Database"));
    let names = vec!["主库".to_string()];
    let identities = vec![("主库".to_string(), "Database".to_string())];
    let plan = plan_import(
        &bundle,
        PlanInput {
            existing_names: &names,
            existing_identities: &identities,
            existing_workspaces: &[],
        },
        &["Database".to_string()],
    );
    assert_eq!(plan.report.added, 0);
    assert_eq!(plan.report.skipped, 2);
    assert!(plan.connections.is_empty());
}

#[test]
fn duplicate_name_gets_suffix() {
    let mut bundle = full_bundle();
    bundle.connections[0].connection_type = "Ftp".to_string();
    let names = vec!["主库".to_string()];
    let plan = plan_import(
        &bundle,
        PlanInput {
            existing_names: &names,
            existing_identities: &[],
            existing_workspaces: &["生产".to_string()],
        },
        &["Ftp".to_string()],
    );
    assert_eq!(plan.connections.len(), 1);
    assert_eq!(plan.connections[0].name, "主库（2）");
    assert!(plan.connections[0].renamed);
    assert_eq!(plan.report.renamed, 1);
}

#[test]
fn unknown_workspace_falls_back_to_ungrouped() {
    let bundle = full_bundle();
    let plan = plan_import(&bundle, empty_input(), &["Database".to_string()]);
    assert_eq!(plan.report.workspaces_created, 1);
    assert_eq!(plan.workspaces[0].name, "生产");
    assert_eq!(plan.connections[0].workspace_key, Some("生产".to_string()));
    assert!(!plan.connections[0].ungrouped);

    let mut orphan = bundle;
    orphan.workspaces.clear();
    let plan = plan_import(&orphan, empty_input(), &["Database".to_string()]);
    assert_eq!(plan.connections[0].workspace_key, None);
    assert!(plan.connections[0].ungrouped);
    assert_eq!(plan.report.ungrouped, 1);
}

#[test]
fn existing_workspace_matches_ignoring_case() {
    let mut bundle = full_bundle();
    bundle.connections[0].workspace = Some("  生产 ".to_string());
    let workspaces = vec!["生产".to_string()];
    let plan = plan_import(
        &bundle,
        PlanInput {
            existing_names: &[],
            existing_identities: &[],
            existing_workspaces: &workspaces,
        },
        &["Database".to_string()],
    );
    assert_eq!(plan.report.workspaces_created, 0);
    assert_eq!(plan.connections[0].workspace_key, Some("生产".to_string()));
}

#[test]
fn unsupported_and_port_forwarding_are_counted_separately() {
    let bundle = ConnectionBundle {
        version: BUNDLE_VERSION,
        exported_at: 0,
        app_id: None,
        workspaces: vec![],
        connections: vec![
            connection("壳", "SshSftp"),
            connection("跳板", "PortForwarding"),
            connection("怪", "Quantum"),
        ],
    };
    let supported = vec!["Database".to_string(), "PortForwarding".to_string()];
    let plan = plan_import(&bundle, empty_input(), &supported);
    assert_eq!(plan.report.unsupported, 2, "SshSftp 与 Quantum 都不支持");
    assert_eq!(plan.report.skipped_port_forwarding, 1);
    assert_eq!(plan.report.added, 0);
}

#[test]
fn encrypted_params_are_reported_for_repair() {
    let mut bundle = full_bundle();
    bundle.connections[0].params = r#"{"password":"ENC:bogus-ciphertext"}"#.to_string();
    let plan = plan_import(&bundle, empty_input(), &["Database".to_string()]);
    assert!(plan.connections[0].needs_repair);
    assert_eq!(plan.report.needs_repair, 1);
    assert_eq!(
        plan.connections[0].params, bundle.connections[0].params,
        "密文必须原样保留，不能静默清空"
    );
}

#[test]
fn summary_lists_every_non_zero_counter() {
    let report = ImportReport {
        added: 2,
        skipped: 1,
        renamed: 1,
        workspaces_created: 1,
        unsupported: 1,
        skipped_port_forwarding: 1,
        ungrouped: 1,
        needs_repair: 1,
    };
    let summary = report.summary();
    for expected in [
        "新增 2 个连接",
        "新建 1 个工作区",
        "1 个重名连接已追加后缀",
        "跳过 1 个重复连接",
        "1 个连接类型当前产品不支持",
        "1 个端口转发暂未导入（连接引用未重映射）",
        "1 个连接已放入未分组",
        "1 个连接需要重新输入密码",
    ] {
        assert!(summary.contains(expected), "缺少「{expected}」：{summary}");
    }
    assert_eq!(ImportReport::default().summary(), "新增 0 个连接");
}

#[test]
fn canonical_kind_rejects_unknown_names() {
    assert_eq!(canonical_kind("Database"), Some("Database".to_string()));
    assert_eq!(canonical_kind("数据库"), None);
    assert_eq!(canonical_kind(""), None);
}

fn workspace(id: &str, parent: Option<&str>, name: &str) -> BundleWorkspace {
    BundleWorkspace {
        id: Some(id.to_string()),
        parent: parent.map(str::to_string),
        name: name.to_string(),
        color: None,
        icon: None,
    }
}

fn workspace_bundle(workspaces: Vec<BundleWorkspace>) -> ConnectionBundle {
    ConnectionBundle {
        version: BUNDLE_VERSION,
        exported_at: 0,
        app_id: Some("navop".to_string()),
        workspaces,
        connections: vec![],
    }
}

#[test]
fn nested_workspaces_keep_their_parent_chain() {
    // 子级故意排在父级前面：迁移包的顺序不可信，规划阶段要先按「父在前」重排。
    let bundle = workspace_bundle(vec![
        workspace("w2", Some("w1"), "生产 / 只读从库"),
        workspace("w1", None, "生产"),
        workspace("w3", Some("w2"), "生产 / 只读从库 / 报表"),
    ]);
    let plan = plan_import(&bundle, empty_input(), &supported_kinds());
    let names: Vec<&str> = plan
        .workspaces
        .iter()
        .map(|workspace| workspace.name.as_str())
        .collect();
    assert_eq!(names, ["生产", "生产 / 只读从库", "生产 / 只读从库 / 报表"]);
    assert_eq!(plan.workspaces[0].parent_key, None);
    assert_eq!(plan.workspaces[1].parent_key.as_deref(), Some("生产"));
    assert_eq!(
        plan.workspaces[2].parent_key.as_deref(),
        Some("生产 / 只读从库")
    );
}

#[test]
fn existing_workspace_is_reused_as_parent() {
    let bundle = workspace_bundle(vec![
        workspace("w1", None, "生产"),
        workspace("w2", Some("w1"), "生产 / 只读从库"),
    ]);
    let existing = vec!["生产".to_string()];
    let plan = plan_import(
        &bundle,
        PlanInput {
            existing_names: &[],
            existing_identities: &[],
            existing_workspaces: &existing,
        },
        &supported_kinds(),
    );
    assert_eq!(plan.workspaces.len(), 1);
    assert_eq!(plan.workspaces[0].name, "生产 / 只读从库");
    assert_eq!(plan.workspaces[0].parent_key.as_deref(), Some("生产"));
}

#[test]
fn cyclic_parent_is_broken_instead_of_hanging() {
    let bundle = workspace_bundle(vec![
        workspace("w1", Some("w404"), "孤儿"),
        workspace("w2", Some("w3"), "甲"),
        workspace("w3", Some("w2"), "乙"),
    ]);
    let plan = plan_import(&bundle, empty_input(), &supported_kinds());
    assert_eq!(plan.workspaces.len(), 3);
    let by_name = |name: &str| {
        plan.workspaces
            .iter()
            .find(|workspace| workspace.name == name)
            .unwrap_or_else(|| panic!("缺少工作区「{name}」"))
    };
    // 父级不存在的降级为顶层。
    assert_eq!(by_name("孤儿").parent_key, None);
    // 父子成环时只断一条边：先出现的「甲」留在顶层，「乙」挂在它下面，导入不会卡死。
    assert_eq!(by_name("甲").parent_key, None);
    assert_eq!(by_name("乙").parent_key.as_deref(), Some("甲"));
}

fn supported_kinds() -> Vec<String> {
    vec!["Database".to_string(), "Ssh".to_string(), "Ftp".to_string()]
}
