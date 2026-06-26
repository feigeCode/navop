use super::*;

#[test]
fn no_command_starts_gui() {
    let parsed = parse_from(["onetcli"]).unwrap();

    assert_eq!(None, parsed);
}

#[test]
fn parses_tool_list_command() {
    let parsed = parse_from(["onetcli", "tool", "list", "--format", "json"]).unwrap();

    assert_eq!(
        Some(OnetCliCommand::Tool(ToolCommand::List {
            format: OutputFormat::Json
        })),
        parsed
    );
}

#[test]
fn parses_tool_schema_command() {
    let parsed = parse_from([
        "onetcli",
        "tool",
        "schema",
        "onetcli.app_info",
        "--format",
        "json",
    ])
    .unwrap();

    assert_eq!(
        Some(OnetCliCommand::Tool(ToolCommand::Schema {
            tool_id: "onetcli.app_info".to_string(),
            format: OutputFormat::Json,
        })),
        parsed
    );
}

#[test]
fn parses_tool_call_command_with_input_option() {
    let parsed = parse_from([
        "onetcli",
        "tool",
        "call",
        "onetcli.app_info",
        "--input",
        r#"{"verbose":true}"#,
    ])
    .unwrap();

    assert_eq!(
        Some(OnetCliCommand::Tool(ToolCommand::Call {
            tool_id: "onetcli.app_info".to_string(),
            input: Some(r#"{"verbose":true}"#.to_string()),
            positional_input: None,
            allow_write: false,
            format: OutputFormat::Json,
        })),
        parsed
    );
}

#[test]
fn parses_tool_call_command_with_allow_write() {
    let parsed = parse_from([
        "onetcli",
        "tool",
        "call",
        "onetcli.connections.create",
        "--input",
        r#"{"kind":"database","values":{}}"#,
        "--allow-write",
    ])
    .unwrap();

    assert_eq!(
        Some(OnetCliCommand::Tool(ToolCommand::Call {
            tool_id: "onetcli.connections.create".to_string(),
            input: Some(r#"{"kind":"database","values":{}}"#.to_string()),
            positional_input: None,
            allow_write: true,
            format: OutputFormat::Json,
        })),
        parsed
    );
}

#[test]
fn parses_connection_list_command() {
    let parsed = parse_from(["onetcli", "connection", "list", "--format", "json"]).unwrap();

    assert_eq!(
        Some(OnetCliCommand::Connection(ConnectionCommand::List {
            format: OutputFormat::Json
        })),
        parsed
    );
}

#[test]
fn parses_connection_show_command() {
    let parsed = parse_from(["onetcli", "connection", "show", "prod", "--format", "json"]).unwrap();

    assert_eq!(
        Some(OnetCliCommand::Connection(ConnectionCommand::Show {
            connection: "prod".to_string(),
            format: OutputFormat::Json,
        })),
        parsed
    );
}

#[test]
fn parses_db_query_command() {
    let parsed = parse_from([
        "onetcli",
        "db",
        "query",
        "prod",
        "--sql",
        "select 1",
        "--readonly",
        "--format",
        "json",
    ])
    .unwrap();

    assert_eq!(
        Some(OnetCliCommand::Db(DbCommand::Query {
            connection: "prod".to_string(),
            sql: "select 1".to_string(),
            readonly: true,
            format: OutputFormat::Json,
        })),
        parsed
    );
}

#[test]
fn parses_ssh_exec_command() {
    let parsed = parse_from([
        "onetcli",
        "ssh",
        "exec",
        "prod-web",
        "--command",
        "uptime",
        "--timeout",
        "10s",
    ])
    .unwrap();

    assert_eq!(
        Some(OnetCliCommand::Ssh(SshCommand::Exec {
            connection: "prod-web".to_string(),
            command: "uptime".to_string(),
            timeout: Some("10s".to_string()),
            format: OutputFormat::Json,
        })),
        parsed
    );
}

#[test]
fn parses_sftp_read_command() {
    let parsed = parse_from([
        "onetcli",
        "sftp",
        "read",
        "prod-web",
        "/var/log/app.log",
        "--max-bytes",
        "65536",
    ])
    .unwrap();

    assert_eq!(
        Some(OnetCliCommand::Sftp(SftpCommand::Read {
            connection: "prod-web".to_string(),
            path: "/var/log/app.log".to_string(),
            max_bytes: Some(65536),
            format: OutputFormat::Json,
        })),
        parsed
    );
}

#[test]
fn parses_sftp_transfer_commands() {
    let stat = parse_from(["onetcli", "sftp", "stat", "prod-web", "/opt/app"]).unwrap();
    assert_eq!(
        Some(OnetCliCommand::Sftp(SftpCommand::Stat {
            connection: "prod-web".to_string(),
            path: "/opt/app".to_string(),
            format: OutputFormat::Json,
        })),
        stat
    );

    let upload = parse_from([
        "onetcli",
        "sftp",
        "upload",
        "prod-web",
        "./dist",
        "/opt/app",
        "--on-exists",
        "overwrite",
    ])
    .unwrap();
    assert_eq!(
        Some(OnetCliCommand::Sftp(SftpCommand::Upload {
            connection: "prod-web".to_string(),
            local_path: "./dist".to_string(),
            remote_path: "/opt/app".to_string(),
            on_exists: "overwrite".to_string(),
            format: OutputFormat::Json,
        })),
        upload
    );

    let download = parse_from([
        "onetcli",
        "sftp",
        "download",
        "prod-web",
        "/var/log/app.log",
        "./app.log",
        "--on-exists",
        "skip",
    ])
    .unwrap();
    assert_eq!(
        Some(OnetCliCommand::Sftp(SftpCommand::Download {
            connection: "prod-web".to_string(),
            remote_path: "/var/log/app.log".to_string(),
            local_path: "./app.log".to_string(),
            on_exists: "skip".to_string(),
            format: OutputFormat::Json,
        })),
        download
    );
}

#[test]
fn keeps_positional_tool_call_input_for_compatibility() {
    let parsed = parse_from([
        "onetcli",
        "tool",
        "call",
        "onetcli.app_info",
        r#"{"verbose":true}"#,
    ])
    .unwrap();

    assert_eq!(
        Some(OnetCliCommand::Tool(ToolCommand::Call {
            tool_id: "onetcli.app_info".to_string(),
            input: None,
            positional_input: Some(r#"{"verbose":true}"#.to_string()),
            allow_write: false,
            format: OutputFormat::Json,
        })),
        parsed
    );
}

#[test]
fn rejects_tool_call_with_two_inputs() {
    let error = parse_from([
        "onetcli",
        "tool",
        "call",
        "onetcli.app_info",
        r#"{"positional":true}"#,
        "--input",
        r#"{"option":true}"#,
    ])
    .unwrap_err();

    assert_eq!(clap::error::ErrorKind::ArgumentConflict, error.kind());
}

#[test]
fn rejects_tool_call_without_tool_id() {
    let error = parse_from(["onetcli", "tool", "call"]).unwrap_err();

    assert_eq!(
        clap::error::ErrorKind::MissingRequiredArgument,
        error.kind()
    );
}
