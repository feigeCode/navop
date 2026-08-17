//! 接口测试数据的本地持久化（JSON 文件，存于应用配置目录）。
//!
//! 文件格式兼容旧版：
//! - 旧版文件是 `StoredRequest` 数组；
//! - 新版文件是包含 `folders` 与 `requests` 的对象。
//! 读取时两种格式都会尝试；保存时统一写新版对象。

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::http::{AuthConfig, BodyType, HttpResponse, KeyValue, RawLanguage, RequestMethod};
use crate::mock::MockRule;
use crate::protocol::Protocol;

const MAX_FAILURE_EXAMPLES: usize = 20;
const SCRIPT_OUTPUT_MARKERS: [&str; 2] =
    ["\n\n// ── Script Output ──", "\n\n// ── 预执行脚本输出 ──"];

/// 请求目录（侧边栏树的目录节点）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredFolder {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    /// 目录说明，仅用于帮助维护接口集合。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// 可选的目录级 Base URL；子请求使用相对 URL 时自动继承。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// 目录级查询参数，按根目录到当前目录的顺序覆盖。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<KeyValue>,
    /// 目录级请求头，按根目录到当前目录的顺序覆盖。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<KeyValue>,
    /// 目录级变量，按根目录到当前目录的顺序覆盖。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<KeyValue>,
}

impl StoredFolder {
    pub fn new(name: impl Into<String>, parent_id: Option<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().simple().to_string(),
            name: name.into(),
            parent_id,
            description: String::new(),
            base_url: None,
            params: Vec::new(),
            headers: Vec::new(),
            variables: Vec::new(),
        }
    }
}

fn deserialize_present_optional<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// 一套可持久化的接口测试环境配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEnvironment {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<KeyValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<KeyValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cookies: Vec<KeyValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<KeyValue>,
}

impl ApiEnvironment {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().simple().to_string(),
            name: name.into(),
            base_url: None,
            params: Vec::new(),
            headers: Vec::new(),
            cookies: Vec::new(),
            variables: Vec::new(),
        }
    }

    /// 把旧版环境变量中的 `baseUrl` 迁移到专用 Base URL 配置。
    pub(crate) fn migrate_base_url_variable(&mut self) {
        let variable_base_url = self
            .variables
            .iter()
            .rev()
            .find(|row| row.enabled && row.key.trim() == "baseUrl")
            .map(|row| row.value.trim())
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let configured_base_url = self
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        self.base_url = configured_base_url.or(variable_base_url);
        if self.base_url.is_some() {
            self.variables.retain(|row| row.key.trim() != "baseUrl");
        }
    }
}

/// 一条从真实请求响应中自动保存的只读示例。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResponseExample {
    pub status: u16,
    #[serde(default)]
    pub status_text: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub saved_at: String,
}

impl ResponseExample {
    fn from_response(response: &HttpResponse) -> Self {
        let body_end = SCRIPT_OUTPUT_MARKERS
            .iter()
            .filter_map(|marker| response.body.find(marker))
            .min()
            .unwrap_or(response.body.len());

        Self {
            status: response.status,
            status_text: response.status_text.clone(),
            body: response.body[..body_end].to_string(),
            saved_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        }
    }

    fn matches_response(&self, response: &HttpResponse) -> bool {
        let candidate = Self::from_response(response);
        self.status == candidate.status && self.body == candidate.body
    }
}

/// 请求完成后自动保存哪些响应示例。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseExampleAutoSaveMode {
    Off,
    SaveSuccess,
    SaveFailure,
    #[default]
    SaveBoth,
}

impl ResponseExampleAutoSaveMode {
    fn saves_success(self) -> bool {
        matches!(self, Self::SaveSuccess | Self::SaveBoth)
    }

    fn saves_failure(self) -> bool {
        matches!(self, Self::SaveFailure | Self::SaveBoth)
    }
}

