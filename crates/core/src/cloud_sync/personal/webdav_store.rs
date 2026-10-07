//! WebDAV 个人同步后端
//!
//! ## 为什么不用 PROPFIND 列目录
//!
//! 标准 WebDAV 用 `PROPFIND` 列目录、用 `MKCOL` 建目录。但各服务端实现差异很大：
//! 坚果云对部分方法有限流和路径约束，群晖默认是 `https://host:5006` 且根目录语义与 Apache
//! 不一致，Nextcloud 的路径必须带 `/remote.php/dav/files/<user>/`，自建 nginx/apache 又各有一套。
//! 客户端依赖 gpui 的 `HttpClient` 转发方法，多一个 WebDAV 专有方法就多一处兼容风险。
//!
//! 因此这里改用「**扁平文件 + 索引**」方案：
//!
//! - 只用到 `GET` / `PUT` `DELETE` 三个最基础的方法，任何 WebDAV 服务端都支持；
//! - 每条记录一个文件，文件名由「数据类型 + 记录 ID」消毒而成；
//! - `index.json` 维护 记录键 -> (updated_at, version, checksum) 的映射，列目录只读它，
//!   并可按 `since` / `data_type` 提前过滤，避免整包下载。
//!
//! 单机场景下这与目录后端等价；多机并发写 `index.json` 时采用「读改写 + 失败重试一次」，
//! 极端情况下并发双方可能只丢失索引中的一条指向（记录文件本身仍然存在），随后一轮全量
//! 扫描会把它补回来。
//!
//! ## 远端文件布局
//!
//! ```text
//! <url>/manifest.json                  包清单
//! <url>/index.json                     记录索引
//! <url>/record-<type>-<id>.json        单条记录
//! <url>/tombstone-<type>-<id>.json     墓碑（软删除留痕，便于跨后端兼容）
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request, StatusCode};
use serde::{Deserialize, Serialize};

use crate::cloud_sync::models::CloudSyncData;

use super::{
    APP_ID, PERSONAL_PROFILE_ID, PersonalSyncManifest, PersonalSyncStore, SUPPORTED_SCHEMA_VERSION,
    SyncDeviceId, SyncStoreError, SyncStoreLock, SyncStoreStatus, SyncTombstone,
};

const MANIFEST_FILE: &str = "manifest.json";
const INDEX_FILE: &str = "index.json";
const RECORD_PREFIX: &str = "record-";
const TOMBSTONE_PREFIX: &str = "tombstone-";
const FILE_SUFFIX: &str = ".json";
const KEY_SEPARATOR: char = '/';
/// 写 `index.json` 冲突时的重试次数。
const INDEX_WRITE_ATTEMPTS: usize = 2;

/// WebDAV 连接凭据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebDavCredentials {
    /// 远端集合地址，例如 `https://dav.jianguoyun.com/dav/navop/`
    pub url: String,
    pub username: String,
    /// 明文密码（由调用方从加密存储解密后传入，不落盘）。
    pub password: String,
}

/// `index.json` 的内容。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebDavSyncIndex {
    /// 每次写入自增，用于读改写时观察并发。
    #[serde(default)]
    pub version: u32,
    /// 记录键（`<data_type>/<id>`）到元数据的映射。
    #[serde(default)]
    pub records: BTreeMap<String, WebDavIndexEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebDavIndexEntry {
    pub updated_at: i64,
    pub version: u32,
    #[serde(default)]
    pub checksum: String,
}

/// 一次 HTTP 交互的结果。
struct HttpReply {
    status: StatusCode,
    body: Vec<u8>,
}

impl HttpReply {
    fn is_success(&self) -> bool {
        self.status.is_success()
    }
}

#[derive(Clone)]
pub struct WebDavSyncStore {
    http: Arc<dyn HttpClient>,
    /// 规范化后的基地址，一定以 `/` 结尾。
    base_url: String,
    username: String,
    password: String,
    /// 目标集合是否已确认存在。首次写入前用 MKCOL 建一次，之后不再重复。
    collection_ready: Arc<AtomicBool>,
}

impl WebDavSyncStore {
    pub fn new(
        http: Arc<dyn HttpClient>,
        credentials: WebDavCredentials,
    ) -> Result<Self, SyncStoreError> {
        let url = credentials.url.trim().to_string();
        if url.is_empty() || credentials.username.trim().is_empty() {
            return Err(SyncStoreError::NotConfigured);
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(SyncStoreError::Io(format!(
                "WebDAV 服务器地址必须以 http:// 或 https:// 开头：{url}"
            )));
        }

        Ok(Self {
            http,
            base_url: normalize_base_url(&url),
            username: credentials.username.trim().to_string(),
            password: credentials.password,
            collection_ready: Arc::new(AtomicBool::new(false)),
        })
    }

    // ------------------------------------------------------------------
    // 路径与文件名
    // ------------------------------------------------------------------

    fn child_url(&self, file_name: &str) -> String {
        format!("{}{}", self.base_url, file_name)
    }

