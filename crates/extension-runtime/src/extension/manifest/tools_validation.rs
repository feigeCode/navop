use std::collections::HashSet;

use serde_json::Value;

use super::{Manifest, ToolContrib};

/// `contributes.tools` 安装期校验。
///
/// 路径风格与 `shell_validation.rs` 一致：错误带 `/contributes/tools/<id>/<field>`
/// 风格的字段路径，安装期直接拒绝，不把坏声明留到运行期。
pub(crate) fn validate_tools(manifest: &Manifest) -> Result<(), ToolsValidationError> {
    if manifest.contributes.tools.is_empty() {
        return Ok(());
    }
    let Some(app) = manifest.contributes.shell_app.as_ref() else {
        return Err(error(
            "/contributes/tools",
            "声明工具的扩展必须同时声明 shellApp（工具是 app 的能力，不允许只有工具没有 UI）",
        ));
    };
    let mut seen_ids = HashSet::new();
    for tool in &manifest.contributes.tools {
        validate_tool(manifest, app, tool, &mut seen_ids)?;
    }
    Ok(())
}

fn validate_tool(
    manifest: &Manifest,
    app: &super::ShellAppContrib,
    tool: &ToolContrib,
    seen_ids: &mut HashSet<String>,
) -> Result<(), ToolsValidationError> {
    let field = |name: &str| format!("/contributes/tools/{}/{}", tool.id, name);

    if !is_valid_tool_id(&tool.id) {
        return Err(error(
            field("id"),
            "id 只能包含小写字母、数字、点、下划线、连字符，长度 1–64，且不能以 .-_ 开头或结尾",
        ));
    }
    if !seen_ids.insert(tool.id.clone()) {
        return Err(error(field("id"), "同一扩展内的工具 id 不能重复"));
    }
    if tool.title.trim().is_empty() {
        return Err(error(field("title"), "title 不能为空"));
    }
    if tool.title.chars().count() > 40 {
        return Err(error(field("title"), "title 长度不能超过 40"));
    }
    if tool.description.trim().is_empty() {
        return Err(error(field("description"), "description 不能为空"));
    }
    if tool.description.chars().count() > 1024 {
        return Err(error(field("description"), "description 长度不能超过 1024"));
    }
    if tool.method.trim().is_empty() {
        return Err(error(field("method"), "method 不能为空"));
    }
    if tool.method.chars().count() > 128 {
        return Err(error(field("method"), "method 长度不能超过 128"));
    }
    let Some(backend_runtime) = app.backends.get(&tool.backend) else {
        return Err(error(
            field("backend"),
            format!(
                "backend `{}` 必须是 shellApp.backends 里声明的别名",
                tool.backend
            ),
        ));
    };
    if tool.connection {
        validate_connection_target(manifest, tool, backend_runtime, &field)?;
    }
    validate_input_schema(&tool.input_schema, &field)?;
    if !(1000..=300_000).contains(&tool.timeout_ms) {
        return Err(error(
            field("timeoutMs"),
            "timeoutMs 必须在 1000–300000 之间",
        ));
    }
    if !(4096..=1_048_576).contains(&tool.max_result_bytes) {
        return Err(error(
            field("maxResultBytes"),
            "maxResultBytes 必须在 4096–1048576 之间",
        ));
    }
    Ok(())
}

/// `connection == true` 的工具：扩展必须至少有一条非 terminal 连接，且其
/// `runtimeId` 能被声明的 backend 指向。
fn validate_connection_target(
    manifest: &Manifest,
    tool: &ToolContrib,
    backend_runtime: &str,
    field: &dyn Fn(&str) -> String,
) -> Result<(), ToolsValidationError> {
    let has_match = manifest
        .contributes
        .connections
        .iter()
        // terminal 型连接没有 provider，工具不能拿它当目标。
        .filter(|conn| conn.terminal.is_none())
        .any(|conn| conn.runtime_id == backend_runtime);
    if !has_match {
        return Err(error(
            field("connection"),
            format!(
                "connection=true 的工具 `{}` 要求扩展至少有一条非 terminal 连接，\
                 且该连接的 runtimeId 能被 backend `{}` 指向",
                tool.id, tool.backend
            ),
        ));
    }
    Ok(())
}

