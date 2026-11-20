//! 目录列举必须对「恶意/异常条目名」保持可用。
//!
//! 历史上 `list_dir` 遇到非单层名字会直接返回错误，于是服务端目录里
//! 只要存在一个 `..` 或者 Linux 上完全合法的 `c:`，Windows 客户端
//! 就会整个目录读不出来、并被导航逻辑回退到工作目录。正确行为是
//! 跳过这些条目：坏条目不该毁掉整个目录浏览。

use super::*;
use russh_sftp::protocol::{File, Handle, Name, Status, Version};
use std::collections::HashMap;

struct FakeListingServer {
    names: Vec<String>,
    listed: bool,
}

impl FakeListingServer {
    fn new(names: &[&str]) -> Self {
        Self {
            names: names.iter().map(|name| (*name).to_string()).collect(),
            listed: false,
        }
    }
}

impl russh_sftp::server::Handler for FakeListingServer {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> impl Future<Output = std::result::Result<Version, Self::Error>> + Send {
        async { Ok(Version::new()) }
    }

    fn opendir(
        &mut self,
        id: u32,
        path: String,
    ) -> impl Future<Output = std::result::Result<Handle, Self::Error>> + Send {
        self.listed = false;
        async move { Ok(Handle { id, handle: path }) }
    }

    fn readdir(
        &mut self,
        id: u32,
        _handle: String,
    ) -> impl Future<Output = std::result::Result<Name, Self::Error>> + Send {
        let result = if self.listed {
            Err(StatusCode::Eof)
        } else {
            self.listed = true;
            let files = self
                .names
                .iter()
                .map(|name| File::new(name.clone(), FileAttributes::empty()))
                .collect();
            Ok(Name { id, files })
        };
        async move { result }
    }

    fn close(
        &mut self,
        id: u32,
        _handle: String,
    ) -> impl Future<Output = std::result::Result<Status, Self::Error>> + Send {
        async move {
            Ok(Status {
                id,
                status_code: StatusCode::Ok,
                error_message: "Ok".to_string(),
                language_tag: "en-US".to_string(),
            })
        }
    }
}

async fn list_fake_directory(names: &[&str]) -> Result<Vec<FileEntry>> {
    let (client_stream, server_stream) = tokio::io::duplex(1024 * 1024);
    russh_sftp::server::run(server_stream, FakeListingServer::new(names)).await;
    let sftp =
        SftpSession::new_with_config(client_stream, sftp_session_config(SftpTransport::Owned))
            .await
            .expect("fake SFTP session must initialize");
    let mut client = RusshSftpClient::for_test(sftp, None);
    client.list_dir("/root").await
}

#[tokio::test]
async fn list_dir_skips_unsafe_names_instead_of_failing_the_directory() {
    let entries = list_fake_directory(&[
        ".",
        "..",
        "notes.txt",
        "../outside.txt",
        "sub/file.txt",
        "c:",
        "报名表.xlsx",
    ])
    .await
    .expect("一个坏条目不能让整个目录读取失败");

    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["c:", "notes.txt", "报名表.xlsx"],
        "只保留单层名字：`.`/`..`、`../x`、`a/b` 必须被跳过"
    );
}

#[tokio::test]
async fn list_dir_reports_an_empty_directory_instead_of_failing() {
    let entries = list_fake_directory(&[".", ".."])
        .await
        .expect("只含 `.`/`..` 的空目录必须正常返回");
    assert!(entries.is_empty());
}
