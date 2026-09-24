use std::path::PathBuf;

use html_preview::resolve_extension_asset_url;

use crate::{
    ExtensionRuntimeCatalog,
    extension::manifest::{
        ApiVersions, CommandContrib, CommandHandlerContrib, ContributesManifest,
        DocumentExporterContrib, Engines, HtmlPreviewTransformContrib, IpcEntry, IpcRuntime,
        IpcTransport, Manifest, MenuCommandRef, MenuContrib, ResourceConnectionContrib,
        ResourceConnectionFieldType, ResourceConnectionForm, ResourceConnectionFormField,
        ResourceConnectionFormTab, ResourceWorkbenchBinding, ResourceWorkbenchBindingSource,
        ResourceWorkbenchColumn, ResourceWorkbenchColumnStyle, ResourceWorkbenchColumnType,
        ResourceWorkbenchContrib, ResourceWorkbenchEffect, ResourceWorkbenchOpen,
        ResourceWorkbenchOperation, ResourceWorkbenchOperationMode, ResourceWorkbenchPage,
        ResourceWorkbenchPagination, ResourceWorkbenchPaginationKind, ResourceWorkbenchPrimitive,
        ResourceWorkbenchRenderer, ResourceWorkbenchRendererKind, ResourceWorkbenchRowAction,
        ResourceWorkbenchTable, ResourceWorkbenchTerminal, ResourceWorkbenchTerminalOperation,
        ResourceWorkbenchValueType, ResourceWorkbenchViewer, ResourceWorkbenchViewerFormat,
        RuntimeSection, ShellHostModule, ShellSurface, ShellViewContrib, WasmRuntime,
        WasmRuntimeKind,
        contributes::{
            RemoteFileEditorCommandContrib, RemoteFileEditorContrib, RemoteFileEditorLaunchMode,
        },
    },
};

#[test]
fn runtime_catalog_registers_wasm_command_and_db_tree_menu() {
    let mut manifest = base_manifest();
    manifest.runtime.wasm.push(wasm_runtime("main"));
    manifest
        .contributes
        .commands
        .push(command("main", "example.sync_table"));
    manifest.contributes.menus.insert(
        "db.tree.table".to_string(),
        vec![MenuContrib {
            command: MenuCommandRef {
                id: "example.sync_table".to_string(),
            },
            label: Some("同步表".to_string()),
            group: Some("extension@10".to_string()),
            when: Some("node.type == 'table'".to_string()),
            requires_active: true,
        }],
    );

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let menu_registry = catalog.db_tree_menu_registry();
    let table_items = menu_registry.items_for_node(db::DbNodeType::Table);

    assert!(
        catalog
            .component_permissions_for_command("example.sync_table")
            .is_ok()
    );
    assert_eq!(1, table_items.len());
    assert_eq!("com.example.tools", table_items[0].extension_id);
    assert_eq!("example.sync_table", table_items[0].command_id);
    assert_eq!("同步表", table_items[0].label);
}

#[test]
fn runtime_catalog_rejects_wasm_command_with_missing_runtime() {
    let mut manifest = base_manifest();
    manifest
        .contributes
        .commands
        .push(command("missing", "example.missing"));

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(error.to_string().contains("unknown runtime_id"));
}

#[test]
fn runtime_catalog_exposes_component_permissions_for_command() {
    let mut manifest = base_manifest();
    manifest.runtime.wasm.push(wasm_runtime("main"));
    manifest
        .contributes
        .commands
        .push(command("main", "example.search"));

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let permissions = catalog
        .component_permissions_for_command("example.search")
        .unwrap();

    assert_eq!(vec!["db:schema:*", "ui:notify"], permissions);
}

#[test]
fn runtime_catalog_registers_wasm_html_preview_transform_with_assets() {
    let mut manifest = base_manifest();
    manifest.runtime.wasm.push(wasm_runtime("main"));
    manifest
        .contributes
        .html_preview_transforms
        .push(HtmlPreviewTransformContrib {
            id: "example.decorate_html".to_string(),
            runtime_id: "main".to_string(),
            function: "transform-html".to_string(),
            languages: vec!["html".to_string(), "htm".to_string()],
            assets: "assets".to_string(),
        });

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let transforms = catalog.html_preview_transforms_for_language("HTML");

    assert_eq!(1, transforms.len());
    assert_eq!("com.example.tools", transforms[0].extension_id);
    assert_eq!("example.decorate_html", transforms[0].id);
    assert_eq!("com.example.tools::main", transforms[0].runtime_id);
    assert_eq!("transform-html", transforms[0].function);
    assert_eq!(
        PathBuf::from("/tmp/com.example.tools/assets"),
        transforms[0].assets_root
    );
    assert_eq!(
        PathBuf::from("/tmp/com.example.tools/assets/app.css"),
        resolve_extension_asset_url("onet-extension://com.example.tools/app.css").unwrap()
    );
}

#[test]
fn runtime_catalog_resolves_document_exporter_by_format() {
    let mut manifest = base_manifest();
    manifest.runtime.wasm.push(wasm_runtime("exporter"));
    manifest
        .contributes
        .document_exporters
        .push(DocumentExporterContrib {
            id: "notes-documents".to_string(),
            display_name: "HTML, PDF and Word".to_string(),
            runtime_id: "exporter".to_string(),
            function: "export-document".to_string(),
            formats: vec!["html".to_string(), "pdf".to_string(), "docx".to_string()],
            output_media_types: vec!["text/html".to_string(), "application/pdf".to_string()],
            priority: 100,
        });

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let exporter = catalog.document_exporter_for_format("PDF").unwrap();

    assert_eq!("notes-documents", exporter.id);
    assert_eq!("com.example.tools::exporter", exporter.runtime_id);
    assert_eq!("export-document", exporter.function);
    assert!(catalog.document_exporter_for_format("odt").is_none());
}

