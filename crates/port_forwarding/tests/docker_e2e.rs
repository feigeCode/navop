use std::env;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use one_core::storage::{
    PortForwardingKind, PortForwardingParams, SshAuthMethod, SshParams, StoredConnection,
};
use port_forwarding::{
    PortForwardingRuntime, build_dynamic_forwarding_request, build_local_forwarding_request,
};
use ssh::LocalPortForwardActivity;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

const ENABLE_ENV: &str = "ONETCLI_DOCKER_E2E";
const SSH_CONNECTION_ID: i64 = 91001;
const LOCAL_FORWARDING_ID: i64 = 91002;
const DYNAMIC_FORWARDING_ID: i64 = 91003;
const IO_TIMEOUT: Duration = Duration::from_secs(8);
const SSH_CONNECT_TIMEOUT_SECS: u64 = 10;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Docker SSH and HTTP services"]
async fn docker_local_and_dynamic_forwarding_roundtrip() -> Result<()> {
    ensure_docker_e2e_enabled()?;

    let ssh_port = env_u16("ONETCLI_DOCKER_SSH_PORT", 2222)?;
    let ssh_user = env_string("ONETCLI_DOCKER_SSH_USER", "onetcli");
    let ssh_password = env_string("ONETCLI_DOCKER_SSH_PASSWORD", "onetcli-pass");
    let target_host = env_string("ONETCLI_DOCKER_TARGET_HOST", "onetcli-pf-target");
    let target_port = env_u16("ONETCLI_DOCKER_TARGET_PORT", 80)?;

    let ssh_connection = ssh_connection(ssh_port, ssh_user, ssh_password);
    let mut runtime = PortForwardingRuntime::new();

    let (activity_tx, mut activity_rx) = tokio::sync::mpsc::unbounded_channel();
    let local_addr = start_local_forwarding(
        &mut runtime,
        &ssh_connection,
        &target_host,
        target_port,
        activity_tx,
    )
    .await?;
    tokio::time::sleep(Duration::from_secs(SSH_CONNECT_TIMEOUT_SECS + 1)).await;
    assert_http_response(local_addr, "Welcome to nginx!").await?;
    assert!(matches!(
        activity_rx.recv().await,
        Some(LocalPortForwardActivity::Connected { .. })
    ));

    let dynamic_addr = start_dynamic_forwarding(&mut runtime, &ssh_connection).await?;
    assert_socks_http_response(dynamic_addr, &target_host, target_port, "Welcome to nginx!")
        .await?;

    assert!(runtime.stop(LOCAL_FORWARDING_ID).await?);
    assert!(runtime.stop(DYNAMIC_FORWARDING_ID).await?);
    assert!(!runtime.is_running(LOCAL_FORWARDING_ID));
    assert!(!runtime.is_running(DYNAMIC_FORWARDING_ID));
    assert_listener_closed(local_addr).await?;
    assert_listener_closed(dynamic_addr).await?;

    Ok(())
}

async fn start_local_forwarding(
    runtime: &mut PortForwardingRuntime,
    ssh_connection: &StoredConnection,
    target_host: &str,
    target_port: u16,
    activity_tx: tokio::sync::mpsc::UnboundedSender<LocalPortForwardActivity>,
) -> Result<SocketAddr> {
    let forwarding = port_forwarding_connection(
        LOCAL_FORWARDING_ID,
        PortForwardingKind::Local,
        target_host.to_string(),
        target_port,
    );
    let mut request = build_local_forwarding_request(&forwarding, ssh_connection)?;
    request.activity_tx = Some(activity_tx);
    runtime.start_local(LOCAL_FORWARDING_ID, request).await
}

async fn start_dynamic_forwarding(
    runtime: &mut PortForwardingRuntime,
    ssh_connection: &StoredConnection,
) -> Result<SocketAddr> {
    let forwarding = port_forwarding_connection(
        DYNAMIC_FORWARDING_ID,
        PortForwardingKind::Dynamic,
        String::new(),
        0,
    );
    let request = build_dynamic_forwarding_request(&forwarding, ssh_connection)?;
    runtime.start_dynamic(DYNAMIC_FORWARDING_ID, request).await
}

fn ssh_connection(port: u16, username: String, password: String) -> StoredConnection {
    let mut connection = StoredConnection::new_ssh(
        "docker ssh".to_string(),
        SshParams {
            host: "127.0.0.1".to_string(),
            port,
            username,
            auth_method: SshAuthMethod::Password { password },
            connect_timeout: Some(SSH_CONNECT_TIMEOUT_SECS),
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            jump_server: None,
            proxy: None,
            os_id: None,
        },
        None,
    );
    connection.id = Some(SSH_CONNECTION_ID);
    connection
}

