//! Directory downloads must overlap small files instead of waiting for each
//! file to finish. The fake server counts open file handles, so a serial
//! implementation cannot pass merely because one file's read pipeline made
//! several requests concurrently.

use super::*;
use russh_sftp::protocol::{Data, File, Handle, Name, Status, Version};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

type ActivePaths = Arc<StdMutex<HashSet<String>>>;
type MaxPaths = Arc<AtomicUsize>;

struct FakeDirectoryServer {
    files: Vec<(String, Vec<u8>)>,
    contents: HashMap<String, Vec<u8>>,
    active_paths: ActivePaths,
    max_paths: MaxPaths,
    listed: bool,
}

impl FakeDirectoryServer {
    fn new(files: Vec<(String, Vec<u8>)>) -> (Self, MaxPaths) {
        let contents = files
            .iter()
            .map(|(name, data)| (format!("/root/{name}"), data.clone()))
            .collect();
        let active_paths = Arc::new(StdMutex::new(HashSet::new()));
        let max_paths = Arc::new(AtomicUsize::new(0));
        (
            Self {
                files,
                contents,
                active_paths: Arc::clone(&active_paths),
                max_paths: Arc::clone(&max_paths),
                listed: false,
            },
            max_paths,
        )
    }
}

impl russh_sftp::server::Handler for FakeDirectoryServer {
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
                .files
                .iter()
                .map(|(name, contents)| {
                    let mut attrs = FileAttributes::empty();
                    attrs.size = Some(contents.len() as u64);
                    attrs.permissions = Some(0o100644);
                    attrs.mtime = Some(0);
                    File::new(name.clone(), attrs)
                })
                .collect();
            Ok(Name { id, files })
        };
        async move { result }
    }

    fn open(
        &mut self,
        id: u32,
        filename: String,
        _pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> impl Future<Output = std::result::Result<Handle, Self::Error>> + Send {
        let mut active_paths = self.active_paths.lock().expect("active path lock");
        active_paths.insert(filename.clone());
        self.max_paths
            .fetch_max(active_paths.len(), Ordering::Relaxed);
        drop(active_paths);
        async move {
            Ok(Handle {
                id,
                handle: filename,
            })
        }
    }

    fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> impl Future<Output = std::result::Result<Data, Self::Error>> + Send {
        let data = self.contents.get(&handle).cloned().unwrap_or_default();
        async move {
            let start = usize::try_from(offset)
                .unwrap_or(usize::MAX)
                .min(data.len());
            let end = start.saturating_add(len as usize).min(data.len());
            Ok(Data {
                id,
                data: data[start..end].to_vec(),
            })
        }
    }

    fn close(
        &mut self,
        id: u32,
        handle: String,
    ) -> impl Future<Output = std::result::Result<Status, Self::Error>> + Send {
        self.active_paths
            .lock()
            .expect("active path lock")
            .remove(&handle);
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

#[tokio::test]
async fn directory_download_overlaps_small_files_and_keeps_progress_monotonic() {
    let files = vec![
        ("alpha.bin".to_string(), vec![1_u8; 32 * 1024]),
        ("beta.bin".to_string(), vec![2_u8; 32 * 1024]),
        ("gamma.bin".to_string(), vec![3_u8; 32 * 1024]),
    ];
    let expected_total: u64 = files.iter().map(|(_, data)| data.len() as u64).sum();
    let (metadata_server, _) = FakeDirectoryServer::new(files.clone());
    let (metadata_client, metadata_server_stream) = tokio::io::duplex(1024 * 1024);
    russh_sftp::server::run(metadata_server_stream, metadata_server).await;

    let (data_server, max_paths) = FakeDirectoryServer::new(files.clone());
    let (data_client, data_server_stream) = tokio::io::duplex(1024 * 1024);
    russh_sftp::server::run(data_server_stream, data_server).await;

    let sftp =
        SftpSession::new_with_config(metadata_client, sftp_session_config(SftpTransport::Owned))
            .await
            .expect("fake SFTP session must initialize");
    let raw = RawSftpSession::new(data_client);
    raw.init()
        .await
        .expect("fake raw SFTP session must initialize");
    let mut client = RusshSftpClient::for_test(sftp, Some(Arc::new(raw)));

    let local = tempfile::tempdir().expect("temporary directory");
    let progress = Arc::new(StdMutex::new(Vec::<TransferProgress>::new()));
    let captured = Arc::clone(&progress);
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        client.download_dir_with_progress(
            "/root",
            local.path().to_str().expect("UTF-8 temporary path"),
            Arc::new(AtomicBool::new(false)),
            Box::new(move |value| {
                captured.lock().expect("progress lock").push(value);
            }),
        ),
    )
    .await
    .expect("directory download should not wait for one file at a time");

    result.expect("directory download should complete");
    assert!(
        max_paths.load(Ordering::Relaxed) >= 2,
        "directory download must start at least two remote files concurrently"
    );

    for (name, expected) in files {
        let actual = std::fs::read(local.path().join(&name)).expect("downloaded file");
        assert_eq!(expected, actual, "downloaded bytes must match {name}");
    }

    let progress = progress.lock().expect("progress lock");
    let transferred: Vec<u64> = progress.iter().map(|value| value.transferred).collect();
    assert!(
        transferred.windows(2).all(|window| window[0] <= window[1]),
        "progress must never move backwards: {transferred:?}"
    );
    assert_eq!(
        Some(&expected_total),
        progress.last().map(|value| &value.transferred),
        "final progress must include every downloaded byte"
    );
}