#[test]
fn runtime_catalog_registers_remote_file_editors() {
    let mut manifest = base_manifest();
    manifest
        .contributes
        .remote_file_editors
        .push(RemoteFileEditorContrib {
            id: "notepad-plus-plus".to_string(),
            display_name: "Notepad++".to_string(),
            platforms: vec!["windows".to_string()],
            file_masks: vec!["*".to_string()],
            priority: 100,
            command: RemoteFileEditorCommandContrib {
                launch_mode: RemoteFileEditorLaunchMode::MacosOpen,
                program_candidates: vec!["notepad++.exe".to_string()],
                args: vec!["{file}".to_string()],
            },
        });

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let editors = catalog.remote_file_editors();

    assert_eq!(1, editors.len());
    assert_eq!("com.example.tools", editors[0].extension_id);
    assert_eq!("notepad-plus-plus", editors[0].id);
    assert_eq!(
        "com.example.tools::notepad-plus-plus",
        editors[0].editor_key
    );
    assert_eq!("Notepad++", editors[0].display_name);
    assert_eq!(vec!["windows"], editors[0].platforms);
    assert_eq!(vec!["*"], editors[0].file_masks);
    assert_eq!(100, editors[0].priority);
    assert_eq!(
        RemoteFileEditorLaunchMode::MacosOpen,
        editors[0].command.launch_mode
    );
    assert_eq!(vec!["notepad++.exe"], editors[0].command.program_candidates);
    assert_eq!(vec!["{file}"], editors[0].command.args);
}

#[test]
fn runtime_catalog_registers_shell_view_with_resolved_backend() {
    let mut manifest = shell_manifest();
    manifest
        .contributes
        .shell_views
        .push(shell_view("ui/explorer.js"));

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let contribution = catalog.shell_view("com.example.tools", "explorer").unwrap();

    assert_eq!("com.example.tools::explorer", contribution.view_key);
    assert_eq!("Example Explorer", contribution.title);
    assert_eq!(
        PathBuf::from("/tmp/com.example.tools/ui/explorer.js"),
        contribution.entry_path
    );
    assert_eq!(
        Some("com.example.tools::provider"),
        contribution.backends.get("search").map(String::as_str)
    );
    assert_eq!(
        vec![ShellHostModule::Context, ShellHostModule::Resource],
        contribution.modules.iter().copied().collect::<Vec<_>>()
    );
    assert_eq!(
        vec!["shell:exec", "spawn:./bin/provider", "secrets:read:self.*"],
        contribution.permissions
    );
}

#[test]
fn runtime_catalog_registers_extension_connection_with_optional_shell_view() {
    let mut manifest = shell_manifest();
    manifest
        .contributes
        .shell_views
        .push(shell_view("ui/explorer.js"));
    manifest.contributes.connections.push(resource_connection());

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let connection = catalog
        .resource_connection("com.example.tools", "search")
        .unwrap();

    assert_eq!("com.example.tools::provider", connection.runtime_id);
    assert_eq!("elasticsearch", connection.resource_type);
    assert_eq!(Some("explorer"), connection.shell_view_id.as_deref());
    assert_eq!(2, connection.form.tabs[0].fields.len());
}

#[test]
fn runtime_catalog_registers_headless_extension_connection() {
    let mut manifest = shell_manifest();
    let mut connection = resource_connection();
    connection.shell_view_id = None;
    manifest.contributes.connections.push(connection);

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();

    assert!(
        catalog
            .resource_connection("com.example.tools", "search")
            .unwrap()
            .shell_view_id
            .is_none()
    );
}

#[test]
fn runtime_catalog_rejects_connection_secret_without_self_permission() {
    let mut manifest = shell_manifest();
    manifest
        .permissions
        .retain(|permission| permission != "secrets:read:self.*");
    let mut connection = resource_connection();
    connection.shell_view_id = None;
    manifest.contributes.connections.push(connection);

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(error.to_string().contains("secrets:read:self.*"), "{error}");
}

#[test]
fn runtime_catalog_rejects_singleton_connection_shell_view() {
    let mut manifest = shell_manifest();
    let mut view = shell_view("ui/explorer.js");
    view.singleton = true;
    manifest.contributes.shell_views.push(view);
    manifest.contributes.connections.push(resource_connection());

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(
        error.to_string().contains("must not be singleton"),
        "{error}"
    );
}

#[test]
fn runtime_catalog_rejects_connection_view_without_connection_runtime() {
    let mut manifest = shell_manifest();
    let mut view = shell_view("ui/explorer.js");
    view.backends.insert("search".into(), "other".into());
    manifest.runtime.ipc.push(IpcRuntime {
        id: "other".into(),
        entry: IpcEntry {
            command: "./bin/provider".into(),
            args: Vec::new(),
            working_dir: None,
            env: Default::default(),
        },
        transport: IpcTransport::default(),
        auto_restart: true,
        max_restart_attempts: 3,
        shutdown_grace_ms: 1_000,
    });
    manifest.contributes.shell_views.push(view);
    manifest.contributes.connections.push(resource_connection());

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(error.to_string().contains("connection runtime"), "{error}");
}

#[test]
fn runtime_catalog_rejects_unknown_visibility_field() {
    let mut manifest = shell_manifest();
    let mut connection = resource_connection();
    connection.shell_view_id = None;
    connection.form.tabs[0].fields[0].visible_when.push(
        crate::extension::manifest::ResourceConnectionVisibilityRule {
            field: "missing".into(),
            equals: "true".into(),
        },
    );
    manifest.contributes.connections.push(connection);

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(
        error.to_string().contains("unknown visibility field"),
        "{error}"
    );
}