    // ------------------------------------------------------------------
    // HTTP 原语
    // ------------------------------------------------------------------

    fn basic_auth(&self) -> String {
        let raw = format!("{}:{}", self.username, self.password);
        format!("Basic {}", BASE64.encode(raw.as_bytes()))
    }

    async fn send(
        &self,
        method: Method,
        file_name: &str,
        body: Option<Vec<u8>>,
    ) -> Result<HttpReply, SyncStoreError> {
        let url = self.child_url(file_name);
        let mut builder = Request::builder()
            .method(method.clone())
            .uri(url.as_str())
            .header("Authorization", self.basic_auth());
        if body.is_some() {
            builder = builder.header("Content-Type", "application/json");
        }
        let payload = body.map(AsyncBody::from).unwrap_or_else(AsyncBody::empty);
        let request = builder
            .body(payload)
            .map_err(|error| SyncStoreError::Io(format!("构造 WebDAV 请求失败: {error}")))?;

        let response = self
            .http
            .send(request)
            .await
            .map_err(|error| SyncStoreError::WebdavUnreachable(error.to_string()))?;

        let status = response.status();
        let mut response_body = response.into_body();
        let mut bytes = Vec::new();
        response_body
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| SyncStoreError::Io(format!("读取 WebDAV 响应失败: {error}")))?;

        Ok(HttpReply {
            status,
            body: bytes,
        })
    }

    /// 把非 2xx 响应翻译成领域错误。
    ///
    /// 注意：**这里不把 409 当成版本冲突**。本实现从不发送 `If-Match` 之类的条件请求头，
    /// 所以服务端返回的 409 只会是「父集合不存在」——`PUT` 无法凭空创建中间目录。
    /// 把它误判成 `Conflict` 会让 worker 进入重试退避并最终「多次失败后暂停」，
    /// 而真正需要的只是先 `MKCOL` 建一次目录。
    fn ensure_success(&self, reply: &HttpReply) -> Result<(), SyncStoreError> {
        if reply.is_success() {
            return Ok(());
        }

        let status = reply.status.as_u16();
        Err(match reply.status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => SyncStoreError::WebdavAuthFailed,

            // 目录层面的问题：映射到 DirectoryUnavailable，界面显示「目录不可用」并给出
            // 可操作提示，而不是误导性的「多次失败后暂停」。
            StatusCode::NOT_FOUND | StatusCode::CONFLICT | StatusCode::PRECONDITION_FAILED => {
                SyncStoreError::DirectoryUnavailable(format!(
                    "目标目录不存在或不可写（HTTP {status}）：{}。\
                     请在 WebDAV 服务器上确认该目录已创建（例如坚果云网页版新建文件夹），\
                     或改用一个已存在的目录。",
                    reply_snippet(reply)
                ))
            }
            StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_IMPLEMENTED => {
                SyncStoreError::DirectoryUnavailable(format!(
                    "服务端不支持该操作（HTTP {status}）：{}。\
                     请确认地址指向一个支持 WebDAV 写入的目录，且账号有写权限。",
                    reply_snippet(reply)
                ))
            }
            StatusCode::INSUFFICIENT_STORAGE | StatusCode::PAYLOAD_TOO_LARGE => {
                SyncStoreError::WebdavStatus {
                    status,
                    message: format!("服务端存储空间不足或拒绝写入：{}", reply_snippet(reply)),
                }
            }
            _ => SyncStoreError::WebdavStatus {
                status,
                message: reply_snippet(reply),
            },
        })
    }

    /// 确保远端目标集合存在，必要时用 `MKCOL` 建一次。
    ///
    /// 这是扁平布局能落地的关键：记录文件名虽然不带目录，但 `PUT` 仍要求父集合已存在。
    /// 坚果云、群晖、Nextcloud、Apache mod_dav、nginx dav 都支持 MKCOL。
    async fn ensure_collection(&self) -> Result<(), SyncStoreError> {
        if self.collection_ready.load(Ordering::Relaxed) {
            return Ok(());
        }

        let reply = self.send(mkcol_method(), "", None).await?;
        // 2xx = 新建成功；301/405/409 = 已经存在，同样视为就绪。
        if reply.is_success() || matches!(reply.status.as_u16(), 301 | 405 | 409) {
            self.collection_ready.store(true, Ordering::Relaxed);
            return Ok(());
        }
        self.ensure_success(&reply)
    }

    async fn download(&self, file_name: &str) -> Result<Option<Vec<u8>>, SyncStoreError> {
        let reply = self.send(Method::GET, file_name, None).await?;
        if reply.status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        self.ensure_success(&reply)?;
        Ok(Some(reply.body))
    }

    async fn upload(
        &self,
        file_name: &str,
        value: &(impl Serialize + Sync + Send),
    ) -> Result<(), SyncStoreError> {
        let bytes = serde_json::to_vec_pretty(value)?;
        // PUT 不会创建中间目录，必须先把集合建出来。
        self.ensure_collection().await?;
        // 首次尝试 clone 一份：失败重试时还要用同一份 payload。
        let reply = self
            .send(Method::PUT, file_name, Some(bytes.clone()))
            .await?;
        if reply.is_success() {
            return Ok(());
        }
        // 服务端可能在两次调用之间把目录删掉，或首次 MKCOL 因父目录缺失而失败；
        // 失效一次缓存后重试一次，再失败就如实报错。
        self.collection_ready.store(false, Ordering::Relaxed);
        if matches!(reply.status.as_u16(), 404 | 409) {
            self.ensure_collection().await?;
            let retry = self.send(Method::PUT, file_name, Some(bytes)).await?;
            return self.ensure_success(&retry);
        }
        self.ensure_success(&reply)
    }

    // ------------------------------------------------------------------
    // 索引读写
    // ------------------------------------------------------------------

    async fn load_index(&self) -> Result<WebDavSyncIndex, SyncStoreError> {
        let Some(bytes) = self.download(INDEX_FILE).await? else {
            // 首次使用时还没有索引，视为空索引。
            return Ok(WebDavSyncIndex::default());
        };
        serde_json::from_slice(&bytes).map_err(|error| {
            SyncStoreError::Parse(format!("{INDEX_FILE} 解析失败: {error}"))
        })
    }

    /// 读改写索引：取出最新索引，应用 `mutate`，再整体回写。
    ///
    /// 写失败（通常是被并发方抢先）时重新读取一次再改，最多 [`INDEX_WRITE_ATTEMPTS`] 轮。
    ///
    /// `mutate` 必须 `Send`：`#[async_trait]` 生成的 future 要求 `Send`，而闭包会被
    /// 跨 `.await` 持有在 future 状态里。
    async fn mutate_index(
        &self,
        mutate: impl Fn(&mut WebDavSyncIndex) + Send,
    ) -> Result<(), SyncStoreError> {
        let mut last_error: Option<SyncStoreError> = None;

        for _ in 0..INDEX_WRITE_ATTEMPTS {
            let mut index = self.load_index().await?;
            mutate(&mut index);
            index.version = index.version.saturating_add(1);

            let bytes = serde_json::to_vec_pretty(&index)?;
            let reply = self.send(Method::PUT, INDEX_FILE, Some(bytes)).await?;
            if reply.is_success() {
                return Ok(());
            }
            last_error = self.ensure_success(&reply).err();
        }

        Err(last_error.unwrap_or_else(|| {
            SyncStoreError::Io(format!("写入 {INDEX_FILE} 失败且原因未知"))
        }))
    }

    // ------------------------------------------------------------------
    // 清单
    // ------------------------------------------------------------------

    async fn ensure_manifest(&self) -> Result<(), SyncStoreError> {
        match self.download(MANIFEST_FILE).await? {
            Some(bytes) => {
                let manifest: PersonalSyncManifest = serde_json::from_slice(&bytes)
                    .map_err(|error| {
                        SyncStoreError::Parse(format!("{MANIFEST_FILE} 解析失败: {error}"))
                    })?;
                manifest.validate()
            }
            None => self.upload(MANIFEST_FILE, &default_manifest()).await,
        }
    }
}