fn port_forwarding_connection(
    id: i64,
    kind: PortForwardingKind,
    target_host: String,
    target_port: u16,
) -> StoredConnection {
    let mut connection = StoredConnection::new_port_forwarding(
        format!("docker {id}"),
        PortForwardingParams {
            ssh_connection_id: SSH_CONNECTION_ID,
            kind,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 0,
            target_host,
            target_port,
        },
        None,
    );
    connection.id = Some(id);
    connection
}

async fn assert_http_response(addr: SocketAddr, expected_body: &str) -> Result<()> {
    let response = http_get(addr, "127.0.0.1").await?;
    assert!(
        response.contains(expected_body),
        "response should contain {expected_body:?}, got: {response}"
    );
    Ok(())
}

async fn assert_socks_http_response(
    socks_addr: SocketAddr,
    target_host: &str,
    target_port: u16,
    expected_body: &str,
) -> Result<()> {
    let mut stream = tcp_connect(socks_addr).await?;
    stream.write_all(&[0x05, 0x01, 0x00]).await?;

    let mut method_selection = [0u8; 2];
    timeout(IO_TIMEOUT, stream.read_exact(&mut method_selection))
        .await
        .context("timed out waiting for SOCKS method selection")??;
    assert_eq!([0x05, 0x00], method_selection);

    send_socks_connect(&mut stream, target_host, target_port).await?;
    let mut reply = [0u8; 10];
    timeout(IO_TIMEOUT, stream.read_exact(&mut reply))
        .await
        .context("timed out waiting for SOCKS connect reply")??;
    assert_eq!(0x05, reply[0]);
    assert_eq!(0x00, reply[1], "SOCKS connect should succeed: {reply:?}");

    write_http_request(&mut stream, target_host).await?;
    let response = read_response(&mut stream).await?;
    assert!(
        response.contains(expected_body),
        "SOCKS response should contain {expected_body:?}, got: {response}"
    );
    Ok(())
}

async fn send_socks_connect(
    stream: &mut TcpStream,
    target_host: &str,
    target_port: u16,
) -> Result<()> {
    let host_len = u8::try_from(target_host.len()).context("target host is too long")?;
    let mut request = Vec::with_capacity(7 + target_host.len());
    request.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, host_len]);
    request.extend_from_slice(target_host.as_bytes());
    request.extend_from_slice(&target_port.to_be_bytes());
    stream.write_all(&request).await?;
    Ok(())
}

async fn http_get(addr: SocketAddr, host_header: &str) -> Result<String> {
    let mut stream = tcp_connect(addr).await?;
    write_http_request(&mut stream, host_header).await?;
    read_response(&mut stream).await
}

async fn tcp_connect(addr: SocketAddr) -> Result<TcpStream> {
    timeout(IO_TIMEOUT, TcpStream::connect(addr))
        .await
        .context("timed out connecting to local forwarded socket")?
        .context("failed to connect to local forwarded socket")
}

async fn assert_listener_closed(addr: SocketAddr) -> Result<()> {
    let result = timeout(IO_TIMEOUT, TcpStream::connect(addr))
        .await
        .context("timed out checking closed forwarded socket")?;
    if result.is_ok() {
        bail!("forwarded socket should be closed: {addr}");
    }
    Ok(())
}

async fn write_http_request(stream: &mut TcpStream, host_header: &str) -> Result<()> {
    let request = format!("GET / HTTP/1.1\r\nHost: {host_header}\r\nConnection: close\r\n\r\n");
    timeout(IO_TIMEOUT, stream.write_all(request.as_bytes()))
        .await
        .context("timed out writing HTTP request")??;
    Ok(())
}

async fn read_response(stream: &mut TcpStream) -> Result<String> {
    let mut bytes = Vec::new();
    timeout(IO_TIMEOUT, stream.read_to_end(&mut bytes))
        .await
        .context("timed out reading HTTP response")??;
    String::from_utf8(bytes).context("HTTP response should be UTF-8")
}

fn ensure_docker_e2e_enabled() -> Result<()> {
    match env::var(ENABLE_ENV).as_deref() {
        Ok("1") => Ok(()),
        _ => bail!("set {ENABLE_ENV}=1 to run Docker E2E tests"),
    }
}

fn env_string(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_u16(key: &str, default: u16) -> Result<u16> {
    match env::var(key) {
        Ok(value) => value
            .parse::<u16>()
            .with_context(|| format!("{key} must be a u16")),
        Err(_) => Ok(default),
    }
}