#[test]
fn runtime_catalog_registers_resource_workbench_for_connection() {
    let mut manifest = shell_manifest();
    manifest
        .contributes
        .shell_views
        .push(shell_view("ui/explorer.js"));
    manifest.contributes.connections.push(resource_connection());
    manifest
        .contributes
        .resource_workbenches
        .push(resource_workbench());

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let workbench = catalog
        .resource_workbench_for_connection("com.example.tools", "search")
        .unwrap();

    assert_eq!("search-workbench", workbench.id);
    assert_eq!("overview", workbench.default_page);
}

/// 万物可连验证:不同 resourceType(docker/mq/nacos)的工作台声明
/// 走同一套注册与查询,不引入领域分支。
#[test]
fn runtime_catalog_supports_workbenches_across_resource_types() {
    for resource_type in ["docker", "mq", "nacos"] {
        let mut manifest = shell_manifest();
        manifest
            .contributes
            .shell_views
            .push(shell_view("ui/explorer.js"));
        let mut connection = resource_connection();
        connection.resource_type = resource_type.into();
        manifest.contributes.connections.push(connection);
        let mut workbench = resource_workbench();
        workbench.resource_type = resource_type.into();
        manifest.contributes.resource_workbenches.push(workbench);

        let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
        let registered = catalog
            .resource_workbench_for_connection("com.example.tools", "search")
            .unwrap_or_else(|| panic!("workbench for {resource_type} must resolve"));
        assert_eq!(resource_type, registered.resource_type);
    }
}

/// 同连接被两个工作台绑定时必须被拒绝(一对一绑定约束)。
#[test]
fn runtime_catalog_rejects_second_workbench_on_same_connection() {
    let mut manifest = shell_manifest();
    manifest
        .contributes
        .shell_views
        .push(shell_view("ui/explorer.js"));
    manifest.contributes.connections.push(resource_connection());
    manifest
        .contributes
        .resource_workbenches
        .push(resource_workbench());
    let mut duplicate = resource_workbench();
    duplicate.id = "second-workbench".into();
    manifest.contributes.resource_workbenches.push(duplicate);

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(
        error.to_string().contains("more than one workbench"),
        "{error}"
    );
}

/// 跨扩展 connectionIds 引用必须被拒绝。
#[test]
fn runtime_catalog_rejects_workbench_with_unknown_connection() {
    let mut manifest = shell_manifest();
    manifest
        .contributes
        .resource_workbenches
        .push(resource_workbench());

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(error.to_string().contains("unknown connection"), "{error}");
}

#[test]
fn runtime_catalog_rejects_embedded_shell_with_raw_provider_modules() {
    let mut manifest = shell_manifest();
    let mut connection = resource_connection();
    connection.shell_view_id = None;
    manifest.contributes.connections.push(connection);

    let mut view = shell_view("ui/search-editor.js");
    view.modules = vec![
        ShellHostModule::Context,
        ShellHostModule::Workbench,
        ShellHostModule::Resource,
    ];
    manifest.contributes.shell_views.push(view);

    let mut workbench = resource_workbench();
    workbench.pages[0].renderer.kind = ResourceWorkbenchRendererKind::Shell;
    workbench.pages[0].renderer.view_id = Some("explorer".into());
    workbench.pages[0].renderer.fallback = Some("native".into());
    manifest.contributes.resource_workbenches.push(workbench);

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("cannot request raw resource/job/event/blob/runtime/dev"),
        "{error}"
    );
}

#[test]
fn runtime_catalog_resolves_collection_row_actions_and_badge_columns() {
    let mut manifest = shell_manifest();
    manifest
        .contributes
        .shell_views
        .push(shell_view("ui/explorer.js"));
    manifest.contributes.connections.push(resource_connection());
    let mut workbench = resource_workbench();
    workbench.operations.insert(
        "listItems".to_string(),
        ResourceWorkbenchOperation {
            mode: ResourceWorkbenchOperationMode::Invoke,
            method: "example/item/list".into(),
            requires: vec!["example/item/list".into()],
            effect: ResourceWorkbenchEffect::Read,
            params: Default::default(),
        },
    );
    workbench.operations.insert(
        "startItem".to_string(),
        ResourceWorkbenchOperation {
            mode: ResourceWorkbenchOperationMode::Invoke,
            method: "example/item/start".into(),
            requires: vec!["example/item/start".into()],
            effect: ResourceWorkbenchEffect::Write,
            params: Default::default(),
        },
    );
    workbench.default_page = "items".into();
    workbench.pages[0] = ResourceWorkbenchPage {
        id: "items".into(),
        title: "Items".into(),
        renderer: ResourceWorkbenchRenderer {
            kind: ResourceWorkbenchRendererKind::Native,
            view_id: None,
            fallback: None,
        },
        load: Some(crate::extension::manifest::ResourceWorkbenchAction {
            operation: "listItems".into(),
        }),
        tab_group_id: None,
        route: None,
        links: vec![],
        stack: vec![ResourceWorkbenchPrimitive::Table(ResourceWorkbenchTable {
            items_path: "/items".into(),
            key_paths: vec!["/id".into()],
            pagination: ResourceWorkbenchPagination {
                kind: ResourceWorkbenchPaginationKind::None,
            },
            columns: vec![ResourceWorkbenchColumn {
                id: "state".into(),
                title: "State".into(),
                path: "/state".into(),
                value_type: ResourceWorkbenchColumnType::Display,
                style: ResourceWorkbenchColumnStyle::Badge,
            }],
            open: None,
            actions: vec![ResourceWorkbenchRowAction {
                id: "start".into(),
                label: "Start".into(),
                operation: "startItem".into(),
            }],
        })],
    };
    manifest.contributes.resource_workbenches.push(workbench);

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let workbench = catalog
        .resource_workbench_for_connection("com.example.tools", "search")
        .unwrap_or_else(|| panic!("workbench must resolve"));
    let table = match &workbench.pages[0].stack[0] {
        ResourceWorkbenchPrimitive::Table(table) => table,
        other => panic!("expected table primitive, got {other:?}"),
    };
    assert_eq!(1, table.actions.len());
    assert_eq!("startItem", table.actions[0].operation);
    assert_eq!(ResourceWorkbenchColumnStyle::Badge, table.columns[0].style);
}