#[async_trait]
impl PersonalSyncStore for WebDavSyncStore {
    fn backend_id(&self) -> &'static str {
        "webdav"
    }

    async fn probe(&self) -> Result<SyncStoreStatus, SyncStoreError> {
        // 写清单既是连通性检测，也是对写权限的检测：401/403 会在这里被翻译成 WebdavAuthFailed。
        self.ensure_manifest().await?;
        // 顺带确认索引可读，避免到真正同步时才报错。
        self.load_index().await?;
        Ok(SyncStoreStatus::ready())
    }

    async fn list_records(
        &self,
        data_type: Option<&str>,
        since: Option<i64>,
    ) -> Result<Vec<CloudSyncData>, SyncStoreError> {
        let index = self.load_index().await?;
        let mut records = Vec::with_capacity(index.records.len());

        for (key, entry) in index.records.iter() {
            let Some((record_type, record_id)) = split_entry_key(key) else {
                continue;
            };
            if data_type.is_some_and(|target| target != record_type) {
                continue;
            }
            if since.is_some_and(|timestamp| entry.updated_at < timestamp) {
                continue;
            }

            let file_name = record_file_name(record_type, record_id);
            let Some(bytes) = self.download(&file_name).await? else {
                // 索引里有指向但文件已被外部删除：跳过，别让一条悬空记录拖垮整轮同步。
                tracing::warn!("[webdav] 索引指向的记录文件缺失，已跳过: {file_name}");
                continue;
            };
            match serde_json::from_slice::<CloudSyncData>(&bytes) {
                Ok(record) => records.push(record),
                Err(error) => {
                    // 单条记录损坏不应让整轮同步失败。
                    tracing::warn!("[webdav] 记录文件解析失败，已跳过 {file_name}: {error}");
                }
            }
        }

        records.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(records)
    }

    async fn upsert_record(
        &self,
        record: &CloudSyncData,
        expected_version: Option<u32>,
    ) -> Result<CloudSyncData, SyncStoreError> {
        self.ensure_manifest().await?;

        let index = self.load_index().await?;
        let key = entry_key(&record.data_type, &record.id);
        let existing_version = index.records.get(&key).map(|entry| entry.version);

        if let Some(expected) = expected_version {
            match existing_version {
                Some(version) if version == expected => {}
                _ => {
                    return Err(SyncStoreError::Conflict(format!(
                        "stale version for record {}",
                        record.id
                    )));
                }
            }
        }

        let stored = next_stored_record(record.clone(), existing_version);
        self.upload(&record_file_name(&stored.data_type, &stored.id), &stored)
            .await?;

        let entry = index_entry(&stored);
        self.mutate_index(|index| {
            index.records.insert(key.clone(), entry.clone());
        })
        .await?;

        Ok(stored)
    }

    async fn tombstone_record(
        &self,
        data_type: &str,
        id: &str,
        expected_version: Option<u32>,
    ) -> Result<(), SyncStoreError> {
        self.ensure_manifest().await?;

        let index = self.load_index().await?;
        let key = entry_key(data_type, id);
        let current_version = index
            .records
            .get(&key)
            .map(|entry| entry.version)
            .ok_or_else(|| {
                SyncStoreError::Conflict(format!("missing {data_type} record {id}"))
            })?;

        if let Some(expected) = expected_version {
            if current_version != expected {
                return Err(SyncStoreError::Conflict(format!(
                    "stale version for record {id}"
                )));
            }
        }

        let file_name = record_file_name(data_type, id);
        let bytes = self.download(&file_name).await?.ok_or_else(|| {
            SyncStoreError::Conflict(format!("missing {data_type} record {id}"))
        })?;
        let mut record: CloudSyncData = serde_json::from_slice(&bytes)?;

        let deleted_at = now_millis();
        record.deleted_at = Some(deleted_at);
        record.updated_at = deleted_at;
        record.version = record.version.saturating_add(1);

        let tombstone = SyncTombstone {
            id: record.id.clone(),
            data_type: record.data_type.clone(),
            deleted_at,
            version: record.version,
            checksum: record.checksum.clone(),
        };

        // 先写墓碑，再写记录：万一中间失败，记录仍可读，不会丢数据。
        self.upload(&tombstone_file_name(data_type, id), &tombstone)
            .await?;
        self.upload(&file_name, &record).await?;

        // 软删除后条目仍留在 `records` 里：worker 依赖读到带 deleted_at 的记录来感知远端删除。
        let entry = index_entry(&record);
        self.mutate_index(|index| {
            index.records.insert(key.clone(), entry.clone());
        })
        .await?;

        Ok(())
    }

    async fn acquire_lock(&self, owner: &SyncDeviceId) -> Result<SyncStoreLock, SyncStoreError> {
        Ok(SyncStoreLock {
            owner: owner.clone(),
        })
    }
}

