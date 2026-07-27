use super::input::{
    optional_bool, optional_i64, optional_str, optional_u8, optional_u16, optional_u32,
    optional_value_str, required_object, required_value_str,
};
use one_core::storage::{
    PortForwardingKind, PortForwardingParams, RemoteDesktopParams, RemoteDesktopProtocol,
    SerialFlowControl, SerialParams, SerialParity, StoredConnection,
};
use serde_json::Value;
use tool_runtime::ToolError;

pub(super) fn build_serial(input: &Value) -> Result<StoredConnection, ToolError> {
    let values = required_object(input, "values")?;
    let params = SerialParams {
        port_name: required_value_str(values, "port_name")?.to_string(),
        baud_rate: optional_u32(values, "baud_rate").unwrap_or(115200),
        data_bits: optional_u8(values, "data_bits").unwrap_or(8),
        stop_bits: optional_u8(values, "stop_bits").unwrap_or(1),
        parity: parse_serial_parity(optional_value_str(values, "parity").unwrap_or("None"))?,
        flow_control: parse_serial_flow_control(
            optional_value_str(values, "flow_control").unwrap_or("None"),
        )?,
    };
    Ok(with_common_fields(
        StoredConnection::new_serial(
            required_value_str(values, "name")?.to_string(),
            params,
            optional_i64(input, "workspace_id"),
        ),
        input,
    ))
}

pub(super) fn build_port_forwarding(input: &Value) -> Result<StoredConnection, ToolError> {
    let values = required_object(input, "values")?;
    let params = PortForwardingParams {
        ssh_connection_id: required_i64(values, "ssh_connection_id")?,
        kind: parse_port_forwarding_kind(optional_value_str(values, "kind").unwrap_or("Local"))?,
        bind_host: optional_value_str(values, "bind_host")
            .unwrap_or("127.0.0.1")
            .to_string(),
        bind_port: optional_u16(values, "bind_port").unwrap_or(0),
        target_host: optional_value_str(values, "target_host")
            .unwrap_or_default()
            .to_string(),
        target_port: optional_u16(values, "target_port").unwrap_or(0),
    };
    Ok(with_common_fields(
        StoredConnection::new_port_forwarding(
            required_value_str(values, "name")?.to_string(),
            params,
            optional_i64(input, "workspace_id"),
        ),
        input,
    ))
}

pub(super) fn build_remote_desktop(
    input: &Value,
    kind: &str,
) -> Result<StoredConnection, ToolError> {
    let values = required_object(input, "values")?;
    let protocol = parse_remote_desktop_protocol(kind)?;
    let params = RemoteDesktopParams {
        protocol,
        host: required_value_str(values, "host")?.to_string(),
        port: optional_u16(values, "port").unwrap_or_else(|| protocol.default_port()),
        username: optional_value_str(values, "username").map(str::to_string),
        password: optional_value_str(values, "password").map(str::to_string),
        domain: optional_value_str(values, "domain").map(str::to_string),
        read_only: optional_bool(values, "read_only").unwrap_or(false),
        audio_playback: protocol == RemoteDesktopProtocol::Rdp
            && optional_bool(values, "audio_playback").unwrap_or(false),
        proxy: None,
    };
    Ok(with_common_fields(
        StoredConnection::new_remote_desktop(
            required_value_str(values, "name")?.to_string(),
            params,
            optional_i64(input, "workspace_id"),
        ),
        input,
    ))
}

fn with_common_fields(mut connection: StoredConnection, input: &Value) -> StoredConnection {
    connection.remark = optional_str(input, "remark").map(str::to_string);
    if let Some(sync_enabled) = optional_bool(input, "sync_enabled") {
        connection.sync_enabled = sync_enabled;
    }
    connection.team_id = optional_str(input, "team_id").map(str::to_string);
    connection
}

fn required_i64(input: &Value, field: &'static str) -> Result<i64, ToolError> {
    input
        .get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| ToolError::Failed {
            message: format!("missing integer field: {field}"),
        })
}

fn parse_serial_parity(value: &str) -> Result<SerialParity, ToolError> {
    match value {
        "None" => Ok(SerialParity::None),
        "Odd" => Ok(SerialParity::Odd),
        "Even" => Ok(SerialParity::Even),
        _ => unknown_value("serial parity", value),
    }
}

fn parse_serial_flow_control(value: &str) -> Result<SerialFlowControl, ToolError> {
    match value {
        "None" => Ok(SerialFlowControl::None),
        "Software" => Ok(SerialFlowControl::Software),
        "Hardware" => Ok(SerialFlowControl::Hardware),
        _ => unknown_value("serial flow control", value),
    }
}

fn parse_port_forwarding_kind(value: &str) -> Result<PortForwardingKind, ToolError> {
    match value {
        "Local" | "local" => Ok(PortForwardingKind::Local),
        "Dynamic" | "dynamic" => Ok(PortForwardingKind::Dynamic),
        _ => unknown_value("port forwarding kind", value),
    }
}

fn parse_remote_desktop_protocol(value: &str) -> Result<RemoteDesktopProtocol, ToolError> {
    match value {
        "rdp" => Ok(RemoteDesktopProtocol::Rdp),
        "vnc" => Ok(RemoteDesktopProtocol::Vnc),
        _ => unknown_value("remote desktop protocol", value),
    }
}

fn unknown_value<T>(label: &str, value: &str) -> Result<T, ToolError> {
    Err(ToolError::Failed {
        message: format!("unknown {label}: {value}"),
    })
}
