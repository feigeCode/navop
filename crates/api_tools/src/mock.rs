//! Mock 规则模型与纯匹配引擎。
//!
//! 基础版本只负责规则持久化、编译和匹配，不在此模块启动 HTTP 服务。

use std::collections::HashMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::http::{KeyValue, RequestMethod};
use crate::request_store::StoredRequest;

/// Mock 路径的匹配方式。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PathPattern {
    /// 完整路径必须相等。
    Exact(String),
    /// 请求路径必须以给定值开头。
    Prefix(String),
    /// 使用自动锚定到完整路径的正则表达式。
    Regex(String),
}

impl Default for PathPattern {
    fn default() -> Self {
        Self::Exact(String::new())
    }
}

impl PathPattern {
    pub fn value(&self) -> &str {
        match self {
            Self::Exact(value) | Self::Prefix(value) | Self::Regex(value) => value,
        }
    }
}

/// 一条可持久化的 Mock 响应规则。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MockRule {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_status")]
    pub status: u16,
    #[serde(default)]
    pub headers: Vec<KeyValue>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub delay_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_method: Option<RequestMethod>,
    #[serde(default)]
    pub match_path: PathPattern,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub match_query: Vec<KeyValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub match_headers: Vec<KeyValue>,
    #[serde(default)]
    pub enable_templates: bool,
}

fn default_true() -> bool {
    true
}

fn default_status() -> u16 {
    200
}

impl Default for MockRule {
    fn default() -> Self {
        Self {
            enabled: true,
            status: 200,
            headers: vec![KeyValue::new("Content-Type", "application/json")],
            body: "{}".to_string(),
            delay_ms: 0,
            match_method: None,
            match_path: PathPattern::Exact(String::new()),
            match_query: Vec::new(),
            match_headers: Vec::new(),
            enable_templates: false,
        }
    }
}

/// 将不同传输层的请求表示适配为 Mock 匹配输入。
pub trait MockRequestLike {
    fn method(&self) -> &str;
    fn path(&self) -> &str;
    fn query(&self) -> &HashMap<String, String>;
    fn headers(&self) -> &HashMap<String, String>;
}

/// 编译后的单条规则。
#[derive(Debug, Clone)]
pub struct CompiledMockRule {
    pub request_id: String,
    pub request_name: String,
    pub rule: MockRule,
    priority: u8,
    pattern: String,
    regex: Option<Regex>,
}

impl CompiledMockRule {
    pub fn matches(&self, request: &impl MockRequestLike) -> bool {
        if let Some(method) = self.rule.match_method
            && !request.method().eq_ignore_ascii_case(method.label())
        {
            return false;
        }

        let path_matches = match &self.rule.match_path {
            PathPattern::Exact(_) => request.path() == self.pattern,
            PathPattern::Prefix(_) => request.path().starts_with(&self.pattern),
            PathPattern::Regex(_) => self
                .regex
                .as_ref()
                .is_some_and(|regex| regex.is_match(request.path())),
        };
        path_matches
            && conditions_match(&self.rule.match_query, request.query(), false)
            && conditions_match(&self.rule.match_headers, request.headers(), true)
    }
}

/// 已按 Exact → Prefix → Regex 稳定排序的规则集合。
#[derive(Debug, Clone, Default)]
pub struct MockRuleSet {
    entries: Vec<CompiledMockRule>,
}

impl MockRuleSet {
    /// 从持久化请求构建规则集合。禁用规则与非法正则会被跳过。
    pub fn from_requests(requests: &[StoredRequest]) -> Self {
        let mut entries = requests
            .iter()
            .filter_map(compile_request_rule)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.priority);
        Self { entries }
    }

    pub fn entries(&self) -> &[CompiledMockRule] {
        &self.entries
    }

    /// 返回优先级最高且声明顺序最早的匹配规则。
    pub fn find_match(&self, request: &impl MockRequestLike) -> Option<&CompiledMockRule> {
        self.entries.iter().find(|entry| entry.matches(request))
    }
}

