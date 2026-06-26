use super::*;
use serde_json::{Value, json};
use tool_runtime::{
    ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolHandler, ToolMode, ToolRegistry,
    ToolResult,
};

#[test]
fn list_outputs_cli_tools() {
    let output = run_tool_command(
        onetcli_cli::ToolCommand::List {
            format: onetcli_cli::OutputFormat::Json,
        },
        test_registry(),
    )
    .unwrap();
    let tools: Value = serde_json::from_str(&output).unwrap();

    assert_eq!("onetcli.app_info", tools[0]["id"]);
    assert_eq!(json!(true), tools[0]["read_only"]);
    assert_eq!(json!(true), tools[0]["annotations"]["read_only"]);
    assert_eq!("deterministic", tools[0]["mode"]);
    assert!(tools[0]["input_schema"].is_object());
}

#[test]
fn schema_outputs_full_tool_descriptor() {
    let output = run_tool_command(
        onetcli_cli::ToolCommand::Schema {
            tool_id: "onetcli.app_info".to_string(),
            format: onetcli_cli::OutputFormat::Json,
        },
        test_registry(),
    )
    .unwrap();
    let schema: Value = serde_json::from_str(&output).unwrap();

    assert_eq!("onetcli.app_info", schema["id"]);
    assert_eq!("object", schema["input_schema"]["type"]);
    assert_eq!(
        json!(["name", "version"]),
        schema["output_schema"]["required"]
    );
}

#[test]
fn call_runs_tool_with_default_empty_input() {
    let output = run_tool_command(
        onetcli_cli::ToolCommand::Call {
            tool_id: "onetcli.app_info".to_string(),
            input: None,
            positional_input: None,
            allow_write: false,
            format: onetcli_cli::OutputFormat::Json,
        },
        test_registry(),
    )
    .unwrap();
    let result: Value = serde_json::from_str(&output).unwrap();

    assert_eq!("onetcli", result["name"]);
    assert_eq!(env!("CARGO_PKG_VERSION"), result["version"]);
}

#[test]
fn call_rejects_invalid_json_input() {
    let error = run_tool_command(
        onetcli_cli::ToolCommand::Call {
            tool_id: "onetcli.app_info".to_string(),
            input: Some("{bad".to_string()),
            positional_input: None,
            allow_write: false,
            format: onetcli_cli::OutputFormat::Json,
        },
        test_registry(),
    )
    .unwrap_err()
    .to_string();

    let error: Value = serde_json::from_str(&error).unwrap();
    assert_eq!(json!(false), error["ok"]);
    assert_eq!("invalid_json", error["error"]["code"]);
}

#[test]
fn mutating_tool_requires_allow_write() {
    let output = run_tool_command(
        onetcli_cli::ToolCommand::Call {
            tool_id: "example.write".to_string(),
            input: None,
            positional_input: None,
            allow_write: false,
            format: onetcli_cli::OutputFormat::Json,
        },
        mutating_registry(),
    )
    .unwrap_err()
    .to_string();
    let error: Value = serde_json::from_str(&output).unwrap();

    assert_eq!("write_not_allowed", error["error"]["code"]);
}

#[test]
fn mutating_tool_runs_with_allow_write() {
    let output = run_tool_command(
        onetcli_cli::ToolCommand::Call {
            tool_id: "example.write".to_string(),
            input: None,
            positional_input: None,
            allow_write: true,
            format: onetcli_cli::OutputFormat::Json,
        },
        mutating_registry(),
    )
    .unwrap();
    let result: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json!(true), result["wrote"]);
}

#[test]
fn connection_list_alias_calls_connection_list_tool() {
    let output = run_connection_command(
        onetcli_cli::ConnectionCommand::List {
            format: onetcli_cli::OutputFormat::Json,
        },
        connection_alias_registry(),
    )
    .unwrap();
    let result: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json!(true), result["listed"]);
}

#[test]
fn connection_show_alias_passes_connection_argument() {
    let output = run_connection_command(
        onetcli_cli::ConnectionCommand::Show {
            connection: "prod".to_string(),
            format: onetcli_cli::OutputFormat::Json,
        },
        connection_alias_registry(),
    )
    .unwrap();
    let result: Value = serde_json::from_str(&output).unwrap();

    assert_eq!("prod", result["connection"]);
}

fn test_registry() -> ToolRegistry {
    crate::builtin_tool_registry()
}

fn mutating_registry() -> ToolRegistry {
    use std::sync::Arc;

    #[derive(Clone)]
    struct WriteTool;

    impl ToolHandler for WriteTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor {
                id: "example.write".to_string(),
                title: "Write".to_string(),
                description: "Write test data.".to_string(),
                input_schema: json!({ "type": "object" }),
                output_schema: json!({ "type": "object" }),
                permissions: Vec::new(),
                mode: ToolMode::Deterministic,
                adapters: vec![ToolAdapter::FunctionCalling],
                annotations: ToolAnnotations::mutating("Write"),
            }
        }

        fn call(
            &self,
            _input: serde_json::Value,
            _context: ToolContext,
        ) -> tool_runtime::ToolFuture {
            Box::pin(async move { Ok(ToolResult::structured(json!({ "wrote": true }))) })
        }
    }

    ToolRegistry::new(vec![Arc::new(WriteTool)])
}

fn connection_alias_registry() -> ToolRegistry {
    use std::sync::Arc;

    #[derive(Clone)]
    struct AliasTool {
        id: &'static str,
    }

    impl ToolHandler for AliasTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor {
                id: self.id.to_string(),
                title: self.id.to_string(),
                description: "Alias test tool.".to_string(),
                input_schema: json!({ "type": "object" }),
                output_schema: json!({ "type": "object" }),
                permissions: Vec::new(),
                mode: ToolMode::Deterministic,
                adapters: vec![ToolAdapter::FunctionCalling],
                annotations: ToolAnnotations::read_only(self.id),
            }
        }

        fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
            let id = self.id;
            Box::pin(async move {
                if id.ends_with(".show") {
                    Ok(ToolResult::structured(json!({
                        "connection": input["connection"]
                    })))
                } else {
                    Ok(ToolResult::structured(json!({ "listed": true })))
                }
            })
        }
    }

    ToolRegistry::new(vec![
        Arc::new(AliasTool {
            id: "connections.list",
        }),
        Arc::new(AliasTool {
            id: "connections.show",
        }),
    ])
}