// ============================================================================
// 纯函数
// ============================================================================

/// 补上结尾的 `/`，并保留用户可能拼错的反斜杠修正。
fn normalize_base_url(url: &str) -> String {
    let url = url.replace('\\', "/");
    if url.ends_with('/') {
        url
    } else {
        format!("{url}/")
    }
}

fn entry_key(data_type: &str, id: &str) -> String {
    format!("{data_type}{KEY_SEPARATOR}{id}")
}

fn split_entry_key(key: &str) -> Option<(&str, &str)> {
    key.split_once(KEY_SEPARATOR)
}

/// 把键中不适合放进 URL 的字符替换掉。
///
/// 索引才是键的权威来源，文件名只需稳定且路径安全，不需要反解。
fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => character,
            _ => '-',
        })
        .collect()
}

pub(crate) fn record_file_name(data_type: &str, id: &str) -> String {
    format!(
        "{}{}-{}{}",
        RECORD_PREFIX,
        sanitize(data_type),
        sanitize(id),
        FILE_SUFFIX
    )
}

pub(crate) fn tombstone_file_name(data_type: &str, id: &str) -> String {
    format!(
        "{}{}-{}{}",
        TOMBSTONE_PREFIX,
        sanitize(data_type),
        sanitize(id),
        FILE_SUFFIX
    )
}

/// MKCOL（RFC 4918）：在远端创建集合。
///
/// 这是 WebDAV 的基础方法，坚果云 / 群晖 / Nextcloud / Apache mod_dav / nginx dav
/// 都支持；不像 PROPFIND 那样各方实现差异大。
fn mkcol_method() -> Method {
    Method::from_bytes(b"MKCOL").expect("MKCOL 是合法的 HTTP 方法字面量")
}

