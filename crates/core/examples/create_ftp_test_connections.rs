//! 使用 navop 自身 storage 层创建本地 FTP/FTPS 测试连接（对接 ftp-ftps-test Docker 服务器）。
//!
//! 运行：cargo run -p one-core --example create_ftp_test_connections
//!
//! 幂等：同名连接已存在时跳过。FTP 主机 127.0.0.1、FTPS 主机 localhost（证书含 DNS SAN）。

use anyhow::{Context, Result};
use one_core::crypto;
use one_core::storage::connection::SqliteConnection;
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, StoredConnection, SshParams, get_db_path};

const FTP_NAME: &str = "FTP测试-本地pure-ftpd";
const FTPS_NAME: &str = "FTPS测试-本地pure-ftpd";

fn ftp_ssh_params(ftp_host: &str, use_tls: bool, display_name: &str) -> Result<SshParams> {
    // SshParams 字段较多且均有 serde 默认值，用 JSON 反序列化构造最不易漏字段。
    let params: SshParams = serde_json::from_value(serde_json::json!({
        "host": "127.0.0.1",
        "port": 22,
        "username": "testuser",
        "auth_method": { "Password": { "password": "testpass" } },
        "connect_timeout": 10,
        "remote_file": {
            "protocol": "Ftp",
            "ftp": {
                "host": ftp_host,
                "port": 2121,
                "username": "testuser",
                "password": "testpass",
                "passive_mode": true,
                "use_tls": use_tls,
                "connect_timeout": 10
            }
        }
    }))
    .with_context(|| format!("反序列化 SshParams 失败: {display_name}"))?;
    Ok(params)
}

fn connection_names(conn: &rusqlite::Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM connections")?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(names)
}

fn main() -> Result<()> {
    if !crypto::try_restore_master_key() {
        anyhow::bail!("未能从本地密钥存储恢复 master key，无法加密落库；请先启动一次 navop");
    }

    let db_path = get_db_path().context("获取数据库路径失败")?;
    println!("数据库: {}", db_path.display());

    let existing = {
        let raw = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .context("只读打开数据库失败")?;
        connection_names(&raw)?
    };

    let repo = ConnectionRepository::new(SqliteConnection::open_with_pool_size(&db_path, 1)?);

    for (name, ftp_host, use_tls) in [
        (FTP_NAME, "127.0.0.1", false),
        (FTPS_NAME, "localhost", true),
    ] {
        if existing.iter().any(|n| n == name) {
            println!("跳过（已存在）: {name}");
            continue;
        }
        let stored = StoredConnection::new_ssh(
            name.to_string(),
            ftp_ssh_params(ftp_host, use_tls, name)?,
            None,
        );
        let mut stored = stored;
        let id = Repository::insert(&repo, &mut stored)?;
        println!("已创建: {name} (id={id}, ftp_host={ftp_host}, tls={use_tls})");
    }

    Ok(())
}