fn compile_request_rule(request: &StoredRequest) -> Option<CompiledMockRule> {
    let mut rule = request.mock.clone()?;
    if !rule.enabled {
        return None;
    }

    if matches!(&rule.match_path, PathPattern::Exact(path) if path.is_empty())
        && let Some(path) = path_of(&request.url)
    {
        rule.match_path = PathPattern::Exact(path);
    }

    let (priority, pattern, regex) = match &rule.match_path {
        PathPattern::Exact(pattern) => (0, pattern.clone(), None),
        PathPattern::Prefix(pattern) => (1, pattern.clone(), None),
        PathPattern::Regex(pattern) => {
            let regex = Regex::new(&format!("^(?:{pattern})$")).ok()?;
            (2, pattern.clone(), Some(regex))
        }
    };

    Some(CompiledMockRule {
        request_id: request.id.clone(),
        request_name: request.name.clone(),
        rule,
        priority,
        pattern,
        regex,
    })
}

fn conditions_match(
    conditions: &[KeyValue],
    values: &HashMap<String, String>,
    case_insensitive_keys: bool,
) -> bool {
    conditions
        .iter()
        .filter(|condition| condition.enabled && !condition.key.trim().is_empty())
        .all(|condition| {
            let key = condition.key.trim();
            let value = if case_insensitive_keys {
                values.iter().find_map(|(candidate, value)| {
                    candidate.eq_ignore_ascii_case(key).then_some(value)
                })
            } else {
                values.get(key)
            };
            value.is_some_and(|value| condition.value.is_empty() || value == &condition.value)
        })
}

/// 从绝对 URL、相对 URL 或 `{{baseUrl}}/path` 中提取路径。
pub fn path_of(url: &str) -> Option<String> {
    let mut value = url.trim();
    if value.starts_with("{{") {
        if let Some(end) = value.find("}}") {
            value = &value[end + 2..];
        }
    }

    if let Ok(parsed) = url::Url::parse(value)
        && parsed.has_host()
    {
        return Some(parsed.path().to_string());
    }

    if let Some(scheme) = value.find("://") {
        let remainder = &value[scheme + 3..];
        return Some(
            remainder
                .find('/')
                .map(|index| &remainder[index..])
                .unwrap_or("/")
                .split(['?', '#'])
                .next()
                .unwrap_or("/")
                .to_string(),
        );
    }

    let path = value.split(['?', '#']).next().unwrap_or(value);
    (!path.is_empty()).then(|| path.to_string())
}