fn default_manifest() -> PersonalSyncManifest {
    let now = now_millis();
    PersonalSyncManifest {
        schema_version: SUPPORTED_SCHEMA_VERSION,
        app: APP_ID.to_string(),
        profile_id: PERSONAL_PROFILE_ID.to_string(),
        created_at: now,
        updated_at: now,
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn next_stored_record(mut record: CloudSyncData, existing_version: Option<u32>) -> CloudSyncData {
    record.version = match existing_version {
        Some(version) => version.saturating_add(1),
        None => record.version.max(1),
    };
    record.updated_at = now_millis();
    record
}

fn index_entry(record: &CloudSyncData) -> WebDavIndexEntry {
    WebDavIndexEntry {
        updated_at: record.updated_at,
        version: record.version,
        checksum: record.checksum.clone(),
    }
}

/// 截取响应体的一小段纯文本，便于把服务端原因透传给界面。
fn reply_snippet(reply: &HttpReply) -> String {
    const MAX_SNIPPET_CHARS: usize = 200;

    let text = String::from_utf8_lossy(&reply.body);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "服务端未返回错误信息".to_string();
    }

    trimmed.chars().take(MAX_SNIPPET_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};

    use futures::future::BoxFuture;
    use futures::FutureExt;
    use gpui::http_client::{AsyncBody, HttpClient, Request, Response, StatusCode, Url};

    use super::{
        INDEX_FILE, MANIFEST_FILE, WebDavCredentials, WebDavSyncStore, WebDavSyncIndex, entry_key,
        normalize_base_url, record_file_name, tombstone_file_name,
    };
    use crate::cloud_sync::models::{CloudSyncData, data_type};
    use crate::cloud_sync::personal::{PersonalSyncStore, SyncStoreError};
    use crate::cloud_sync::personal::test_support::test_record;

    /// 一个实现 GET / PUT / MKCOL 的内存版 WebDAV 服务端。
    ///
    /// 这样测试跑的是真实的 store 逻辑（建集合、索引读写、版本判定、错误翻译），
    /// 只是把网络层换成了字典。集合语义也照搬 RFC 4918：集合不存在时
    /// MKCOL 返回 201，PUT 返回 409。
    #[derive(Default)]
    struct MemoryWebDav {
        files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        /// 已创建的集合；空表示集合尚不存在。
        collections: Arc<Mutex<HashSet<String>>>,
        /// 非 None 时所有请求直接返回这个状态码，用来模拟 401 / 500。
        forced_status: Option<u16>,
        /// 服务端错误响应体。
        error_body: String,
        /// 记录每个请求的方法，便于断言只用到基础方法。
        methods: Arc<Mutex<Vec<String>>>,
    }

    impl MemoryWebDav {
        fn new() -> Arc<MemoryWebDav> {
            Arc::new(Self::default())
        }

        /// 集合已存在的服务端。
        ///
        /// 集合在 mock 里是以 **URL 路径** 为键的（`MKCOL` 分支用 `req.uri().path()` 写入），
        /// 所以这里必须塞 store 真正请求的那条路径。早先写死的 `String::new()`
        /// 永远匹配不上，`MKCOL` 会一律回 201 —— 「服务端已存在该集合」这条分支
        /// 从来没有被测到过。
        fn with_collection() -> Arc<MemoryWebDav> {
            let server = Self::new();
            server
                .collections
                .lock()
                .expect("lock")
                .insert(collection_path());
            server
        }

        fn failing(status: u16) -> Arc<MemoryWebDav> {
            Arc::new(MemoryWebDav {
                forced_status: Some(status),
                error_body: "server said no".to_string(),
                ..MemoryWebDav::default()
            })
        }

        fn file(&self, name: &str) -> Option<Vec<u8>> {
            self.files.lock().expect("files lock").get(name).cloned()
        }

        fn contains(&self, name: &str) -> bool {
            self.files.lock().expect("files lock").contains_key(name)
        }

        fn collection_exists(&self) -> bool {
            !self.collections.lock().expect("collections lock").is_empty()
        }

        fn count(&self) -> usize {
            self.files.lock().expect("files lock").len()
        }

        fn index(&self) -> WebDavSyncIndex {
            let bytes = self.file(INDEX_FILE).expect("index exists");
            serde_json::from_slice(&bytes).expect("index parses")
        }

        fn record_count(&self) -> usize {
            self.index().records.len()
        }

        fn methods_seen(&self) -> Vec<String> {
            self.methods.lock().expect("methods lock").clone()
        }
    }

    /// 包一层本地类型再实现 `HttpClient`：避免 `impl ForeignTrait for Arc<Local>` 的孤儿规则限制。
    struct MemoryWebDavClient(Arc<MemoryWebDav>);

    impl MemoryWebDavClient {
        fn new(server: &Arc<MemoryWebDav>) -> Arc<dyn HttpClient> {
            Arc::new(MemoryWebDavClient(Arc::clone(server)))
        }
    }

    impl HttpClient for MemoryWebDavClient {
        fn user_agent(&self) -> Option<&gpui::http_client::http::HeaderValue> {
            None
        }

        fn send(
            &self,
            req: Request<AsyncBody>,
        ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
            let server = Arc::clone(&self.0);
            let forced_status = server.forced_status;
            let error_body = server.error_body.clone();

            async move {
                let method = req.method().to_string();
                let path = req.uri().path().to_string();
                // 必须先把 body 读完再动文件表：Put 的 payload 在 body 里。
                let payload = read_body(req).await;

                server
                    .methods
                    .lock()
                    .expect("methods lock")
                    .push(method.clone());

                if let Some(status) = forced_status {
                    return build_response(status, error_body.into_bytes());
                }

                let mut collections = server.collections.lock().expect("collections lock");

                // 集合层：URL 以 / 结尾且没有文件名，就是对集合本身操作。
                if method == "MKCOL" {
                    return if collections.insert(path.clone()) {
                        build_response(StatusCode::CREATED.as_u16(), Vec::new())
                    } else {
                        build_response(StatusCode::METHOD_NOT_ALLOWED.as_u16(), Vec::new())
                    };
                }

                // RFC 4918：父集合不存在时 PUT 返回 409（不会凭空创建中间目录）；
                // 而 GET 只会得到 404，客户端据此认为「文件还不存在」。
                if method == "PUT" && !collections.contains(&parent_of(&path)) {
                    return build_response(StatusCode::CONFLICT.as_u16(), Vec::new());
                }

                let file_name = path.rsplit('/').next().unwrap_or_default().to_string();
                let mut files = server.files.lock().expect("files lock");

                match method.as_str() {
                    "GET" => match files.get(&file_name) {
                        Some(bytes) => build_response(StatusCode::OK.as_u16(), bytes.clone()),
                        None => build_response(StatusCode::NOT_FOUND.as_u16(), Vec::new()),
                    },
                    "PUT" => {
                        files.insert(file_name, payload);
                        build_response(StatusCode::CREATED.as_u16(), Vec::new())
                    }
                    "DELETE" => {
                        files.remove(&file_name);
                        build_response(StatusCode::NO_CONTENT.as_u16(), Vec::new())
                    }
                    other => panic!("测试用服务端收到了未预期的 {other} 请求"),
                }
            }
            .boxed()
        }

        fn proxy(&self) -> Option<&gpui::http_client::Url> {
            None
        }
    }

    /// 取路径的父集合部分。`/dav/navop/record-x.json` -> `/dav/navop/`
    fn parent_of(path: &str) -> String {
        match path.rfind('/') {
            Some(index) => path[..=index].to_string(),
            None => "/".to_string(),
        }
    }

    async fn read_body(req: Request<AsyncBody>) -> Vec<u8> {
        use futures::AsyncReadExt;

        let mut body = req.into_body();
        let mut bytes = Vec::new();
        body.read_to_end(&mut bytes).await.expect("body reads");
        bytes
    }

    fn build_response(status: u16, body: Vec<u8>) -> anyhow::Result<Response<AsyncBody>> {
        Response::builder()
            .status(status)
            .body(AsyncBody::from(body))
            .map_err(|error| anyhow::anyhow!(error))
    }

    fn credentials(url: &str) -> WebDavCredentials {
        WebDavCredentials {
            url: url.to_string(),
            username: "user".to_string(),
            password: "pass".to_string(),
        }
    }

    fn store(server: &Arc<MemoryWebDav>) -> WebDavSyncStore {
        WebDavSyncStore::new(MemoryWebDavClient::new(server), credentials(BASE_URL))
            .expect("store builds")
    }

    const BASE_URL: &str = "https://dav.example.com/dav/navop";

    /// `store()` 请求目标集合时落到 mock 上的 URL 路径。
    ///
    /// 复用 store 自己的归一化逻辑（补尾部 `/`）再取 path：mock 的集合键就是
    /// `req.uri().path()`，两边必须用同一条路径，否则「集合已存在」根本模拟不出来。
    fn collection_path() -> String {
        Url::parse(&normalize_base_url(BASE_URL))
            .expect("BASE_URL 可解析")
            .path()
            .to_string()
    }

    fn record_for(id: &str) -> CloudSyncData {
        test_record(id, data_type::CONNECTION, 1, "checksum")
    }

    // ------------------------------------------------------------------
    // 配置校验
    // ------------------------------------------------------------------

    #[test]
    fn missing_server_url_is_rejected() {
        let result = WebDavSyncStore::new(
            MemoryWebDavClient::new(&MemoryWebDav::new()),
            WebDavCredentials {
                url: "   ".to_string(),
                username: "user".to_string(),
                password: "pass".to_string(),
            },
        );

        assert_eq!(Some(SyncStoreError::NotConfigured), result.err());
    }

    #[test]
    fn missing_username_is_rejected() {
        let result = WebDavSyncStore::new(
            MemoryWebDavClient::new(&MemoryWebDav::new()),
            WebDavCredentials {
                url: BASE_URL.to_string(),
                username: "  ".to_string(),
                password: "pass".to_string(),
            },
        );

        assert_eq!(Some(SyncStoreError::NotConfigured), result.err());
    }

    #[test]
    fn non_http_url_is_rejected() {
        let result = WebDavSyncStore::new(
            MemoryWebDavClient::new(&MemoryWebDav::new()),
            credentials("dav.example.com/dav"),
        );

        assert!(matches!(result, Err(SyncStoreError::Io(_))));
    }

    #[test]
    fn base_url_always_ends_with_a_slash() {
        let store = store(&MemoryWebDav::new());

        assert_eq!("https://dav.example.com/dav/navop/", store.base_url);
    }

    #[test]
    fn file_names_are_path_safe() {
        assert_eq!(
            "record-connection-abc-123.json",
            record_file_name("connection", "abc 123")
        );
        assert_eq!(
            "tombstone-connection-abc-123.json",
            tombstone_file_name("connection", "abc 123")
        );
    }

    // ------------------------------------------------------------------
    // 协议行为
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn probe_writes_manifest_when_absent() {
        let server = MemoryWebDav::new();

        store(&server)
            .probe()
            .await
            .expect("probe against an empty collection succeeds");

        assert!(server.contains(MANIFEST_FILE));
        assert_eq!("webdav", store(&server).backend_id());
    }

    #[tokio::test]
    async fn upsert_then_list_round_trips_a_record() {
        let server = MemoryWebDav::new();
        let store = store(&server);

        let upserted = store
            .upsert_record(&record_for("cloud-1"), None)
            .await
            .expect("upsert succeeds");

        assert_eq!(1, upserted.version);
        assert_eq!(data_type::CONNECTION, upserted.data_type);

        let listed = store
            .list_records(Some(data_type::CONNECTION), None)
            .await
            .expect("list succeeds");

        assert_eq!(1, listed.len());
        assert_eq!("cloud-1", listed[0].id);
        assert_eq!("checksum", listed[0].checksum);
    }

    #[tokio::test]
    async fn upsert_bumps_the_stored_version() {
        let server = MemoryWebDav::new();
        let store = store(&server);

        let first = store
            .upsert_record(&record_for("cloud-1"), None)
            .await
            .expect("first upsert");
        let second = store
            .upsert_record(&record_for("cloud-1"), None)
            .await
            .expect("second upsert");

        assert_eq!(1, first.version);
        assert_eq!(2, second.version);
    }

    #[tokio::test]
    async fn upsert_honours_expected_version() {
        let server = MemoryWebDav::new();
        let store = store(&server);
        store
            .upsert_record(&record_for("cloud-1"), None)
            .await
            .expect("seed");

        let result = store.upsert_record(&record_for("cloud-1"), Some(7)).await;

        assert!(matches!(result, Err(SyncStoreError::Conflict(_))));
    }

    #[tokio::test]
    async fn list_filters_by_data_type_and_since() {
        let server = MemoryWebDav::new();
        let store = store(&server);
        store
            .upsert_record(&record_for("cloud-1"), None)
            .await
            .expect("connection seed");
        store
            .upsert_record(&test_record("cred-1", data_type::CREDENTIAL, 1, "c"), None)
            .await
            .expect("credential seed");

        let connections = store
            .list_records(Some(data_type::CONNECTION), None)
            .await
            .expect("filtered list");
        assert_eq!(1, connections.len());
        assert_eq!(data_type::CONNECTION, connections[0].data_type);

        let nothing = store
            .list_records(None, Some(i64::MAX / 2))
            .await
            .expect("since-filtered list");
        assert!(nothing.is_empty());
    }

    #[tokio::test]
    async fn tombstone_keeps_the_record_visible_so_remote_deletes_propagate() {
        let server = MemoryWebDav::new();
        let store = store(&server);
        store
            .upsert_record(&record_for("cloud-1"), None)
            .await
            .expect("seed");

        store
            .tombstone_record(data_type::CONNECTION, "cloud-1", None)
            .await
            .expect("tombstone succeeds");

        let listed = store
            .list_records(Some(data_type::CONNECTION), None)
            .await
            .expect("list succeeds");
        assert_eq!(1, listed.len(), "软删除的记录仍需被 worker 读到");
        assert!(listed[0].deleted_at.is_some());

        let tombstone_name = tombstone_file_name(data_type::CONNECTION, "cloud-1");
        assert!(server.contains(&tombstone_name), "应写出墓碑文件");
    }

    #[tokio::test]
    async fn tombstone_rejects_a_missing_record() {
        let server = MemoryWebDav::new();
        let store = store(&server);

        let result = store
            .tombstone_record(data_type::CONNECTION, "ghost", None)
            .await;

        assert!(matches!(result, Err(SyncStoreError::Conflict(_))));
    }

    #[tokio::test]
    async fn tombstone_honours_expected_version() {
        let server = MemoryWebDav::new();
        let store = store(&server);
        store
            .upsert_record(&record_for("cloud-1"), None)
            .await
            .expect("seed");

        let result = store
            .tombstone_record(data_type::CONNECTION, "cloud-1", Some(99))
            .await;

        assert!(matches!(result, Err(SyncStoreError::Conflict(_))));
    }

    #[tokio::test]
    async fn list_skips_dangling_index_entries_instead_of_failing() {
        let server = MemoryWebDav::new();
        let store = store(&server);
        store
            .upsert_record(&record_for("cloud-1"), None)
            .await
            .expect("seed");
        server
            .files
            .lock()
            .expect("files lock")
            .remove(&record_file_name(data_type::CONNECTION, "cloud-1"));

        let listed = store
            .list_records(None, None)
            .await
            .expect("listing must not fail on a dangling entry");

        assert!(listed.is_empty());
    }

    // ------------------------------------------------------------------
    // 错误翻译
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn authentication_failures_surface_as_webdav_auth_failed() {
        for status in [401, 403] {
            let server = MemoryWebDav::failing(status);
            let result = store(&server).probe().await;

            assert!(
                matches!(result, Err(SyncStoreError::WebdavAuthFailed)),
                "status {status} 应翻译为 WebdavAuthFailed，实际为 {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn server_errors_carry_the_status_and_body_snippet() {
        let server = MemoryWebDav::failing(500);
        let result = store(&server).probe().await;

        match result {
            Err(SyncStoreError::WebdavStatus { status, message }) => {
                assert_eq!(500, status);
                assert!(message.contains("server said no"), "message: {message}");
            }
            other => panic!("期望 WebdavStatus，实际为 {other:?}"),
        }
    }

    #[tokio::test]
    async fn unsupported_methods_explain_the_writable_directory_hint() {
        let server = MemoryWebDav::failing(405);
        let result = store(&server).probe().await;

        match result {
            Err(SyncStoreError::DirectoryUnavailable(message)) => {
                assert!(message.contains("405"), "应带上状态码：{message}");
                assert!(
                    message.contains("写权限"),
                    "提示应说明需要写权限：{message}"
                );
            }
            other => panic!("期望 DirectoryUnavailable，实际为 {other:?}"),
        }
    }

    // ------------------------------------------------------------------
    // 兼容性护栏
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn only_basic_webdav_methods_are_used() {
        let server = MemoryWebDav::new();
        let store = store(&server);

        store.probe().await.expect("probe");
        store
            .upsert_record(&record_for("cloud-1"), None)
            .await
            .expect("upsert");
        store
            .list_records(None, None)
            .await
            .expect("list");

        let mut methods = server.methods_seen();
        methods.sort();
        methods.dedup();
        // MKCOL 只在集合缺失时发一次；刻意不使用 PROPFIND / PROPPATCH。
        assert_eq!(
            vec!["GET".to_string(), "MKCOL".to_string(), "PUT".to_string()],
            methods
        );
    }

    #[tokio::test]
    async fn probe_creates_the_target_collection_with_mkcol() {
        let server = MemoryWebDav::new();
        assert!(!server.collection_exists(), "集合初始应不存在");

        store(&server).probe().await.expect("probe");

        assert!(server.collection_exists(), "probe 应先用 MKCOL 建集合");
        assert!(server.contains(MANIFEST_FILE));
    }

    #[tokio::test]
    async fn existing_collection_is_not_recreated() {
        // 服务端上集合已存在。store 刻意不用 PROPFIND 探测目录，所以 probe 仍会发
        // 一次 MKCOL；服务端回 405「已存在」，必须被当成「目录就绪」而不是错误。
        let server = MemoryWebDav::with_collection();
        let store = store(&server);
        store
            .probe()
            .await
            .expect("集合已存在时 probe 应成功（405 视为就绪）");

        // 再写一条记录：`collection_ready` 已置位，不允许重复发 MKCOL。
        store
            .upsert_record(&record_for("cloud-1"), None)
            .await
            .expect("upsert");

        let methods = server.methods_seen();
        assert_eq!(
            1,
            methods.iter().filter(|method| method.as_str() == "MKCOL").count(),
            "集合已就绪后不应重复发 MKCOL，实际方法序列：{methods:?}"
        );
    }

    #[tokio::test]
    async fn unresolvable_directory_reports_directory_unavailable_with_a_hint() {
        // 服务端一律 409：MKCOL 被当成「已存在」放行，PUT 仍失败。
        let server = MemoryWebDav::failing(409);
        let result = store(&server).probe().await;

        match result {
            Err(SyncStoreError::DirectoryUnavailable(message)) => {
                assert!(message.contains("目标目录不存在"), "message: {message}");
                assert!(message.contains("创建"), "提示应告诉用户怎么建目录：{message}");
            }
            other => panic!("期望 DirectoryUnavailable，实际为 {other:?}"),
        }
    }

    #[tokio::test]
    async fn index_tracks_every_upserted_record() {
        let server = MemoryWebDav::new();
        let store = store(&server);

        store
            .upsert_record(&record_for("cloud-1"), None)
            .await
            .expect("first");
        store
            .upsert_record(&test_record("cred-1", data_type::CREDENTIAL, 1, "c"), None)
            .await
            .expect("second");

        assert_eq!(2, server.record_count());
        let index = server.index();
        assert!(index
            .records
            .contains_key(&entry_key(data_type::CONNECTION, "cloud-1")));
        assert!(index
            .records
            .contains_key(&entry_key(data_type::CREDENTIAL, "cred-1")));
        // manifest + index + 两条记录
        assert_eq!(4, server.count());
    }
}