/// 一条持久化的测试请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRequest {
    pub id: String,
    pub name: String,
    /// 请求用途、约束和维护说明。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub method: RequestMethod,
    #[serde(default)]
    pub protocol: Protocol,
    pub url: String,
    #[serde(default)]
    pub folder_id: Option<String>,
    /// 请求级 Base URL 覆盖：
    /// `None` 继承目录，`Some(None)` 显式禁用，`Some(Some(url))` 使用覆盖值。
    #[serde(
        default,
        deserialize_with = "deserialize_present_optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub base_url_override: Option<Option<String>>,
    /// 请求头文本（每行 `Key: Value`），与 `header_rows` 双向同步，兼容旧文件。
    #[serde(default, alias = "headers")]
    pub headers: String,
    /// URL 查询参数。
    #[serde(default)]
    pub params: Vec<KeyValue>,
    /// URL 路径变量（`{{key}}` → value）。
    #[serde(default)]
    pub path_vars: Vec<KeyValue>,
    /// 当前请求私有变量，优先级高于全局与环境变量。
    #[serde(default)]
    pub variables: Vec<KeyValue>,
    /// 请求头键值行。
    #[serde(default)]
    pub header_rows: Vec<KeyValue>,
    /// Cookie 键值行。
    #[serde(default)]
    pub cookies: Vec<KeyValue>,
    /// Raw 请求体文本。
    #[serde(default, alias = "body")]
    pub body: String,
    /// 请求体类型。
    #[serde(default)]
    pub body_type: BodyType,
    /// Raw 请求体语言。
    #[serde(default)]
    pub raw_language: RawLanguage,
    /// form-data / x-www-form-urlencoded 请求体键值行。
    #[serde(default)]
    pub body_rows: Vec<KeyValue>,
    /// 鉴权配置。
    #[serde(default)]
    pub auth: AuthConfig,
    /// 预执行脚本（JavaScript）。
    #[serde(default)]
    pub pre_script: String,
    /// Tests 测试脚本（JavaScript）。
    #[serde(default)]
    pub tests: String,
    /// 可选的本地 Mock 匹配与响应规则。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mock: Option<MockRule>,
    /// 最近一次完成的完整响应，用于重新打开请求时恢复结果面板。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_response: Option<HttpResponse>,
    /// 最近一次成功响应示例。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_example: Option<ResponseExample>,
    /// 最近的失败响应示例，最新一条位于最前。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fail_examples: Vec<ResponseExample>,
}

impl StoredRequest {
    pub fn new(name: impl Into<String>, method: RequestMethod) -> Self {
        Self {
            id: uuid::Uuid::new_v4().simple().to_string(),
            name: name.into(),
            description: String::new(),
            method,
            protocol: Protocol::Http,
            url: String::new(),
            folder_id: None,
            base_url_override: None,
            headers: String::new(),
            params: Vec::new(),
            path_vars: Vec::new(),
            variables: Vec::new(),
            header_rows: Vec::new(),
            cookies: Vec::new(),
            body: String::new(),
            body_type: BodyType::None,
            raw_language: RawLanguage::Json,
            body_rows: Vec::new(),
            auth: AuthConfig::default(),
            pre_script: String::new(),
            tests: String::new(),
            mock: None,
            last_response: None,
            success_example: None,
            fail_examples: Vec::new(),
        }
    }
}

/// 根据配置把一次非流式响应保存到请求的成功或失败示例中。
pub fn apply_response_example_autosave(
    request: &mut StoredRequest,
    response: &HttpResponse,
    mode: ResponseExampleAutoSaveMode,
) {
    if response.streaming || mode == ResponseExampleAutoSaveMode::Off {
        return;
    }

    let succeeded = response.error.is_none() && (200..400).contains(&response.status);
    if succeeded {
        if mode.saves_success() {
            request.success_example = Some(ResponseExample::from_response(response));
        }
        return;
    }

    if !mode.saves_failure()
        || request
            .fail_examples
            .iter()
            .any(|example| example.matches_response(response))
    {
        return;
    }

    request
        .fail_examples
        .insert(0, ResponseExample::from_response(response));
    request.fail_examples.truncate(MAX_FAILURE_EXAMPLES);
}