#[test]
fn runtime_catalog_rejects_collection_action_with_unknown_operation() {
    let mut manifest = shell_manifest();
    manifest
        .contributes
        .shell_views
        .push(shell_view("ui/explorer.js"));
    manifest.contributes.connections.push(resource_connection());
    let mut workbench = resource_workbench();
    workbench.default_page = "items".into();
    let mut page = resource_workbench().pages.remove(0);
    page.id = "items".into();
    page.load = Some(crate::extension::manifest::ResourceWorkbenchAction {
        operation: "clusterInfo".into(),
    });
    page.stack = vec![ResourceWorkbenchPrimitive::Table(ResourceWorkbenchTable {
        items_path: "/items".into(),
        key_paths: vec!["/id".into()],
        pagination: ResourceWorkbenchPagination {
            kind: ResourceWorkbenchPaginationKind::None,
        },
        columns: vec![],
        open: None,
        actions: vec![ResourceWorkbenchRowAction {
            id: "start".into(),
            label: "Start".into(),
            operation: "missingOperation".into(),
        }],
    })];
    workbench.pages = vec![page];
    manifest.contributes.resource_workbenches.push(workbench);

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("table action references an unknown operation"),
        "{error}"
    );
}
/// 页面 stack 只支持一个内容原语:多原语会被渲染器静默丢弃其中之一,
/// 注册阶段必须拒绝,而不是接受一份"界面不忠实于声明"的工作台。
#[test]
fn runtime_catalog_rejects_page_with_multiple_primitives() {
    let mut manifest = shell_manifest();
    manifest
        .contributes
        .shell_views
        .push(shell_view("ui/explorer.js"));
    manifest.contributes.connections.push(resource_connection());
    let mut workbench = resource_workbench();
    workbench.default_page = "items".into();
    let mut page = resource_workbench().pages.remove(0);
    page.id = "items".into();
    page.stack = vec![
        ResourceWorkbenchPrimitive::Viewer(ResourceWorkbenchViewer {
            format: ResourceWorkbenchViewerFormat::Text,
        }),
        ResourceWorkbenchPrimitive::Stream,
    ];
    workbench.pages = vec![page];
    manifest.contributes.resource_workbenches.push(workbench);

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("page stack must declare exactly one primitive"),
        "{error}"
    );
}

/// 差异资源验收:events 页面与 paging 绑定已经具备完整执行链,
/// 注册阶段必须接受这些声明,而不是拒绝。
#[test]
fn runtime_catalog_accepts_events_pages_and_paging_bindings() {
    let mut manifest = shell_manifest();
    manifest
        .contributes
        .shell_views
        .push(shell_view("ui/explorer.js"));
    manifest.contributes.connections.push(resource_connection());
    let mut workbench = resource_workbench();

    workbench.operations.insert(
        "followLogs".to_string(),
        ResourceWorkbenchOperation {
            mode: ResourceWorkbenchOperationMode::Invoke,
            method: "example/logs/follow".into(),
            requires: vec!["example/logs/follow".into()],
            effect: ResourceWorkbenchEffect::Read,
            params: Default::default(),
        },
    );
    workbench.operations.insert(
        "listItems".to_string(),
        ResourceWorkbenchOperation {
            mode: ResourceWorkbenchOperationMode::Invoke,
            method: "example/item/list".into(),
            requires: vec!["example/item/list".into()],
            effect: ResourceWorkbenchEffect::Read,
            params: [(
                "page".to_string(),
                crate::extension::manifest::ResourceWorkbenchBinding {
                    source: crate::extension::manifest::ResourceWorkbenchBindingSource::Paging,
                    path: "/page".into(),
                    value_type: crate::extension::manifest::ResourceWorkbenchValueType::Number,
                    value: None,
                },
            )]
            .into_iter()
            .collect(),
        },
    );

    let events_page = ResourceWorkbenchPage {
        id: "events".into(),
        title: "Events".into(),
        renderer: ResourceWorkbenchRenderer {
            kind: ResourceWorkbenchRendererKind::Native,
            view_id: None,
            fallback: None,
        },
        load: Some(crate::extension::manifest::ResourceWorkbenchAction {
            operation: "followLogs".into(),
        }),
        tab_group_id: None,
        route: None,
        links: vec![],
        stack: vec![ResourceWorkbenchPrimitive::Stream],
    };
    let collection_page = ResourceWorkbenchPage {
        id: "items".into(),
        title: "Items".into(),
        renderer: ResourceWorkbenchRenderer {
            kind: ResourceWorkbenchRendererKind::Native,
            view_id: None,
            fallback: None,
        },
        load: Some(crate::extension::manifest::ResourceWorkbenchAction {
            operation: "listItems".into(),
        }),
        tab_group_id: None,
        route: None,
        links: vec![],
        stack: vec![ResourceWorkbenchPrimitive::Table(ResourceWorkbenchTable {
            items_path: "/items".into(),
            key_paths: vec!["/id".into()],
            pagination: ResourceWorkbenchPagination {
                kind: ResourceWorkbenchPaginationKind::Cursor,
            },
            columns: vec![],
            open: None,
            actions: vec![],
        })],
    };
    workbench.pages = vec![events_page, collection_page];
    workbench.default_page = "events".into();
    manifest.contributes.resource_workbenches.push(workbench);

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let workbench = catalog
        .resource_workbench_for_connection("com.example.tools", "search")
        .unwrap_or_else(|| panic!("workbench must resolve"));
    assert_eq!(2, workbench.pages.len());
    assert!(matches!(
        workbench.pages[0].stack[0],
        ResourceWorkbenchPrimitive::Stream
    ));
    let table = match &workbench.pages[1].stack[0] {
        ResourceWorkbenchPrimitive::Table(table) => table,
        other => panic!("expected table primitive, got {other:?}"),
    };
    assert_eq!(
        ResourceWorkbenchPaginationKind::Cursor,
        table.pagination.kind
    );
}

