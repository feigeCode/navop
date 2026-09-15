//! 真实 FTP/FTPS 服务端冒烟测试。
//!
//! 依赖本机 Docker 启动的 pure-ftpd 测试服务器（`navop-workspace/ftp-ftps-test/`，
//! 端口 2121，用户 testuser/testpass）。默认 `#[ignore]`，不影响 CI；
//! 显式运行：`cargo test -p ftp -- --ignored`。
//!
//! FTPS 用例要求服务端证书能被系统信任链校验：
//! `security add-trusted-cert -r trustRoot -k ~/Library/Keychains/login.keychain-db ca.pem`
//! （`ca.pem` 为 ftp-ftps-test/certs/ 下自建 CA）。

use ftp::{FtpClient, FtpConnectConfig};
use sftp::RemoteFileClient;

fn test_config(use_tls: bool) -> FtpConnectConfig {
    FtpConnectConfig {
        host: std::env::var("NAVO_FTP_TEST_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
        port: std::env::var("NAVO_FTP_TEST_PORT")
            .ok()
            .and_then(|port| port.parse().ok())
            .unwrap_or(2121),
        username: std::env::var("NAVO_FTP_TEST_USER").unwrap_or_else(|_| "testuser".to_string()),
        password: std::env::var("NAVO_FTP_TEST_PASS").unwrap_or_else(|_| "testpass".to_string()),
        passive_mode: true,
        use_tls,
        connect_timeout: Some(15),
    }
}

async fn run_smoke(use_tls: bool) -> anyhow::Result<()> {
    let mut client = FtpClient::connect(test_config(use_tls)).await?;

    let entries = client.list_dir("/").await?;
    println!("[tls={use_tls}] root listing: {} entries", entries.len());

    let payload = format!("navop smoke tls={use_tls} pid={}", std::process::id()).into_bytes();
    client.write_file("smoke.txt", &payload).await?;
    let roundtrip = client.read_file("smoke.txt", 4096).await?;
    assert_eq!(roundtrip, payload, "roundtrip content mismatch");
    client.delete("smoke.txt", false).await?;
    println!("[tls={use_tls}] write/read/delete OK");
    Ok(())
}

#[tokio::test]
#[ignore = "需要本机 Docker FTP 测试服务器（navop-workspace/ftp-ftps-test/）"]
async fn real_ftp_smoke() {
    run_smoke(false).await.expect("plain FTP smoke failed");
}

#[tokio::test]
#[ignore = "需要本机 Docker FTP 测试服务器，且自建 CA 已装入系统信任链"]
async fn real_ftps_smoke() {
    run_smoke(true).await.expect("explicit FTPS smoke failed");
}
