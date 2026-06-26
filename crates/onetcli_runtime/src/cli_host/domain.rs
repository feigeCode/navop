use serde_json::json;
use tool_runtime::ToolRegistry;

use super::{call_tool, run_function_tool};

pub fn run_connection_command(
    command: onetcli_cli::ConnectionCommand,
    registry: ToolRegistry,
) -> anyhow::Result<String> {
    match command {
        onetcli_cli::ConnectionCommand::List { format } => {
            run_function_tool("connections.list", json!({}), false, registry, format)
        }
        onetcli_cli::ConnectionCommand::Show { connection, format } => call_tool(
            "connections.show",
            Some(json!({ "connection": connection }).to_string()),
            false,
            registry,
            format,
        ),
    }
}

pub fn run_db_command(
    command: onetcli_cli::DbCommand,
    registry: ToolRegistry,
) -> anyhow::Result<String> {
    match command {
        onetcli_cli::DbCommand::Schema { connection, format } => run_function_tool(
            "db.schema",
            json!({ "connection": connection }),
            false,
            registry,
            format,
        ),
        onetcli_cli::DbCommand::Query {
            connection,
            sql,
            readonly,
            format,
        } => run_function_tool(
            "db.query",
            json!({ "connection": connection, "sql": sql, "readonly": readonly }),
            false,
            registry,
            format,
        ),
        onetcli_cli::DbCommand::Exec {
            connection,
            file,
            write,
            format,
        } => run_function_tool(
            "db.exec",
            json!({ "connection": connection, "file": file, "write": write }),
            write,
            registry,
            format,
        ),
    }
}

pub fn run_ssh_command(
    command: onetcli_cli::SshCommand,
    registry: ToolRegistry,
) -> anyhow::Result<String> {
    match command {
        onetcli_cli::SshCommand::Exec {
            connection,
            command,
            timeout,
            format,
        } => run_function_tool(
            "ssh.exec",
            json!({ "connection": connection, "command": command, "timeout": timeout }),
            false,
            registry,
            format,
        ),
        onetcli_cli::SshCommand::Shell {
            connection,
            workdir,
            init,
            transcript,
        } => run_function_tool(
            "ssh.shell",
            json!({
                "connection": connection,
                "workdir": workdir,
                "init": init,
                "transcript": transcript
            }),
            false,
            registry,
            onetcli_cli::OutputFormat::Json,
        ),
        onetcli_cli::SshCommand::Tunnel {
            connection,
            local,
            remote,
        } => run_function_tool(
            "ssh.tunnel",
            json!({ "connection": connection, "local": local, "remote": remote }),
            false,
            registry,
            onetcli_cli::OutputFormat::Json,
        ),
        onetcli_cli::SshCommand::Socks { connection, local } => run_function_tool(
            "ssh.socks",
            json!({ "connection": connection, "local": local }),
            false,
            registry,
            onetcli_cli::OutputFormat::Json,
        ),
    }
}

pub fn run_sftp_command(
    command: onetcli_cli::SftpCommand,
    registry: ToolRegistry,
) -> anyhow::Result<String> {
    match command {
        onetcli_cli::SftpCommand::List {
            connection,
            path,
            format,
        } => run_function_tool(
            "sftp.list",
            json!({ "connection": connection, "path": path }),
            false,
            registry,
            format,
        ),
        onetcli_cli::SftpCommand::Read {
            connection,
            path,
            max_bytes,
            format,
        } => run_function_tool(
            "sftp.read",
            json!({ "connection": connection, "path": path, "max_bytes": max_bytes }),
            false,
            registry,
            format,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use tool_runtime::{
        ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolHandler, ToolMode,
        ToolResult,
    };

    #[test]
    fn db_query_alias_calls_function_tool() {
        let output = run_db_command(
            onetcli_cli::DbCommand::Query {
                connection: "prod".to_string(),
                sql: "select 1".to_string(),
                readonly: true,
                format: onetcli_cli::OutputFormat::Json,
            },
            domain_alias_registry(),
        )
        .unwrap();
        let result: Value = serde_json::from_str(&output).unwrap();

        assert_eq!("db.query", result["tool"]);
        assert_eq!("prod", result["input"]["connection"]);
        assert_eq!("select 1", result["input"]["sql"]);
        assert_eq!(json!(true), result["input"]["readonly"]);
    }

    #[test]
    fn ssh_exec_alias_calls_function_tool() {
        let output = run_ssh_command(
            onetcli_cli::SshCommand::Exec {
                connection: "prod-web".to_string(),
                command: "uptime".to_string(),
                timeout: Some("10s".to_string()),
                format: onetcli_cli::OutputFormat::Json,
            },
            domain_alias_registry(),
        )
        .unwrap();
        let result: Value = serde_json::from_str(&output).unwrap();

        assert_eq!("ssh.exec", result["tool"]);
        assert_eq!("prod-web", result["input"]["connection"]);
        assert_eq!("uptime", result["input"]["command"]);
        assert_eq!("10s", result["input"]["timeout"]);
    }

    #[test]
    fn sftp_read_alias_calls_function_tool() {
        let output = run_sftp_command(
            onetcli_cli::SftpCommand::Read {
                connection: "prod-web".to_string(),
                path: "/var/log/app.log".to_string(),
                max_bytes: Some(65536),
                format: onetcli_cli::OutputFormat::Json,
            },
            domain_alias_registry(),
        )
        .unwrap();
        let result: Value = serde_json::from_str(&output).unwrap();

        assert_eq!("sftp.read", result["tool"]);
        assert_eq!("prod-web", result["input"]["connection"]);
        assert_eq!("/var/log/app.log", result["input"]["path"]);
        assert_eq!(json!(65536), result["input"]["max_bytes"]);
    }

    fn domain_alias_registry() -> ToolRegistry {
        use std::sync::Arc;

        #[derive(Clone)]
        struct DomainTool {
            id: &'static str,
        }

        impl ToolHandler for DomainTool {
            fn descriptor(&self) -> ToolDescriptor {
                ToolDescriptor {
                    id: self.id.to_string(),
                    title: self.id.to_string(),
                    description: "Domain alias test tool.".to_string(),
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
                    Ok(ToolResult::structured(json!({
                        "tool": id,
                        "input": input
                    })))
                })
            }
        }

        ToolRegistry::new(vec![
            Arc::new(DomainTool { id: "db.query" }),
            Arc::new(DomainTool { id: "ssh.exec" }),
            Arc::new(DomainTool { id: "sftp.read" }),
        ])
    }
}