fn resource_workbench() -> ResourceWorkbenchContrib {
    ResourceWorkbenchContrib {
        schema_version: 3,
        id: "search-workbench".into(),
        title: "Search".into(),
        connection_ids: vec!["search".into()],
        runtime_id: "provider".into(),
        resource_type: "elasticsearch".into(),
        default_page: "overview".into(),
        operations: [(
            "clusterInfo".into(),
            ResourceWorkbenchOperation {
                mode: ResourceWorkbenchOperationMode::Invoke,
                method: "elasticsearch/cluster/info".into(),
                requires: vec!["elasticsearch/cluster/info".into()],
                effect: ResourceWorkbenchEffect::Read,
                params: Default::default(),
            },
        )]
        .into_iter()
        .collect(),
        layout: None,
        pages: vec![ResourceWorkbenchPage {
            id: "overview".into(),
            title: "Overview".into(),
            renderer: ResourceWorkbenchRenderer {
                kind: ResourceWorkbenchRendererKind::Native,
                view_id: None,
                fallback: None,
            },
            load: Some(crate::extension::manifest::ResourceWorkbenchAction {
                operation: "clusterInfo".into(),
            }),
            tab_group_id: None,
            route: None,
            links: vec![],
            stack: vec![ResourceWorkbenchPrimitive::Viewer(
                ResourceWorkbenchViewer {
                    format: ResourceWorkbenchViewerFormat::Json,
                },
            )],
        }],
    }
}

fn resource_connection() -> ResourceConnectionContrib {
    ResourceConnectionContrib {
        id: "search".into(),
        label: "Search Cluster".into(),
        description: None,
        icon: None,
        runtime_id: "provider".into(),
        resource_type: "elasticsearch".into(),
        shell_view_id: Some("explorer".into()),
        form: ResourceConnectionForm {
            tabs: vec![ResourceConnectionFormTab {
                id: "general".into(),
                label: "General".into(),
                fields: vec![
                    ResourceConnectionFormField {
                        id: "url".into(),
                        label: "URL".into(),
                        field_type: ResourceConnectionFieldType::Text,
                        required: true,
                        default_value: None,
                        placeholder: None,
                        secret: false,
                        options: Vec::new(),
                        visible_when: Vec::new(),
                        rows: None,
                    },
                    ResourceConnectionFormField {
                        id: "api_key".into(),
                        label: "API key".into(),
                        field_type: ResourceConnectionFieldType::Password,
                        required: false,
                        default_value: None,
                        placeholder: None,
                        secret: true,
                        options: Vec::new(),
                        visible_when: Vec::new(),
                        rows: None,
                    },
                ],
            }],
        },
    }
}

#[test]
fn runtime_catalog_rejects_shell_view_with_unknown_backend() {
    let mut manifest = shell_manifest();
    let mut view = shell_view("ui/explorer.js");
    view.backends
        .insert("search".to_string(), "missing".to_string());
    manifest.contributes.shell_views.push(view);

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(error.to_string().contains("unknown IPC runtime"), "{error}");
}

#[test]
fn runtime_catalog_rejects_shell_view_without_shell_exec_permission() {
    let mut manifest = shell_manifest();
    manifest
        .permissions
        .retain(|permission| permission != "shell:exec");
    manifest
        .contributes
        .shell_views
        .push(shell_view("ui/explorer.js"));

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(error.to_string().contains("shell:exec"), "{error}");
}

#[test]
fn runtime_catalog_rejects_shell_view_with_reserved_backend_alias() {
    let mut manifest = shell_manifest();
    let mut view = shell_view("ui/explorer.js");
    view.backends.clear();
    view.backends
        .insert("navop".to_string(), "provider".to_string());
    manifest.contributes.shell_views.push(view);

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(error.to_string().contains("reserved"), "{error}");
}

#[test]
fn runtime_catalog_rejects_shell_view_entry_escape() {
    let mut manifest = shell_manifest();
    manifest
        .contributes
        .shell_views
        .push(shell_view("../escape.js"));

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(error.to_string().contains("entry"), "{error}");
}

#[test]
fn runtime_catalog_registers_resolved_ipc_runtime_binding() {
    let mut manifest = base_manifest();
    manifest.permissions = vec![
        "net:tcp:127.0.0.1:9092".to_string(),
        "secrets:read:kafka.*".to_string(),
        "spawn:./runtime/bin/provider".to_string(),
    ];
    manifest.runtime.ipc.push(IpcRuntime {
        id: "main".to_string(),
        entry: IpcEntry {
            command: "./bin/provider".to_string(),
            args: vec!["--mode".to_string(), "kafka".to_string()],
            working_dir: Some("runtime".to_string()),
            env: std::collections::BTreeMap::from([("RUST_LOG".to_string(), "info".to_string())]),
        },
        transport: IpcTransport {
            kind: "local_socket".to_string(),
            connect_timeout_ms: Some(7_500),
        },
        auto_restart: true,
        max_restart_attempts: 5,
        shutdown_grace_ms: 2_500,
    });

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let binding = catalog.ipc_runtime_bindings().next().unwrap();

    assert_eq!("com.example.tools::main", binding.runtime_key);
    assert_eq!(
        PathBuf::from("/tmp/com.example.tools/runtime/bin/provider"),
        binding.command
    );
    assert_eq!(
        Some(PathBuf::from("/tmp/com.example.tools/runtime")),
        binding.working_dir
    );
    assert_eq!(vec!["--mode", "kafka"], binding.args);
    assert_eq!(
        Some("info"),
        binding.env.get("RUST_LOG").map(String::as_str)
    );
    assert_eq!("local_socket", binding.transport_kind);
    assert_eq!(Some(7_500), binding.connect_timeout_ms);
    assert!(binding.auto_restart);
    assert_eq!(5, binding.max_restart_attempts);
    assert_eq!(2_500, binding.shutdown_grace_ms);
    assert_eq!(
        vec![
            "net:tcp:127.0.0.1:9092",
            "secrets:read:kafka.*",
            "spawn:./runtime/bin/provider"
        ],
        binding.permissions
    );
}