/// 一次发送操作的完整历史快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestHistoryEntry {
    pub id: String,
    pub sent_at: i64,
    #[serde(default)]
    pub request_id: Option<String>,
    pub request_name: String,
    pub method: RequestMethod,
    pub url: String,
    #[serde(default)]
    pub status: u16,
    #[serde(default)]
    pub status_text: String,
    #[serde(default)]
    pub time_ms: u64,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub error: Option<String>,
    pub request: StoredRequest,
}

/// 新版持久化对象。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiStore {
    #[serde(default)]
    pub folders: Vec<StoredFolder>,
    #[serde(default)]
    pub requests: Vec<StoredRequest>,
    #[serde(default)]
    pub globals: Vec<KeyValue>,
    /// 自动附加到所有请求的查询参数。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_params: Vec<KeyValue>,
    /// 自动附加到所有请求的请求头。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_headers: Vec<KeyValue>,
    /// 自动合并到所有请求的 Cookies。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_cookies: Vec<KeyValue>,
    #[serde(default)]
    pub environments: Vec<ApiEnvironment>,
    #[serde(default)]
    pub active_environment_id: Option<String>,
    #[serde(default)]
    pub history: Vec<RequestHistoryEntry>,
    #[serde(default)]
    pub response_example_autosave: ResponseExampleAutoSaveMode,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum StoreFile {
    V2(ApiStore),
    Legacy(Vec<StoredRequest>),
}

/// 默认存储路径：`<config_dir>/api_tools/requests.json`。
fn default_store_path() -> Option<PathBuf> {
    one_core::storage::manager::get_config_dir()
        .ok()
        .map(|dir| dir.join("api_tools").join("requests.json"))
}

/// 从默认路径加载完整数据；文件不存在或解析失败时返回空数据。
pub fn load_store() -> ApiStore {
    default_store_path()
        .and_then(|path| load_store_from(&path).ok())
        .unwrap_or_default()
}

/// 保存完整数据到默认路径（尽力而为，失败仅记录日志）。
pub fn save_store(store: &ApiStore) {
    if let Some(path) = default_store_path() {
        save_store_to(store, &path).unwrap_or_else(|e| {
            tracing::warn!("api_tools: save store failed: {e}");
        });
    }
}

/// 从默认路径加载请求列表（兼容旧版数组）。
#[allow(dead_code)]
pub fn load_requests() -> Vec<StoredRequest> {
    load_store().requests
}

/// 保存请求列表到默认路径（兼容旧调用方，不保存目录）。
#[allow(dead_code)]
pub fn save_requests(requests: &[StoredRequest]) {
    save_store(&ApiStore {
        folders: Vec::new(),
        requests: requests.to_vec(),
        ..ApiStore::default()
    });
}

/// 从指定路径加载完整数据。
pub fn load_store_from(path: &Path) -> Result<ApiStore> {
    let text = std::fs::read_to_string(path)?;
    let mut store = match serde_json::from_str::<StoreFile>(&text)? {
        StoreFile::V2(store) => store,
        StoreFile::Legacy(requests) => ApiStore {
            folders: Vec::new(),
            requests,
            ..ApiStore::default()
        },
    };
    for environment in &mut store.environments {
        environment.migrate_base_url_variable();
    }
    Ok(store)
}

/// 保存完整数据到指定路径（自动创建父目录）。
pub fn save_store_to(store: &ApiStore, path: &Path) -> Result<()> {
    write_json(store, path)
}