/// inputSchema 受限子集校验（§6.3）：不同模型对 JSON Schema 支持参差，
/// 安装期就把 schema 收窄到 function-calling 稳定支持的子集。
fn validate_input_schema(
    schema: &Value,
    field: &dyn Fn(&str) -> String,
) -> Result<(), ToolsValidationError> {
    let Some(obj) = schema.as_object() else {
        return Err(error(field("inputSchema"), "inputSchema 必须是对象"));
    };
    for key in obj.keys() {
        if !matches!(
            key.as_str(),
            "type" | "properties" | "required" | "title" | "description"
        ) {
            return Err(error(
                field("inputSchema"),
                format!(
                    "inputSchema 顶层不支持 `{key}`；只允许 type/properties/required/title/description"
                ),
            ));
        }
    }
    if let Some(ty) = obj.get("type") {
        if ty.as_str() != Some("object") {
            return Err(error(
                field("inputSchema"),
                "inputSchema.type 必须是 \"object\"",
            ));
        }
    }
    if let Some(required) = obj.get("required") {
        let Some(items) = required.as_array() else {
            return Err(error(field("inputSchema"), "required 必须是字符串数组"));
        };
        if items.iter().any(|item| !item.is_string()) {
            return Err(error(field("inputSchema"), "required 必须是字符串数组"));
        }
    }
    let Some(properties) = obj.get("properties").and_then(Value::as_object) else {
        return Ok(());
    };
    let declared: HashSet<&str> = obj
        .get("required")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    for (name, prop) in properties {
        validate_property(name, prop, &declared, field)?;
    }
    Ok(())
}

const ALLOWED_PROPERTY_KEYS: &[&str] = &[
    "type",
    "description",
    "enum",
    "default",
    "minimum",
    "maximum",
    "minLength",
    "maxLength",
    "items",
    "format",
];

const PRIMITIVE_TYPES: &[&str] = &["string", "number", "integer", "boolean"];
const ALLOWED_FORMATS: &[&str] = &["date-time", "uri", "email"];

fn validate_property(
    name: &str,
    prop: &Value,
    required: &HashSet<&str>,
    field: &dyn Fn(&str) -> String,
) -> Result<(), ToolsValidationError> {
    let prop_field = |key: &str| format!("{}/properties/{}", field("inputSchema"), format_key(name, key));
    if !prop.is_object() {
        return Err(error(
            prop_field(""),
            format!("属性 `{name}` 必须是对象"),
        ));
    }
    let obj = prop.as_object().expect("checked above");
    for key in obj.keys() {
        if !ALLOWED_PROPERTY_KEYS.contains(&key.as_str()) {
            return Err(error(
                prop_field(key),
                format!(
                    "属性 `{name}` 不支持 `{key}`；只允许 {}（嵌套对象/组合关键词在 v1 不支持）",
                    ALLOWED_PROPERTY_KEYS.join("/")
                ),
            ));
        }
    }
    // 属性必须声明 type，且 required 里的属性必须真的在 properties 里声明。
    let Some(ty) = obj.get("type").and_then(Value::as_str) else {
        return Err(error(
            prop_field("type"),
            format!("属性 `{name}` 必须声明 type"),
        ));
    };
    if !PRIMITIVE_TYPES.contains(&ty) && ty != "array" {
        return Err(error(
            prop_field("type"),
            format!(
                "属性 `{name}` 的 type 只能是 string/number/integer/boolean/array；\
                 嵌套 object 在 v1 不支持，复杂数据请让模型传字符串或数组"
            ),
        ));
    }
    if let Some(format) = obj.get("format").and_then(Value::as_str) {
        if !ALLOWED_FORMATS.contains(&format) {
            return Err(error(
                prop_field("format"),
                format!("format 只支持 date-time/uri/email"),
            ));
        }
    }
    if ty == "array" {
        match obj.get("items") {
            None => {
                return Err(error(
                    prop_field("items"),
                    format!("数组属性 `{name}` 必须声明 items（仅原始类型）"),
                ));
            }
            Some(items) => {
                let Some(item_ty) = items.get("type").and_then(Value::as_str) else {
                    return Err(error(
                        prop_field("items"),
                        format!("属性 `{name}` 的 items 必须声明原始类型的 type"),
                    ));
                };
                if !PRIMITIVE_TYPES.contains(&item_ty) {
                    return Err(error(
                        prop_field("items"),
                        format!("属性 `{name}` 的 items.type 只能是 string/number/integer/boolean"),
                    ));
                }
                for key in items.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default() {
                    if !matches!(key.as_str(), "type" | "description") {
                        return Err(error(
                            prop_field("items"),
                            format!("属性 `{name}` 的 items 只支持 type/description"),
                        ));
                    }
                }
            }
        }
    }
    let _ = required;
    Ok(())
}
fn format_key(name: &str, key: &str) -> String {
    if key.is_empty() {
        name.to_string()
    } else {
        format!("{name}/{key}")
    }
}