#[test]
fn runtime_catalog_preserves_allowlisted_absolute_ipc_command() {
    let mut manifest = base_manifest();
    manifest.permissions.push("spawn:/usr/bin/env".to_string());
    manifest.runtime.ipc.push(IpcRuntime {
        id: "main".to_string(),
        entry: IpcEntry {
            command: "/usr/bin/env".to_string(),
            args: vec!["python3".to_string(), "./provider.py".to_string()],
            working_dir: None,
            env: std::collections::BTreeMap::new(),
        },
        transport: IpcTransport::default(),
        auto_restart: false,
        max_restart_attempts: 0,
        shutdown_grace_ms: 1_000,
    });

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let binding = catalog.ipc_runtime_bindings().next().unwrap();

    assert_eq!(PathBuf::from("/usr/bin/env"), binding.command);
    assert_eq!(
        Some(PathBuf::from("/tmp/com.example.tools")),
        binding.working_dir
    );
}

#[test]
fn runtime_catalog_rejects_remote_editor_without_program_candidates() {
    let mut manifest = base_manifest();
    manifest
        .contributes
        .remote_file_editors
        .push(RemoteFileEditorContrib {
            id: "broken".to_string(),
            display_name: "Broken Editor".to_string(),
            platforms: Vec::new(),
            file_masks: Vec::new(),
            priority: 0,
            command: RemoteFileEditorCommandContrib::default(),
        });

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(error.to_string().contains("programCandidates"));
}

#[test]
fn runtime_catalog_loads_compatible_extensions_from_composite_root() {
    let root = tempfile::TempDir::new().unwrap();
    write_composite_manifest(
        root.path(),
        "com.example.echo",
        r#"{
            "schema_version": 1,
            "id": "com.example.echo",
            "name": "Echo",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.1.0" },
            "runtime": {
                "wasm": [{
                    "id": "main",
                    "module": "./wasm/plugin.wasm",
                    "kind": "component"
                }]
            },
            "contributes": {
                "commands": [{
                    "id": "example.echo",
                    "title": "Echo",
                    "handler": {
                        "kind": "wasm",
                        "runtime_id": "main"
                    }
                }]
            }
        }"#,
    );
    write_composite_manifest(
        root.path(),
        "com.example.echo.backup-0.0.9",
        r#"{
            "schema_version": 1,
            "id": "com.example.echo",
            "name": "Echo Backup",
            "version": "0.0.9",
            "engines": { "onetcli": ">=0.1.0" },
            "runtime": {
                "wasm": [{
                    "id": "main",
                    "module": "./wasm/plugin.wasm",
                    "kind": "component"
                }]
            }
        }"#,
    );
    write_composite_manifest(
        root.path(),
        ".com.example.echo.install-backup-1-0",
        r#"{
            "schema_version": 1,
            "id": "com.example.echo",
            "name": "Echo Transaction Backup",
            "version": "0.0.8",
            "engines": { "onetcli": ">=0.1.0" },
            "runtime": {
                "wasm": [{
                    "id": "main",
                    "module": "./wasm/plugin.wasm",
                    "kind": "component"
                }]
            }
        }"#,
    );
    std::fs::create_dir_all(root.path().join("_staging")).unwrap();
    std::fs::create_dir_all(root.path().join("noise")).unwrap();

    let report =
        ExtensionRuntimeCatalog::from_installed_composite_root_with_report(root.path()).unwrap();
    let catalog = report.catalog;

    assert!(
        catalog
            .component_permissions_for_command("example.echo")
            .is_ok()
    );
    assert_eq!(report.loaded.len(), 1);
    assert_eq!(report.loaded[0].id, "com.example.echo");
    assert_eq!(
        report.loaded[0].wasm_runtimes,
        vec!["com.example.echo::main".to_string()]
    );
}

#[test]
fn runtime_catalog_rebuild_after_uninstall_drops_db_tree_menu() {
    let root = tempfile::TempDir::new().unwrap();
    write_composite_manifest(
        root.path(),
        "com.example.cleanup",
        r#"{
            "schema_version": 1,
            "id": "com.example.cleanup",
            "name": "Cleanup",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.1.0" },
            "runtime": {
                "wasm": [{
                    "id": "main",
                    "module": "./wasm/plugin.wasm",
                    "kind": "component"
                }]
            },
            "contributes": {
                "commands": [{
                    "id": "cleanup.run",
                    "title": "Cleanup",
                    "handler": {
                        "kind": "wasm",
                        "runtime_id": "main"
                    }
                }],
                "menus": {
                    "db.tree.table": [{
                        "command": "cleanup.run",
                        "label": "Cleanup",
                        "group": "extension@10"
                    }]
                }
            }
        }"#,
    );

    let before = ExtensionRuntimeCatalog::from_installed_composite_root(root.path()).unwrap();
    assert_eq!(
        1,
        before
            .db_tree_menu_registry()
            .items_for_node(db::DbNodeType::Table)
            .len()
    );

    std::fs::remove_dir_all(root.path().join("com.example.cleanup")).unwrap();
    let after = ExtensionRuntimeCatalog::from_installed_composite_root(root.path()).unwrap();

    assert!(
        after
            .db_tree_menu_registry()
            .items_for_node(db::DbNodeType::Table)
            .is_empty()
    );
}

