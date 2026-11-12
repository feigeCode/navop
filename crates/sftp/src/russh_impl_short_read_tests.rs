//! issue #363 的端到端回归：服务端只回「短读」时，流水线下载必须自己续读补齐。
//!
//! 现场是经 JumpServer 的链路：客户端按 61440 字节一个分片请求，服务端只回
//! 32768 字节。「不超过请求长度」是 SFTP 协议允许的应答，短读完全合法；旧实现
//! 在 `validate_chunk_len` 处直接判失败，于是整条下载以
//! `SFTP short read at offset 368640: received 32768 bytes, expected 61440` 中断。
//!
//! 这里把 JumpServer 的行为搬进进程内：`russh_sftp::server` 起一个真的 SFTP
//! 服务端，只把 `read` 覆写成一次最多回 32 KiB，接在 `tokio::io::duplex` 的另一端，
//! 让 `pipelined_read_into_writer` 走完整协议往返，再断言落盘字节与源文件逐字节
//! 一致。整个测试不碰网络、不开端口、不依赖任何外部服务。

use super::*;
use russh_sftp::protocol::{Data, Handle, Status, Version};
use std::collections::HashMap;

/// 复刻 JumpServer：无论客户端要多少，一次最多回 32 KiB。
const SHORT_READ_CAP: usize = 32 * 1024;

/// 服务端观察到的 `(请求长度, 实际回出长度)`，用来证明这个场景**确实**发生了短读。
type ReadLog = Arc<StdMutex<Vec<(u32, usize)>>>;

/// 真实 App 里只有大于 `PIPELINE_THRESHOLD` 的文件才走流水线下载，
/// 所以这里必须造一个超过阈值的文件，场景才与 issue #363 等价。
const PAYLOAD_LEN: usize = PIPELINE_CHUNK_SIZE as usize * 9 + 1234;

struct FakeSftpServer {
    payload: Arc<Vec<u8>>,
    /// `None` 表示老实的服务端（一次读满）；`Some(cap)` 表示每次最多回 cap 字节。
    cap: Option<usize>,
    log: ReadLog,
}

impl FakeSftpServer {
    fn new(payload: Arc<Vec<u8>>, cap: Option<usize>, log: ReadLog) -> Self {
        Self { payload, cap, log }
    }
}

impl russh_sftp::server::Handler for FakeSftpServer {
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

