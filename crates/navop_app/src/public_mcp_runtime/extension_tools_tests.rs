//! 扩展工具接入 Agent 注册表的契约测试。
//!
//! 覆盖设计文档 plugin-ai-tools.md §5.2：
//! `tool_exposure.agent.extensions` 打开时，catalog 里的 `contributes.tools`
//! 以 `ext_<ns>_<id>` 名字进 Agent 注册表；开关关闭（默认）或 global
//! service 缺失时不注入、也不报错。

use super::agent_runtime_tool_registry;
use crate::public_mcp_runtime::tool_registry::tests_helpers;
use one_core::settings::ToolExposureToolsetSettings;

const TOOL_MANIFEST: &str = r#"{
    "schema_version": 1,
    "id": "com.navop.docker",
    "name": "Docker",
    "version": "0.1.0",
    "engines": { "onetcli": ">=0.4.0", "nsh": "0.3.0" },
    "permissions": ["shell:exec", "spawn:./bin/provider"],
    "runtime": {
        "ipc": [{
            "id": "main",
            "entry": { "command": "./bin/provider" },
            "transport": { "kind": "local_socket" }
        }]
    },
    "contributes": {
        "shellApp": {
            "entry": "./app/index.html",
            "backends": { "main": "main" },
            "modules": ["resource"]
        },
        "tools": [{
            "id": "containers.list",
            "title": "容器列表",
            "description": "列出当前连接的容器",
            "backend": "main",
            "method": "containers/list",
            "risk": "read"
        }]
    }
}"#;

#[gpui::test]
fn agent_registry_includes_extension_tools_when_enabled(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        tests_helpers::register_connection_repository(cx);
        tests_helpers::install_extension_catalog(cx, TOOL_MANIFEST);

        let mut settings = one_core::settings::AppSettings::default();
        settings.tool_exposure.agent = ToolExposureToolsetSettings {
            extensions: true,
            ..settings.tool_exposure.agent
        };
        cx.set_global(settings);

        let registry = agent_runtime_tool_registry(cx).expect("agent registry should build");
        let names: Vec<String> = registry
            .names()
            .into_iter()
            .map(|name| name.to_string())
            .collect();
        assert!(
            names.iter().any(|name| name.contains("containers_list")),
            "扩展工具应进注册表: {names:?}"
        );
    });
}

#[gpui::test]
fn agent_registry_skips_extension_tools_by_default(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        tests_helpers::register_connection_repository(cx);
        tests_helpers::install_extension_catalog(cx, TOOL_MANIFEST);
        // 不开 extensions 开关（默认 false）。
        cx.set_global(one_core::settings::AppSettings::default());

        let registry = agent_runtime_tool_registry(cx).expect("agent registry should build");
        let names: Vec<String> = registry
            .names()
            .into_iter()
            .map(|name| name.to_string())
            .collect();
        assert!(
            !names.iter().any(|name| name.contains("containers_list")),
            "默认关闭时扩展工具不应出现: {names:?}"
        );
    });
}

#[gpui::test]
fn agent_registry_tolerates_missing_extension_service(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        tests_helpers::register_connection_repository(cx);
        let mut settings = one_core::settings::AppSettings::default();
        settings.tool_exposure.agent = ToolExposureToolsetSettings {
            extensions: true,
            ..settings.tool_exposure.agent
        };
        cx.set_global(settings);
        // 没有 GlobalUniversalPluginService 也不炸。

        let registry = agent_runtime_tool_registry(cx).expect("agent registry should build");
        let names: Vec<String> = registry
            .names()
            .into_iter()
            .map(|name| name.to_string())
            .collect();
        assert!(
            !names.iter().any(|name| name.starts_with("ext_")),
            "无 service 时不应有扩展工具: {names:?}"
        );
    });
}