fn shell_manifest() -> Manifest {
    let mut manifest = base_manifest();
    manifest.permissions = vec![
        "shell:exec".to_string(),
        "spawn:./bin/provider".to_string(),
        "secrets:read:self.*".to_string(),
    ];
    manifest.runtime.ipc.push(IpcRuntime {
        id: "provider".to_string(),
        entry: IpcEntry {
            command: "./bin/provider".to_string(),
            args: Vec::new(),
            working_dir: None,
            env: std::collections::BTreeMap::new(),
        },
        transport: IpcTransport::default(),
        auto_restart: true,
        max_restart_attempts: 3,
        shutdown_grace_ms: 1_000,
    });
    manifest
}

fn shell_view(entry: &str) -> ShellViewContrib {
    ShellViewContrib {
        id: "explorer".to_string(),
        title: "Example Explorer".to_string(),
        description: None,
        icon: None,
        entry: entry.to_string(),
        surface: ShellSurface::Tab,
        category: None,
        keywords: None,
        singleton: false,
        backends: std::collections::BTreeMap::from([(
            "search".to_string(),
            "provider".to_string(),
        )]),
        modules: vec![ShellHostModule::Context, ShellHostModule::Resource],
    }
}

fn wasm_runtime(id: &str) -> WasmRuntime {
    WasmRuntime {
        id: id.to_string(),
        module: "./wasm/plugin.wasm".to_string(),
        kind: WasmRuntimeKind::Component,
        timeout_ms: 5_000,
        max_memory_mb: 64,
        fuel_per_call: 100_000_000,
    }
}

fn command(runtime_id: &str, command_id: &str) -> CommandContrib {
    CommandContrib {
        id: command_id.to_string(),
        title: "Sync Table".to_string(),
        category: String::new(),
        icon: None,
        enablement_when: Some("node.type == 'table'".to_string()),
        handler: CommandHandlerContrib {
            kind: "wasm".to_string(),
            runtime_id: runtime_id.to_string(),
            function: Some("run".to_string()),
        },
    }
}

fn base_manifest() -> Manifest {
    Manifest {
        schema_version: 1,
        id: "com.example.tools".to_string(),
        name: "Example Tools".to_string(),
        version: "0.1.0".to_string(),
        publisher: String::new(),
        license: String::new(),
        homepage: String::new(),
        repository: String::new(),
        icon: String::new(),
        description_i18n: String::new(),
        description: String::new(),
        categories: vec![],
        keywords: vec![],
        engines: Engines {
            onetcli: ">=0.1.0".to_string(),
            gpui_shell: "0.6.4".to_string(),
        },
        api: ApiVersions::default(),
        activation: vec![],
        permissions: vec!["db:schema:*".to_string(), "ui:notify".to_string()],
        runtime: RuntimeSection::default(),
        contributes: ContributesManifest::default(),
        manifest_dir: PathBuf::from("/tmp/com.example.tools"),
    }
}

fn write_composite_manifest(root: &std::path::Path, id: &str, content: &str) {
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.json"), content).unwrap();
}

#[test]
fn catalog_toolbox_views_exclude_connection_owned_shell_views() {
    // 独立工具(tab surface + toolbox surface,未被连接引用)进工具箱;
    // 连接 shellViewId 引用的视图不进。
    let mut standalone_tab = shell_view("ui/standalone.js");
    standalone_tab.id = "standalone-tool".into();
    standalone_tab.singleton = true;
    standalone_tab.surface = crate::extension::manifest::ShellSurface::Tab;

    let mut standalone_toolbox = shell_view("ui/toolbox.js");
    standalone_toolbox.id = "toolbox-tool".into();
    standalone_toolbox.surface = crate::extension::manifest::ShellSurface::Toolbox;

    let mut owned = shell_view("ui/explorer.js");
    owned.id = "explorer".into();

    let mut manifest = shell_manifest();
    manifest.contributes.shell_views.push(standalone_tab);
    manifest.contributes.shell_views.push(standalone_toolbox);
    manifest.contributes.shell_views.push(owned);
    manifest.contributes.connections.push(resource_connection());

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let tools = catalog.toolbox_views();
    let ids: Vec<&str> = tools.iter().map(|view| view.id.as_str()).collect();
    assert!(
        ids.contains(&"standalone-tool"),
        "独立 tab 工具应进工具箱: {ids:?}"
    );
    assert!(
        ids.contains(&"toolbox-tool"),
        "toolbox surface 工具应进工具箱: {ids:?}"
    );
    assert!(
        !ids.contains(&"explorer"),
        "连接关联的 shell view 不应进工具箱: {ids:?}"
    );
}