/// 对 Mock 查询条件和路径片段做最小 URL 解码。
pub fn url_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let Ok(decoded) = u8::from_str_radix(
                std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or_default(),
                16,
            )
        {
            output.push(decoded);
            index += 3;
            continue;
        }
        output.push(if bytes[index] == b'+' {
            b' '
        } else {
            bytes[index]
        });
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestRequest {
        method: String,
        path: String,
        query: HashMap<String, String>,
        headers: HashMap<String, String>,
    }

    impl TestRequest {
        fn new(method: &str, path: &str) -> Self {
            Self {
                method: method.into(),
                path: path.into(),
                query: HashMap::new(),
                headers: HashMap::new(),
            }
        }
    }

    impl MockRequestLike for TestRequest {
        fn method(&self) -> &str {
            &self.method
        }

        fn path(&self) -> &str {
            &self.path
        }

        fn query(&self) -> &HashMap<String, String> {
            &self.query
        }

        fn headers(&self) -> &HashMap<String, String> {
            &self.headers
        }
    }

    fn stored(name: &str, url: &str, pattern: PathPattern) -> StoredRequest {
        let mut request = StoredRequest::new(name, RequestMethod::Get);
        request.url = url.into();
        request.mock = Some(MockRule {
            match_path: pattern,
            ..Default::default()
        });
        request
    }

    #[test]
    fn default_rule_matches_verve_defaults() {
        let rule = MockRule::default();
        assert!(rule.enabled);
        assert_eq!(rule.status, 200);
        assert_eq!(
            rule.headers[0],
            KeyValue::new("Content-Type", "application/json")
        );
        assert_eq!(rule.body, "{}");
        assert_eq!(rule.match_path, PathPattern::Exact(String::new()));
    }

    #[test]
    fn exact_prefix_and_regex_follow_priority_and_stable_order() {
        let requests = vec![
            stored("regex", "", PathPattern::Regex("/users/.*".into())),
            stored("prefix-first", "", PathPattern::Prefix("/users".into())),
            stored("prefix-second", "", PathPattern::Prefix("/users/42".into())),
            stored("exact", "", PathPattern::Exact("/users/42".into())),
        ];
        let rules = MockRuleSet::from_requests(&requests);
        let request = TestRequest::new("GET", "/users/42");

        assert_eq!(rules.find_match(&request).unwrap().request_name, "exact");
        assert_eq!(
            rules
                .entries()
                .iter()
                .map(|entry| entry.request_name.as_str())
                .collect::<Vec<_>>(),
            vec!["exact", "prefix-first", "prefix-second", "regex"]
        );
    }

    #[test]
    fn invalid_regex_and_disabled_rules_are_skipped() {
        let invalid = stored("invalid", "", PathPattern::Regex("(".into()));
        let mut disabled = stored("disabled", "", PathPattern::Exact("/".into()));
        disabled.mock.as_mut().unwrap().enabled = false;

        assert!(
            MockRuleSet::from_requests(&[invalid, disabled])
                .entries()
                .is_empty()
        );
    }

    #[test]
    fn matches_method_query_and_case_insensitive_headers() {
        let mut stored = stored("conditional", "", PathPattern::Exact("/users".into()));
        let rule = stored.mock.as_mut().unwrap();
        rule.match_method = Some(RequestMethod::Post);
        rule.match_query = vec![
            KeyValue::new("tenant", "navop"),
            KeyValue::new("present", ""),
        ];
        rule.match_headers = vec![KeyValue::new("X-Token", "secret")];
        let rules = MockRuleSet::from_requests(&[stored]);
        let mut request = TestRequest::new("post", "/users");
        request.query.insert("tenant".into(), "navop".into());
        request.query.insert("present".into(), "anything".into());
        request.headers.insert("x-token".into(), "secret".into());

        assert!(rules.find_match(&request).is_some());
        request.query.insert("tenant".into(), "other".into());
        assert!(rules.find_match(&request).is_none());
    }

    #[test]
    fn disabled_and_empty_conditions_are_ignored() {
        let mut stored = stored("conditional", "", PathPattern::Exact("/users".into()));
        let mut disabled = KeyValue::new("missing", "value");
        disabled.enabled = false;
        stored.mock.as_mut().unwrap().match_query = vec![disabled, KeyValue::new("", "ignored")];

        let rules = MockRuleSet::from_requests(&[stored]);
        assert!(
            rules
                .find_match(&TestRequest::new("GET", "/users"))
                .is_some()
        );
    }

    #[test]
    fn empty_exact_path_is_derived_from_request_url() {
        let request = stored(
            "derived",
            "https://example.test/users/42?verbose=true",
            PathPattern::Exact(String::new()),
        );
        let rules = MockRuleSet::from_requests(&[request]);

        assert_eq!(
            rules.entries()[0].rule.match_path,
            PathPattern::Exact("/users/42".into())
        );
        assert!(
            rules
                .find_match(&TestRequest::new("GET", "/users/42"))
                .is_some()
        );
    }

    #[test]
    fn path_extraction_and_url_decode_cover_common_inputs() {
        assert_eq!(path_of("https://example.test"), Some("/".into()));
        assert_eq!(path_of("{{baseUrl}}/users?q=1"), Some("/users".into()));
        assert_eq!(path_of("/relative?q=1#fragment"), Some("/relative".into()));
        assert_eq!(url_decode("hello+world%2F42"), "hello world/42");
    }
}