fn is_valid_tool_id(id: &str) -> bool {
    id.len() <= 64
        && !id.is_empty()
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".-_".contains(&b))
        && !id.starts_with(['.', '-', '_'])
        && !id.ends_with(['.', '-', '_'])
}
fn error(field: impl Into<String>, reason: impl Into<String>) -> ToolsValidationError {
    ToolsValidationError {
        field: field.into(),
        reason: reason.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolsValidationError {
    pub field: String,
    pub reason: String,
}

#[cfg(test)]
mod tests {    use serde_json::json;

    use super::*;

    /// 造一个带合法 tools 声明的清单；manifest_dir 为空路径（exists()=false），
    /// 校验不依赖磁盘。tools 传 serde_json::Value（数组），connections 同。
    fn manifest_with(tools: Value, connections: Value) -> Manifest {
        serde_json::from_value(json!({
            "schema_version": 1,
            "id": "com.example.docker",
            "name": "测试扩展",
            "version": "0.1.0",
            "engines": { "onetcli": ">=0.1.0", "nsh": "0.3.0" },
            "permissions": ["shell:exec"],
            "runtime": {
                "ipc": [{ "id": "provider", "entry": { "command": "./bin/provider" } }]
            },
            "contributes": {
                "shellApp": {
                    "title": "测试 app",
                    "entry": "ui/dist/bundle.js",
                    "modules": ["resource"],
                    "backends": { "docker": "provider" }
                },
                "connections": connections,
                "tools": tools
            }
        }))
        .expect("测试清单必须能被反序列化")
    }

    fn valid_tool() -> Value {
        json!({
            "id": "containers.list",
            "title": "列出容器",
            "description": "列出当前连接上的容器。",
            "backend": "docker",
            "method": "docker/containers/list",
            "risk": "read",
            "connection": true
        })
    }

    fn non_terminal_connection() -> Value {
        json!([{
            "id": "docker-local",
            "label": "Docker",
            "runtimeId": "provider",
            "resourceType": "docker"
        }])
    }

    fn validate_error(manifest: &Manifest) -> ToolsValidationError {
        validate_tools(manifest).expect_err("应当被安装期校验拒绝")
    }

    #[test]
    fn valid_tools_pass() {
        let manifest = manifest_with(
            json!([valid_tool(), {
                "id": "containers.stop",
                "title": "停止容器",
                "description": "停止一个容器。",
                "backend": "docker",
                "method": "docker/containers/stop",
                "risk": "medium",
                "connection": true
            }]),
            non_terminal_connection(),
        );
        validate_tools(&manifest).expect("合法声明应通过");
    }

    #[test]
    fn tools_without_shell_app_are_rejected() {
        let mut manifest = manifest_with(json!([valid_tool()]), non_terminal_connection());
        manifest.contributes.shell_app = None;
        let error = validate_error(&manifest);
        assert_eq!("/contributes/tools", error.field);
        assert!(error.reason.contains("shellApp"), "{}", error.reason);
    }

    #[test]
    fn duplicate_tool_ids_are_rejected() {
        let manifest = manifest_with(json!([valid_tool(), valid_tool()]), non_terminal_connection());
        let error = validate_error(&manifest);
        assert_eq!("/contributes/tools/containers.list/id", error.field);
        assert!(error.reason.contains("重复"), "{}", error.reason);
    }

    #[test]
    fn invalid_id_characters_are_rejected() {
        let mut tool = valid_tool();
        tool["id"] = json!("Bad Id");
        let manifest = manifest_with(json!([tool]), non_terminal_connection());
        let error = validate_error(&manifest);
        assert_eq!("/contributes/tools/Bad Id/id", error.field);
    }

    #[test]
    fn overly_long_fields_are_rejected() {
        let mut tool = valid_tool();
        tool["title"] = json!("x".repeat(41));
        let manifest = manifest_with(json!([tool]), non_terminal_connection());
        assert_eq!(
            validate_error(&manifest).field,
            "/contributes/tools/containers.list/title"
        );

        let mut tool = valid_tool();
        tool["description"] = json!("x".repeat(1025));
        let manifest = manifest_with(json!([tool]), non_terminal_connection());
        assert_eq!(
            validate_error(&manifest).field,
            "/contributes/tools/containers.list/description"
        );
    }

    #[test]
    fn unknown_backend_is_rejected() {
        let mut tool = valid_tool();
        tool["backend"] = json!("nope");
        let manifest = manifest_with(json!([tool]), non_terminal_connection());
        let error = validate_error(&manifest);
        assert_eq!("/contributes/tools/containers.list/backend", error.field);
        assert!(error.reason.contains("nope"), "{}", error.reason);
    }

    #[test]
    fn connection_tool_requires_a_provider_connection() {
        // backend 指向的 runtime 没有任何非 terminal 连接声明。
        let mut tool = valid_tool();
        tool["connection"] = json!(true);
        let manifest = manifest_with(json!([tool]), json!([]));
        let error = validate_error(&manifest);
        assert_eq!("/contributes/tools/containers.list/connection", error.field);
        assert!(error.reason.contains("terminal"), "{}", error.reason);
    }

    #[test]
    fn connection_tool_accepts_empty_flag_without_connections() {
        // connection=false 的工具不需要连接声明。
        let mut tool = valid_tool();
        tool["connection"] = json!(false);
        let manifest = manifest_with(json!([tool]), json!([]));
        validate_tools(&manifest).expect("connection=false 不要求连接声明");
    }

    #[test]
    fn terminal_connections_do_not_satisfy_connection_tools() {
        let mut tool = valid_tool();
        tool["connection"] = json!(true);
        let terminal = json!([{
            "id": "shell",
            "label": "Shell",
            "terminal": { "command": "sh" }
        }]);
        let manifest = manifest_with(json!([tool]), terminal);
        let error = validate_error(&manifest);
        assert_eq!("/contributes/tools/containers.list/connection", error.field);
    }

    #[test]
    fn schema_top_level_keywords_are_limited() {
        let mut tool = valid_tool();
        tool["inputSchema"] = json!({
            "type": "object",
            "properties": {},
            "$ref": "#/definitions/x"
        });
        let manifest = manifest_with(json!([tool]), non_terminal_connection());
        let error = validate_error(&manifest);
        assert!(error.field.contains("inputSchema"), "{}", error.field);
        assert!(error.reason.contains("$ref"), "{}", error.reason);
    }

    #[test]
    fn schema_property_keywords_are_limited() {
        let mut tool = valid_tool();
        tool["inputSchema"] = json!({
            "type": "object",
            "properties": {
                "filters": { "type": "object" }
            }
        });
        let manifest = manifest_with(json!([tool]), non_terminal_connection());
        let error = validate_error(&manifest);
        assert!(error.field.contains("filters"), "{}", error.field);
        assert!(error.reason.contains("object"), "{}", error.reason);
    }

    #[test]
    fn schema_array_items_must_be_primitive() {
        let mut tool = valid_tool();
        tool["inputSchema"] = json!({
            "type": "object",
            "properties": {
                "ids": { "type": "array", "items": { "type": "object" } }
            }
        });
        let manifest = manifest_with(json!([tool]), non_terminal_connection());
        let error = validate_error(&manifest);
        assert!(error.field.contains("items"), "{}", error.field);
    }

    #[test]
    fn schema_format_is_limited() {
        let mut tool = valid_tool();
        tool["inputSchema"] = json!({
            "type": "object",
            "properties": {
                "when": { "type": "string", "format": "date" }
            }
        });
        let manifest = manifest_with(json!([tool]), non_terminal_connection());
        let error = validate_error(&manifest);
        assert!(error.field.contains("format"), "{}", error.field);
    }

    #[test]
    fn timeout_and_result_bounds_are_enforced() {
        let mut tool = valid_tool();
        tool["timeoutMs"] = json!(500);
        let manifest = manifest_with(json!([tool]), non_terminal_connection());
        assert_eq!(
            validate_error(&manifest).field,
            "/contributes/tools/containers.list/timeoutMs"
        );

        let mut tool = valid_tool();
        tool["maxResultBytes"] = json!(1024);
        let manifest = manifest_with(json!([tool]), non_terminal_connection());
        assert_eq!(
            validate_error(&manifest).field,
            "/contributes/tools/containers.list/maxResultBytes"
        );
    }

    #[test]
    fn empty_tools_section_is_fine() {
        let manifest = manifest_with(json!([]), non_terminal_connection());
        validate_tools(&manifest).expect("没有工具就没有校验");
    }
}