#[test]
fn catalog_toolbox_views_exclude_workbench_page_bodies() {
    // 工作台页体(被 `pages[*].renderer.viewId` 引用的视图)与任何声明
    // `workbench` 模块的视图,都只在工作台挂载会话里可运行;独立打开时
    // 宿主必然失败("navop.workbench requires a borrowed resource-workbench
    // session"),列进工具箱就是点了就报错的假卡片。
    let mut page_body = shell_view("ui/page-body.js");
    page_body.id = "page-body".into();
    page_body.modules = vec![ShellHostModule::Context, ShellHostModule::Workbench];

    // 没有被任何页面引用,但同样声明了 workbench 模块 —— 同样独立打不开。
    let mut embedded_only = shell_view("ui/embedded-only.js");
    embedded_only.id = "embedded-only".into();
    embedded_only.modules = vec![ShellHostModule::Context, ShellHostModule::Workbench];

    // 真独立工具:context + resource,不经工作台。
    let mut standalone = shell_view("ui/standalone.js");
    standalone.id = "standalone-tool".into();

    let mut manifest = shell_manifest();
    manifest.contributes.shell_views.push(page_body);
    manifest.contributes.shell_views.push(embedded_only);
    manifest.contributes.shell_views.push(standalone);
    // 本用例只关心「页体/嵌入专用视图」的排除,连接一律不挂 shell 视图。
    let mut connection = resource_connection();
    connection.shell_view_id = None;
    manifest.contributes.connections.push(connection);

    let mut workbench = resource_workbench();
    workbench.pages[0].renderer.kind = ResourceWorkbenchRendererKind::Shell;
    workbench.pages[0].renderer.view_id = Some("page-body".into());
    workbench.pages[0].renderer.fallback = Some("native".into());
    manifest.contributes.resource_workbenches.push(workbench);

    let catalog = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap();
    let ids: Vec<&str> = catalog
        .toolbox_views()
        .iter()
        .map(|view| view.id.as_str())
        .collect();
    assert!(
        !ids.contains(&"page-body"),
        "工作台页体不应作为独立工具进工具箱: {ids:?}"
    );
    assert!(
        !ids.contains(&"embedded-only"),
        "声明 workbench 模块的视图不应进工具箱: {ids:?}"
    );
    assert!(
        ids.contains(&"standalone-tool"),
        "独立工具仍应进工具箱: {ids:?}"
    );
}

/// `terminal.operation` 是**预留声明**:扩展协议目前只有请求-响应与 job
/// 两种形态,没有 provider PTY 流式通道,宿主侧必然返回
/// "runtime terminal operation ... is not supported yet"。
///
/// 回归:注册期只校验 operation 存在就放行,于是扩展能装成功、用户点开
/// 必定失败 —— 失败发生在使用而不是安装。这里固定住"在校验期拒绝"。
#[test]
fn runtime_catalog_rejects_terminal_operation_declaration() {
    let mut manifest = shell_manifest();
    manifest
        .contributes
        .shell_views
        .push(shell_view("ui/explorer.js"));
    manifest.contributes.connections.push(resource_connection());

    let mut workbench = resource_workbench();
    // operation 存在,所以拒绝原因必须是"未实现"而不是"未知操作"。
    workbench.operations.insert(
        "openShell".to_string(),
        ResourceWorkbenchOperation {
            mode: ResourceWorkbenchOperationMode::Invoke,
            method: "example/exec/open".into(),
            requires: vec!["example/exec/open".into()],
            effect: ResourceWorkbenchEffect::Read,
            params: Default::default(),
        },
    );
    workbench.pages[0].load = None;
    workbench.pages[0].stack = vec![ResourceWorkbenchPrimitive::Terminal(
        ResourceWorkbenchTerminal {
            command: None,
            args: vec![],
            env: Default::default(),
            working_dir: None,
            operation: Some(ResourceWorkbenchTerminalOperation {
                operation: "openShell".into(),
            }),
        },
    )];
    manifest.contributes.resource_workbenches.push(workbench);

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(
        error.to_string().contains("terminal.operation is reserved"),
        "{error}"
    );
}

/// 反向守卫:本地 command 形式的终端是**已实现**的,不能被上面的拒绝误伤。
#[test]
fn runtime_catalog_accepts_local_command_terminal() {
    let mut manifest = shell_manifest();
    manifest
        .contributes
        .shell_views
        .push(shell_view("ui/explorer.js"));
    manifest.contributes.connections.push(resource_connection());

    let mut workbench = resource_workbench();
    workbench.pages[0].load = None;
    workbench.pages[0].stack = vec![ResourceWorkbenchPrimitive::Terminal(
        ResourceWorkbenchTerminal {
            command: Some("docker".into()),
            args: vec!["exec".into(), "-it".into(), "{{id}}".into(), "sh".into()],
            env: Default::default(),
            working_dir: None,
            operation: None,
        },
    )];
    manifest.contributes.resource_workbenches.push(workbench);

    let catalog =
        ExtensionRuntimeCatalog::from_manifests(vec![manifest]).expect("command terminal is valid");
    let page = &catalog
        .resource_workbench_for_connection("com.example.tools", "search")
        .expect("workbench must resolve")
        .pages[0];
    assert!(matches!(
        page.stack[0],
        ResourceWorkbenchPrimitive::Terminal(_)
    ));
}

/// route 绑定只接受导航那一刻取得到值的来源:`input`/`paging` 在行点击
/// 与链接导航中既没有表单输入也没有列表上下文,声明了只会静默产出空值。
#[test]
fn runtime_catalog_rejects_route_binding_with_input_source() {
    let mut manifest = shell_manifest();
    manifest
        .contributes
        .shell_views
        .push(shell_view("ui/explorer.js"));
    manifest.contributes.connections.push(resource_connection());

    let mut workbench = resource_workbench();
    workbench.default_page = "items".into();
    let mut page = resource_workbench().pages.remove(0);
    page.id = "items".into();
    page.load = None;
    page.stack = vec![ResourceWorkbenchPrimitive::Table(ResourceWorkbenchTable {
        items_path: "/items".into(),
        key_paths: vec!["/id".into()],
        pagination: ResourceWorkbenchPagination {
            kind: ResourceWorkbenchPaginationKind::None,
        },
        columns: vec![],
        open: Some(ResourceWorkbenchOpen {
            page_id: "items".into(),
            route: [(
                "id".to_string(),
                ResourceWorkbenchBinding {
                    source: ResourceWorkbenchBindingSource::Input,
                    path: "/id".into(),
                    value_type: ResourceWorkbenchValueType::String,
                    value: None,
                },
            )]
            .into_iter()
            .collect(),
        }),
        actions: vec![],
    })];
    workbench.pages = vec![page];
    manifest.contributes.resource_workbenches.push(workbench);

    let error = ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("table open route binding `id` uses an input/paging source"),
        "{error}"
    );
}