/// 从指定路径加载请求列表（兼容旧版数组）。
#[allow(dead_code)]
pub fn load_requests_from(path: &Path) -> Result<Vec<StoredRequest>> {
    Ok(load_store_from(path)?.requests)
}

/// 以旧版数组格式保存请求列表（供兼容测试与旧调用方使用）。
#[allow(dead_code)]
pub fn save_requests_to(requests: &[StoredRequest], path: &Path) -> Result<()> {
    write_json(requests, path)
}

fn write_json(value: &(impl Serialize + ?Sized), path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(value)?;
    std::fs::write(path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::PathPattern;
    use crate::protocol::Protocol;

    fn response(status: u16, body: impl Into<String>) -> HttpResponse {
        HttpResponse {
            status,
            status_text: if status < 400 {
                "OK".into()
            } else {
                "Failed".into()
            },
            body: body.into(),
            ..Default::default()
        }
    }

    #[test]
    fn round_trip_preserves_requests() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.json");
        let mut req = StoredRequest::new("登录", RequestMethod::Post);
        req.url = "https://api.example.com/login".to_string();
        req.tests = "apt.assert(response.status === 200)".to_string();
        req.variables = vec![KeyValue::new("token", "request-token")];

        save_requests_to(&[req.clone()], &path).unwrap();
        let loaded = load_requests_from(&path).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, req.id);
        assert_eq!(loaded[0].name, "登录");
        assert_eq!(loaded[0].method, RequestMethod::Post);
        assert_eq!(loaded[0].url, req.url);
        assert_eq!(loaded[0].tests, req.tests);
        assert_eq!(loaded[0].variables[0].value, "request-token");
    }

    #[test]
    fn request_description_mock_and_last_response_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.json");
        let mut request = StoredRequest::new("带 Mock 的请求", RequestMethod::Post);
        request.description = "用于登录失败场景".into();
        request.mock = Some(MockRule {
            status: 401,
            body: r#"{"error":"unauthorized"}"#.into(),
            match_method: Some(RequestMethod::Post),
            match_path: PathPattern::Exact("/login".into()),
            match_headers: vec![KeyValue::new("X-Tenant", "navop")],
            ..MockRule::default()
        });
        request.last_response = Some(HttpResponse {
            status: 401,
            status_text: "Unauthorized".into(),
            time_ms: 28,
            size: 24,
            headers: vec![KeyValue::new("Content-Type", "application/json")],
            raw_body: r#"{"error":"unauthorized"}"#.into(),
            body: r#"{"error":"unauthorized"}"#.into(),
            is_json: true,
            error: Some("assertion failed".into()),
            ..HttpResponse::default()
        });

        save_requests_to(&[request.clone()], &path).unwrap();
        let loaded = load_requests_from(&path).unwrap();

        assert_eq!(loaded[0].description, request.description);
        assert_eq!(loaded[0].mock, request.mock);
        assert_eq!(loaded[0].last_response, request.last_response);
    }

    #[test]
    fn http_response_serde_round_trip_preserves_complete_snapshot() {
        let response = HttpResponse {
            status: 200,
            status_text: "OK".into(),
            time_ms: 17,
            size: 12,
            headers: vec![KeyValue::new("X-Trace", "abc")],
            raw_body: "{\"ok\":true}".into(),
            body: "{\n  \"ok\": true\n}".into(),
            is_json: true,
            streaming: false,
            error: None,
        };

        let json = serde_json::to_string(&response).unwrap();
        let loaded: HttpResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded, response);
    }

    #[test]
    fn request_protocol_round_trips_in_lowercase() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.json");
        let mut request = StoredRequest::new("事件流", RequestMethod::Get);
        request.protocol = Protocol::Sse;

        save_requests_to(&[request], &path).unwrap();
        let json = std::fs::read_to_string(&path).unwrap();
        let loaded = load_requests_from(&path).unwrap();

        assert!(json.contains(r#""protocol": "sse""#));
        assert_eq!(loaded[0].protocol, Protocol::Sse);
    }

    #[test]
    fn store_round_trip_preserves_folders() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.json");
        let mut folder = StoredFolder::new("用户模块", None);
        folder.description = "用户认证与资料接口".into();
        folder.base_url = Some("https://{{host}}/v1".into());
        folder.params = vec![KeyValue::new("locale", "zh-CN")];
        folder.headers = vec![KeyValue::new("X-Client", "navop")];
        folder.variables = vec![KeyValue::new("host", "api.example.test")];
        let mut req = StoredRequest::new("登录", RequestMethod::Post);
        req.folder_id = Some(folder.id.clone());
        let store = ApiStore {
            folders: vec![folder.clone()],
            requests: vec![req],
            ..ApiStore::default()
        };

        save_store_to(&store, &path).unwrap();
        let loaded = load_store_from(&path).unwrap();

        assert_eq!(loaded.folders.len(), 1);
        assert_eq!(loaded.folders[0].name, folder.name);
        assert_eq!(loaded.folders[0].description, folder.description);
        assert_eq!(loaded.folders[0].base_url, folder.base_url);
        assert_eq!(loaded.folders[0].params, folder.params);
        assert_eq!(loaded.folders[0].headers, folder.headers);
        assert_eq!(loaded.folders[0].variables, folder.variables);
        assert_eq!(
            loaded.requests[0].folder_id.as_deref(),
            Some(folder.id.as_str())
        );
    }

    #[test]
    fn old_folder_json_defaults_new_inherited_fields() {
        let folder: StoredFolder = serde_json::from_value(serde_json::json!({
            "id": "legacy-folder",
            "name": "Legacy",
            "parent_id": null,
            "base_url": "https://example.test",
            "variables": []
        }))
        .unwrap();

        assert!(folder.description.is_empty());
        assert!(folder.params.is_empty());
        assert!(folder.headers.is_empty());
    }

    #[test]
    fn request_base_url_override_preserves_missing_null_and_string_states() {
        let request = StoredRequest::new("请求", RequestMethod::Get);
        let value = serde_json::to_value(&request).unwrap();

        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove("base_url_override");
        assert_eq!(
            serde_json::from_value::<StoredRequest>(missing)
                .unwrap()
                .base_url_override,
            None
        );

        let mut explicit_null = value.clone();
        explicit_null
            .as_object_mut()
            .unwrap()
            .insert("base_url_override".into(), serde_json::Value::Null);
        assert_eq!(
            serde_json::from_value::<StoredRequest>(explicit_null)
                .unwrap()
                .base_url_override,
            Some(None)
        );

        let mut override_url = value;
        override_url.as_object_mut().unwrap().insert(
            "base_url_override".into(),
            serde_json::Value::String("https://override.test".into()),
        );
        let loaded = serde_json::from_value::<StoredRequest>(override_url).unwrap();
        assert_eq!(
            loaded.base_url_override,
            Some(Some("https://override.test".into()))
        );
        assert_eq!(
            serde_json::to_value(&loaded).unwrap()["base_url_override"],
            serde_json::Value::String("https://override.test".into())
        );

        let explicit_null = StoredRequest {
            base_url_override: Some(None),
            ..StoredRequest::new("请求", RequestMethod::Get)
        };
        assert_eq!(
            serde_json::to_value(explicit_null).unwrap()["base_url_override"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn store_round_trip_preserves_environments_and_globals() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.json");
        let mut environment = ApiEnvironment::new("开发环境");
        environment.base_url = Some("https://dev.example.test".into());
        environment.params = vec![KeyValue::new("debug", "true")];
        environment.headers = vec![KeyValue::new("X-Environment", "development")];
        environment.cookies = vec![KeyValue::new("region", "cn")];
        environment.variables = vec![KeyValue::new("tenant_id", "navop")];
        let store = ApiStore {
            globals: vec![KeyValue::new("tenant", "navop")],
            global_params: vec![KeyValue::new("locale", "zh-CN")],
            global_headers: vec![KeyValue::new("X-Client", "navop")],
            global_cookies: vec![KeyValue::new("session", "token")],
            active_environment_id: Some(environment.id.clone()),
            environments: vec![environment.clone()],
            ..ApiStore::default()
        };

        save_store_to(&store, &path).unwrap();
        let loaded = load_store_from(&path).unwrap();

        assert_eq!(loaded.globals[0].value, "navop");
        assert_eq!(loaded.global_params[0].value, "zh-CN");
        assert_eq!(loaded.global_headers[0].value, "navop");
        assert_eq!(loaded.global_cookies[0].value, "token");
        assert_eq!(
            loaded.active_environment_id.as_deref(),
            Some(environment.id.as_str())
        );
        assert_eq!(loaded.environments[0].name, "开发环境");
        assert_eq!(
            loaded.environments[0].base_url.as_deref(),
            Some("https://dev.example.test")
        );
        assert_eq!(loaded.environments[0].params[0].value, "true");
        assert_eq!(loaded.environments[0].headers[0].value, "development");
        assert_eq!(loaded.environments[0].cookies[0].value, "cn");
        assert_eq!(loaded.environments[0].variables[0].value, "navop");
    }

    #[test]
    fn legacy_environment_without_scoped_settings_is_still_loadable() {
        let environment: ApiEnvironment = serde_json::from_str(
            r#"{
                "id": "legacy-env",
                "name": "旧环境",
                "variables": [
                    {"key": "token", "value": "legacy", "enabled": true}
                ]
            }"#,
        )
        .unwrap();

        assert_eq!(environment.id, "legacy-env");
        assert_eq!(environment.name, "旧环境");
        assert_eq!(environment.base_url, None);
        assert!(environment.params.is_empty());
        assert!(environment.headers.is_empty());
        assert!(environment.cookies.is_empty());
        assert_eq!(environment.variables[0].value, "legacy");
    }

    #[test]
    fn loading_store_migrates_legacy_base_url_variable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.json");
        std::fs::write(
            &path,
            r#"{
                "environments": [{
                    "id": "legacy-env",
                    "name": "旧环境",
                    "variables": [
                        {"key": "baseUrl", "value": "https://legacy.example.test", "enabled": true},
                        {"key": "token", "value": "legacy-token", "enabled": true}
                    ]
                }],
                "active_environment_id": "legacy-env"
            }"#,
        )
        .unwrap();

        let store = load_store_from(&path).unwrap();
        let environment = &store.environments[0];

        assert_eq!(
            environment.base_url.as_deref(),
            Some("https://legacy.example.test")
        );
        assert_eq!(
            environment.variables,
            vec![KeyValue::new("token", "legacy-token")]
        );
    }

    #[test]
    fn store_round_trip_preserves_request_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.json");
        let request = StoredRequest::new("历史请求", RequestMethod::Post);
        let entry = RequestHistoryEntry {
            id: "history-1".into(),
            sent_at: 123,
            request_id: Some(request.id.clone()),
            request_name: request.name.clone(),
            method: request.method,
            url: "https://example.test/history".into(),
            status: 500,
            status_text: "Server Error".into(),
            time_ms: 42,
            size: 8,
            error: Some("boom".into()),
            request,
        };
        let store = ApiStore {
            history: vec![entry],
            ..ApiStore::default()
        };

        save_store_to(&store, &path).unwrap();
        let loaded = load_store_from(&path).unwrap();

        assert_eq!(loaded.history.len(), 1);
        assert_eq!(loaded.history[0].id, "history-1");
        assert_eq!(loaded.history[0].request_name, "历史请求");
        assert_eq!(loaded.history[0].error.as_deref(), Some("boom"));
    }

    #[test]
    fn legacy_request_array_is_still_loadable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.json");
        std::fs::write(
            &path,
            r#"[{"id":"legacy-1","name":"旧请求","method":"Get","url":"https://example.com","headers":"X-Test: 1","body":"{}","tests":"apt.assert(true)"}]"#,
        )
        .unwrap();

        let loaded = load_store_from(&path).unwrap();
        assert_eq!(loaded.requests.len(), 1);
        assert_eq!(loaded.requests[0].id, "legacy-1");
        assert_eq!(loaded.requests[0].headers, "X-Test: 1");
        assert_eq!(loaded.requests[0].body, "{}");
        assert_eq!(loaded.requests[0].body_type, BodyType::None);
        assert_eq!(loaded.requests[0].protocol, Protocol::Http);
        assert!(loaded.requests[0].variables.is_empty());
        assert!(loaded.requests[0].header_rows.is_empty());
        assert!(loaded.requests[0].cookies.is_empty());
        assert!(loaded.requests[0].body_rows.is_empty());
        assert_eq!(loaded.requests[0].auth, AuthConfig::default());
        assert!(loaded.requests[0].description.is_empty());
        assert!(loaded.requests[0].mock.is_none());
        assert!(loaded.requests[0].last_response.is_none());
        assert!(loaded.requests[0].success_example.is_none());
        assert!(loaded.requests[0].fail_examples.is_empty());
        assert!(loaded.global_params.is_empty());
        assert!(loaded.global_headers.is_empty());
        assert!(loaded.global_cookies.is_empty());
        assert_eq!(
            loaded.response_example_autosave,
            ResponseExampleAutoSaveMode::SaveBoth
        );
    }

    #[test]
    fn legacy_array_can_be_saved_as_v2_and_loaded_again() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join("legacy.json");
        let v2_path = dir.path().join("v2.json");
        std::fs::write(
            &legacy_path,
            r#"[{"id":"legacy-1","name":"旧请求","method":"Get","url":"https://example.com"}]"#,
        )
        .unwrap();

        let store = load_store_from(&legacy_path).unwrap();
        save_store_to(&store, &v2_path).unwrap();
        let reloaded = load_store_from(&v2_path).unwrap();
        let saved = std::fs::read_to_string(&v2_path).unwrap();

        assert!(saved.trim_start().starts_with('{'));
        assert_eq!(reloaded.requests.len(), 1);
        assert_eq!(reloaded.requests[0].id, "legacy-1");
        assert!(reloaded.globals.is_empty());
        assert!(reloaded.environments.is_empty());
    }

    #[test]
    fn load_missing_file_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_requests_from(&dir.path().join("nope.json"));
        assert!(result.is_err());
    }

    #[test]
    fn load_corrupt_file_is_ignored_by_default_loader() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.json");
        std::fs::write(&path, "not json").unwrap();
        let result = load_requests_from(&path);
        assert!(result.is_err());
    }

    #[test]
    fn response_examples_and_autosave_mode_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.json");
        let mut request = StoredRequest::new("示例", RequestMethod::Get);
        request.success_example = Some(ResponseExample {
            status: 200,
            status_text: "OK".into(),
            body: r#"{"ok":true}"#.into(),
            saved_at: "2026-08-16 12:00:00".into(),
        });
        request.fail_examples = vec![ResponseExample {
            status: 500,
            status_text: "Server Error".into(),
            body: "boom".into(),
            saved_at: "2026-08-16 12:01:00".into(),
        }];
        let store = ApiStore {
            requests: vec![request],
            response_example_autosave: ResponseExampleAutoSaveMode::SaveFailure,
            ..ApiStore::default()
        };

        save_store_to(&store, &path).unwrap();
        let loaded = load_store_from(&path).unwrap();

        assert_eq!(
            loaded.response_example_autosave,
            ResponseExampleAutoSaveMode::SaveFailure
        );
        assert_eq!(
            loaded.requests[0].success_example.as_ref().unwrap().status,
            200
        );
        assert_eq!(loaded.requests[0].fail_examples[0].body, "boom");
    }

    #[test]
    fn successful_response_overwrites_success_example_and_accepts_redirects() {
        let mut request = StoredRequest::new("示例", RequestMethod::Get);
        apply_response_example_autosave(
            &mut request,
            &response(200, "first"),
            ResponseExampleAutoSaveMode::SaveBoth,
        );
        apply_response_example_autosave(
            &mut request,
            &response(302, "redirect"),
            ResponseExampleAutoSaveMode::SaveBoth,
        );

        let example = request.success_example.unwrap();
        assert_eq!(example.status, 302);
        assert_eq!(example.body, "redirect");
        assert!(request.fail_examples.is_empty());
    }

    #[test]
    fn errored_or_zero_status_response_is_saved_as_failure() {
        let mut request = StoredRequest::new("示例", RequestMethod::Get);
        let mut failed = response(200, "script failed");
        failed.error = Some("assertion failed".into());
        apply_response_example_autosave(
            &mut request,
            &failed,
            ResponseExampleAutoSaveMode::SaveBoth,
        );
        apply_response_example_autosave(
            &mut request,
            &response(0, "network failed"),
            ResponseExampleAutoSaveMode::SaveBoth,
        );

        assert!(request.success_example.is_none());
        assert_eq!(request.fail_examples.len(), 2);
        assert_eq!(request.fail_examples[0].status, 0);
        assert_eq!(request.fail_examples[1].body, "script failed");
    }

    #[test]
    fn failure_examples_are_deduplicated_newest_first_and_capped() {
        let mut request = StoredRequest::new("示例", RequestMethod::Get);
        apply_response_example_autosave(
            &mut request,
            &response(500, "duplicate"),
            ResponseExampleAutoSaveMode::SaveBoth,
        );
        apply_response_example_autosave(
            &mut request,
            &response(500, "duplicate"),
            ResponseExampleAutoSaveMode::SaveBoth,
        );
        for index in 0..25 {
            apply_response_example_autosave(
                &mut request,
                &response(500, format!("failure-{index}")),
                ResponseExampleAutoSaveMode::SaveBoth,
            );
        }

        assert_eq!(request.fail_examples.len(), MAX_FAILURE_EXAMPLES);
        assert_eq!(request.fail_examples[0].body, "failure-24");
        assert_eq!(request.fail_examples[19].body, "failure-5");
    }

    #[test]
    fn autosave_modes_route_examples_and_skip_streaming_responses() {
        let mut request = StoredRequest::new("示例", RequestMethod::Get);
        apply_response_example_autosave(
            &mut request,
            &response(200, "off"),
            ResponseExampleAutoSaveMode::Off,
        );
        apply_response_example_autosave(
            &mut request,
            &response(500, "success-only"),
            ResponseExampleAutoSaveMode::SaveSuccess,
        );
        apply_response_example_autosave(
            &mut request,
            &response(200, "failure-only"),
            ResponseExampleAutoSaveMode::SaveFailure,
        );
        let mut streaming = response(200, "streaming");
        streaming.streaming = true;
        apply_response_example_autosave(
            &mut request,
            &streaming,
            ResponseExampleAutoSaveMode::SaveBoth,
        );

        assert!(request.success_example.is_none());
        assert!(request.fail_examples.is_empty());
    }

    #[test]
    fn response_example_strips_appended_script_output() {
        let mut request = StoredRequest::new("示例", RequestMethod::Get);
        apply_response_example_autosave(
            &mut request,
            &response(
                200,
                "response body\n\n// ── Script Output ──\nconsole.log('done')",
            ),
            ResponseExampleAutoSaveMode::SaveBoth,
        );

        assert_eq!(
            request.success_example.as_ref().unwrap().body,
            "response body"
        );
    }
}