    fn open(
        &mut self,
        id: u32,
        filename: String,
        _pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> impl Future<Output = std::result::Result<Handle, Self::Error>> + Send {
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
        _handle: String,
        offset: u64,
        len: u32,
    ) -> impl Future<Output = std::result::Result<Data, Self::Error>> + Send {
        let start = usize::try_from(offset).expect("offset must fit in usize");
        let allowed = match self.cap {
            Some(cap) => (len as usize).min(cap),
            None => len as usize,
        };
        let end = start.saturating_add(allowed).min(self.payload.len());
        let data = if start >= self.payload.len() {
            Vec::new()
        } else {
            self.payload[start..end].to_vec()
        };
        self.log
            .lock()
            .expect("read log mutex")
            .push((len, data.len()));
        async move { Ok(Data { id, data }) }
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

/// 让客户端的流水线读端真跑在协议另一头，返回（统计的字节数，落盘内容，服务端读日志）。
async fn pipelined_download_from(
    server: FakeSftpServer,
    log: ReadLog,
    total_size: u64,
) -> (u64, Vec<u8>, Vec<(u32, usize)>) {
    let (client_stream, server_stream) = tokio::io::duplex(1024 * 1024);
    // `run` 只负责把服务端循环丢到后台任务里，`await` 后会立刻返回。
    russh_sftp::server::run(server_stream, server).await;

    let raw = RawSftpSession::new(client_stream);
    raw.init()
        .await
        .expect("fake sftp server must complete the init handshake");

    let directory = tempfile::tempdir().expect("temp dir");
    let local_path = directory.path().join("big.bin");
    let local_file = File::create(&local_path)
        .await
        .expect("local file must be created");
    let mut writer = BufWriter::with_capacity(BUFFER_SIZE, local_file);

    let transferred = RusshSftpClient::pipelined_read_into_writer(
        Arc::new(raw),
        "/big.bin",
        total_size,
        MAX_INFLIGHT_REQUESTS,
        &AtomicBool::new(false),
        &mut writer,
        |_| {},
    )
    .await
    .expect("流水线下载不该失败");

    writer.flush().await.expect("local file must flush");
    drop(writer);

    let written = fs::read(&local_path)
        .await
        .expect("downloaded file must be readable");
    let logged = log.lock().expect("read log mutex").clone();
    (transferred, written, logged)
}

fn payload_bytes(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

/// #363 主用例：服务端每次只回 32 KiB（JumpServer 的形态），下载仍须逐字节正确。
#[tokio::test]
async fn a_short_reading_server_still_yields_a_byte_exact_download() {
    assert!(
        PAYLOAD_LEN as u64 > PIPELINE_THRESHOLD,
        "用例必须先落在流水线区间，否则测不到 #363 的路径"
    );

    let payload = Arc::new(payload_bytes(PAYLOAD_LEN));
    let log: ReadLog = Arc::new(StdMutex::new(Vec::new()));
    let (transferred, written, logged) = pipelined_download_from(
        FakeSftpServer::new(Arc::clone(&payload), Some(SHORT_READ_CAP), Arc::clone(&log)),
        Arc::clone(&log),
        payload.len() as u64,
    )
    .await;

    assert_eq!(
        payload.len() as u64,
        transferred,
        "统计字节数必须等于文件大小"
    );
    assert_eq!(*payload, written, "下载内容必须与远端逐字节一致");

    // 先证明这个用例真的复现了短读：客户端按 61440 要，服务端只给了 32768。
    // 否则（万一假服务端被改成读满）用例会变成空转，测不到 #363。
    assert!(
        logged
            .iter()
            .any(|(requested, returned)| *requested == PIPELINE_CHUNK_SIZE
                && *returned == SHORT_READ_CAP),
        "用例未能复现短读，等于没测到 #363：{logged:?}"
    );
    // 协议约束：服务端绝不能回超过请求长度的数据。
    assert!(
        logged
            .iter()
            .all(|(requested, returned)| *returned <= *requested as usize),
        "服务端回出了超过请求长度的数据：{logged:?}"
    );
    // 续读要收窄缺口：9 个满片各两次（32768 + 28672），末尾残片一次。
    let expected_requests = 9 * 2 + 1;
    assert_eq!(
        expected_requests,
        logged.len(),
        "每个分片应当只补读一次就凑满，实际请求序列：{logged:?}"
    );
}

/// 对照组：老实的服务端（一次读满）行为不变——不该多出任何一次读请求。
/// 这条同时兜住「假服务端本身是坏的」这种情况，让上面那条断言不至于空过。
#[tokio::test]
async fn a_well_behaved_server_still_downloads_without_extra_requests() {
    let payload = Arc::new(payload_bytes(PAYLOAD_LEN));
    let log: ReadLog = Arc::new(StdMutex::new(Vec::new()));
    let (transferred, written, logged) = pipelined_download_from(
        FakeSftpServer::new(Arc::clone(&payload), None, Arc::clone(&log)),
        Arc::clone(&log),
        payload.len() as u64,
    )
    .await;

    assert_eq!(payload.len() as u64, transferred);
    assert_eq!(*payload, written);
    assert!(
        logged
            .iter()
            .all(|(requested, returned)| requested == &(*returned as u32)),
        "读满的服务端不该触发续读：{logged:?}"
    );
    // 9 个满片 + 1 个残片，一个分片一发。
    assert_eq!(
        10,
        logged.len(),
        "读满的服务端应当恰好一枪一个分片：{logged:?}"
    );
}
