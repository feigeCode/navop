//! ACP elicitation:agent 主动向用户提问，用户在聊天里作答。
//!
//! 与工具权限问答（`permission.rs`）的形状一致：agent 侧发起一个请求，运行期把它塞进
//! 一条通道交给视图渲染，视图拿到用户的选择后再把结果送回去。区别只在载荷——权限请求
//! 是「选一个选项」，elicitation 是「填一张表单」或「去一个 URL」。
//!
//! 这里刻意不把 SDK 的类型泄漏给 UI：`CreateElicitationRequest` 只在 [`acp_elicitation_request`]
//! 里被读一次，之后 UI 只看本模块自己的类型。

use agent_client_protocol::schema::v1::{
    CreateElicitationRequest, CreateElicitationResponse, ElicitationAcceptAction,
    ElicitationAction, ElicitationContentValue, ElicitationMode, ElicitationPropertySchema,
    ElicitationSchema, ElicitationScope, MultiSelectItems, StringPropertySchema,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// 填表单比「同意/拒绝」慢得多，超时给足，避免用户还在打字就被判取消。
const ELICITATION_TIMEOUT: Duration = Duration::from_secs(600);

pub type AcpElicitationFuture =
    Pin<Box<dyn Future<Output = AcpElicitationOutcome> + Send + 'static>>;
pub type AcpElicitationProvider =
    Arc<dyn Fn(AcpElicitationRequest) -> AcpElicitationFuture + Send + Sync + 'static>;

/// 一次提问：agent 的说明文字 + 取答案的方式。
#[derive(Clone, Debug, PartialEq)]
pub struct AcpElicitationRequest {
    pub request_id: String,
    pub session_id: String,
    pub message: String,
    pub mode: AcpElicitationMode,
}

impl AcpElicitationRequest {
    /// 表单模式下没有可渲染字段时，UI 只能给它一个「关闭」按钮。
    pub fn is_answerable(&self) -> bool {
        match &self.mode {
            AcpElicitationMode::Form(form) => {
                form.fields.iter().any(|field| field.kind.is_editable())
            }
            AcpElicitationMode::Url { .. } => true,
            AcpElicitationMode::Unsupported { .. } => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AcpElicitationMode {
    Form(AcpElicitationForm),
    /// 让用户去一个 URL 完成（例如 OAuth 授权页），完成后 agent 会再发完成通知。
    Url {
        url: String,
    },
    /// 协议未来新增的模式。不认识的模式不猜渲染，只允许跳过。
    Unsupported {
        mode: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AcpElicitationForm {
    pub title: Option<String>,
    pub description: Option<String>,
    pub fields: Vec<AcpElicitationField>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AcpElicitationField {
    pub name: String,
    pub title: String,
    pub description: Option<String>,
    pub required: bool,
    pub default: Option<Value>,
    pub kind: AcpElicitationFieldKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AcpElicitationFieldKind {
    Text {
        format: Option<String>,
    },
    Integer,
    Number,
    Boolean,
    SingleSelect {
        options: Vec<AcpElicitationOption>,
    },
    MultiSelect {
        options: Vec<AcpElicitationOption>,
    },
    /// 协议未来新增的属性类型：保留原始 type 名，不渲染成已知控件。
    Unsupported {
        type_name: String,
    },
}

impl AcpElicitationFieldKind {
    /// 有没有可以真正收集用户输入的控件。
    pub fn is_editable(&self) -> bool {
        !matches!(self, Self::Unsupported { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpElicitationOption {
    pub value: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AcpElicitationOutcome {
    /// 用户提交了表单内容（键为属性名）。
    Accept(BTreeMap<String, Value>),
    /// 用户明确拒绝回答。
    Decline,
    /// 请求被取消（超时、连接断开、用户关掉卡片）。
    Cancel,
}

/// 交给视图的消息：要么来了一条新提问，要么那条提问已经过期。
pub(crate) enum AcpElicitationMessage {
    Requested(AcpElicitationEnvelope),
    Expired { request_id: String },
}

pub(crate) struct AcpElicitationEnvelope {
    request: AcpElicitationRequest,
    response_tx: oneshot::Sender<AcpElicitationOutcome>,
}

impl AcpElicitationEnvelope {
    pub(crate) fn new(
        request: AcpElicitationRequest,
    ) -> (Self, oneshot::Receiver<AcpElicitationOutcome>) {
        let (response_tx, response_rx) = oneshot::channel();
        (
            Self {
                request,
                response_tx,
            },
            response_rx,
        )
    }

    pub(crate) fn request(&self) -> &AcpElicitationRequest {
        &self.request
    }

    /// 把用户的答案送回去。通道已关闭（视图已经放弃这张卡片）时返回 false。
    pub(crate) fn resolve(self, outcome: AcpElicitationOutcome) -> bool {
        self.response_tx.send(outcome).is_ok()
    }
}

pub(crate) fn acp_elicitation_channel() -> (
    AcpElicitationProvider,
    mpsc::UnboundedReceiver<AcpElicitationMessage>,
) {
    acp_elicitation_channel_with_timeout(ELICITATION_TIMEOUT)
}

fn acp_elicitation_channel_with_timeout(
    timeout: Duration,
) -> (
    AcpElicitationProvider,
    mpsc::UnboundedReceiver<AcpElicitationMessage>,
) {
    let (sender, receiver) = mpsc::unbounded_channel();
    // 多个连接可能同时挂着 elicitation，用通道 id 给 request_id 加前缀，避免串答案。
    let channel_id = uuid::Uuid::new_v4().to_string();
    let provider: AcpElicitationProvider = Arc::new(move |mut request| {
        let sender = sender.clone();
        request.request_id = format!("{channel_id}:{}", request.request_id);
        Box::pin(async move {
            let (envelope, response_rx) = AcpElicitationEnvelope::new(request);
            let request_id = envelope.request().request_id.clone();
            if sender
                .send(AcpElicitationMessage::Requested(envelope))
                .is_err()
            {
                return AcpElicitationOutcome::Cancel;
            }
            match tokio::time::timeout(timeout, response_rx).await {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(_)) => AcpElicitationOutcome::Cancel,
                Err(_) => {
                    let _ = sender.send(AcpElicitationMessage::Expired { request_id });
                    AcpElicitationOutcome::Cancel
                }
            }
        })
    });
    (provider, receiver)
}

/// 把通道里的答案翻回协议响应。没有 provider（视图没挂上）时直接取消。
pub(crate) async fn resolve_acp_elicitation_request(
    provider: Option<AcpElicitationProvider>,
    request: CreateElicitationRequest,
) -> CreateElicitationResponse {
    let Some(provider) = provider else {
        return cancelled_elicitation_response();
    };
    match provider(acp_elicitation_request(request)).await {
        AcpElicitationOutcome::Accept(content) => {
            let content = content
                .into_iter()
                .filter_map(|(name, value)| content_value(&value).map(|value| (name, value)))
                .collect::<BTreeMap<_, _>>();
            CreateElicitationResponse::new(ElicitationAction::Accept(
                ElicitationAcceptAction::new().content(content),
            ))
        }
        AcpElicitationOutcome::Decline => {
            CreateElicitationResponse::new(ElicitationAction::Decline)
        }
        AcpElicitationOutcome::Cancel => cancelled_elicitation_response(),
    }
}

fn cancelled_elicitation_response() -> CreateElicitationResponse {
    CreateElicitationResponse::new(ElicitationAction::Cancel)
}

/// 协议内容值只接受这五种标量/字符串数组，其余一律丢弃（宁可少填也不报错）。
fn content_value(value: &Value) -> Option<ElicitationContentValue> {
    match value {
        Value::String(text) => Some(ElicitationContentValue::String(text.clone())),
        Value::Bool(flag) => Some(ElicitationContentValue::Boolean(*flag)),
        Value::Number(number) => number
            .as_i64()
            .map(ElicitationContentValue::Integer)
            .or_else(|| number.as_f64().map(ElicitationContentValue::Number)),
        Value::Array(items) => items
            .iter()
            .map(|item| item.as_str().map(ToString::to_string))
            .collect::<Option<Vec<_>>>()
            .map(ElicitationContentValue::StringArray),
        _ => None,
    }
}

fn acp_elicitation_request(request: CreateElicitationRequest) -> AcpElicitationRequest {
    let message = request.message.clone();
    let (session_id, mode) = match &request.mode {
        ElicitationMode::Form(form) => (
            scope_session_id(&form.scope),
            AcpElicitationMode::Form(elicitation_form(&form.requested_schema)),
        ),
        ElicitationMode::Url(url) => (
            scope_session_id(&url.scope),
            AcpElicitationMode::Url {
                url: url.url.clone(),
            },
        ),
        other => (
            String::new(),
            AcpElicitationMode::Unsupported {
                mode: unsupported_mode_name(other),
            },
        ),
    };
    AcpElicitationRequest {
        request_id: elicitation_request_id(&session_id, &mode),
        session_id,
        message,
        mode,
    }
}

/// 提问没有协议自带的 id（它由 JSON-RPC 的请求 id 承载，SDK 不透出），
/// 因此用「会话 + 内容」拼一个稳定的键，供视图去重与回填答案。
fn elicitation_request_id(session_id: &str, mode: &AcpElicitationMode) -> String {
    match mode {
        AcpElicitationMode::Url { url } => format!("{session_id}:{url}"),
        AcpElicitationMode::Unsupported { mode } => format!("{session_id}:{mode}"),
        AcpElicitationMode::Form(form) => {
            let names = form
                .fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>()
                .join(",");
            format!("{session_id}:{names}")
        }
    }
}

fn unsupported_mode_name(mode: &ElicitationMode) -> String {
    match mode {
        ElicitationMode::Other(other) => other.mode.clone(),
        _ => "unknown".to_string(),
    }
}

fn scope_session_id(scope: &ElicitationScope) -> String {
    match scope {
        ElicitationScope::Session(session) => session.session_id.0.to_string(),
        // 请求级提问（会话还没建立，例如配置阶段）没有 session id，用请求 id 兜底，
        // 好让视图仍有一个稳定的键去重。
        ElicitationScope::Request(request) => request.request_id.to_string(),
        // 协议未来新增的作用域：不知道它属于谁，留空即可（request_id 仍能区分）。
        _ => String::new(),
    }
}

fn elicitation_form(schema: &ElicitationSchema) -> AcpElicitationForm {
    let required = schema
        .required
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    AcpElicitationForm {
        title: schema.title.clone(),
        description: schema.description.clone(),
        fields: schema
            .properties
            .iter()
            .map(|(name, property)| elicitation_field(name, property, &required))
            .collect(),
    }
}

fn elicitation_field(
    name: &str,
    property: &ElicitationPropertySchema,
    required: &BTreeSet<&str>,
) -> AcpElicitationField {
    let (title, description, default, kind) = match property {
        ElicitationPropertySchema::String(string) => (
            string.title.clone(),
            string.description.clone(),
            string.default.clone().map(Value::String),
            string_kind(string),
        ),
        ElicitationPropertySchema::Integer(integer) => (
            integer.title.clone(),
            integer.description.clone(),
            integer.default.map(Value::from),
            AcpElicitationFieldKind::Integer,
        ),
        ElicitationPropertySchema::Number(number) => (
            number.title.clone(),
            number.description.clone(),
            number.default.map(Value::from),
            AcpElicitationFieldKind::Number,
        ),
        ElicitationPropertySchema::Boolean(boolean) => (
            boolean.title.clone(),
            boolean.description.clone(),
            boolean.default.map(Value::from),
            AcpElicitationFieldKind::Boolean,
        ),
        ElicitationPropertySchema::Array(array) => (
            array.title.clone(),
            array.description.clone(),
            array
                .default
                .clone()
                .map(|values| Value::Array(values.into_iter().map(Value::String).collect())),
            AcpElicitationFieldKind::MultiSelect {
                options: multi_select_options(&array.items),
            },
        ),
        ElicitationPropertySchema::Other(other) => (
            None,
            None,
            None,
            AcpElicitationFieldKind::Unsupported {
                type_name: other.type_.clone(),
            },
        ),
        _ => (
            None,
            None,
            None,
            AcpElicitationFieldKind::Unsupported {
                type_name: "unknown".to_string(),
            },
        ),
    };
    AcpElicitationField {
        name: name.to_string(),
        title: title.unwrap_or_else(|| name.to_string()),
        description,
        required: required.contains(name),
        default,
        kind,
    }
}

/// 字符串属性在有 `enum` / `oneOf` 时是单选，否则是普通文本框。
fn string_kind(schema: &StringPropertySchema) -> AcpElicitationFieldKind {
    if let Some(options) = &schema.one_of {
        return AcpElicitationFieldKind::SingleSelect {
            options: options
                .iter()
                .map(|option| AcpElicitationOption {
                    value: option.value.clone(),
                    title: option.title.clone(),
                })
                .collect(),
        };
    }
    if let Some(values) = &schema.enum_values {
        return AcpElicitationFieldKind::SingleSelect {
            options: values
                .iter()
                .map(|value| AcpElicitationOption {
                    value: value.clone(),
                    title: value.clone(),
                })
                .collect(),
        };
    }
    AcpElicitationFieldKind::Text {
        format: schema.format.map(|format| {
            serde_json::to_value(format)
                .ok()
                .and_then(|value| value.as_str().map(ToString::to_string))
                .unwrap_or_else(|| "unknown".to_string())
        }),
    }
}

fn multi_select_options(items: &MultiSelectItems) -> Vec<AcpElicitationOption> {
    match items {
        MultiSelectItems::String(string) => string
            .values
            .iter()
            .map(|value| AcpElicitationOption {
                value: value.clone(),
                title: value.clone(),
            })
            .collect(),
        MultiSelectItems::Titled(titled) => titled
            .options
            .iter()
            .map(|option| AcpElicitationOption {
                value: option.value.clone(),
                title: option.title.clone(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AcpElicitationFieldKind, AcpElicitationMessage, AcpElicitationOutcome,
        acp_elicitation_channel, acp_elicitation_channel_with_timeout, acp_elicitation_request,
        content_value, resolve_acp_elicitation_request,
    };
    use agent_client_protocol::schema::v1::{
        BooleanPropertySchema, CreateElicitationRequest, ElicitationAction,
        ElicitationContentValue, ElicitationFormMode, ElicitationSchema, ElicitationSessionScope,
        EnumOption, StringPropertySchema,
    };
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn form_request_exposes_typed_fields_and_builds_an_id() {
        let request = acp_elicitation_request(form_request());

        assert_eq!("session", request.session_id);
        let super::AcpElicitationMode::Form(form) = &request.mode else {
            panic!("expected form mode");
        };
        assert_eq!(Some("Deploy target"), form.title.as_deref());
        // 属性来自 BTreeMap，顺序按名字排：confirm 在 env 前。
        assert_eq!(vec!["confirm", "env"], names(form));
        let env = field(form, "env");
        let confirm = field(form, "confirm");
        // `env` 是必填且带标题；`confirm` 不是必填，标题缺省时回落成属性名。
        assert!(env.required);
        assert_eq!("Environment", env.title);
        assert!(!confirm.required);
        assert_eq!("confirm", confirm.title);
        // request_id 由「会话 + 字段名」拼出，供视图去重与回填答案。
        assert_eq!("session:confirm,env", request.request_id);
    }

    #[test]
    fn string_enum_becomes_single_select_and_boolean_stays_boolean() {
        let request = acp_elicitation_request(form_request());

        let super::AcpElicitationMode::Form(form) = &request.mode else {
            panic!("expected form mode");
        };
        match &field(form, "env").kind {
            AcpElicitationFieldKind::SingleSelect { options } => {
                assert_eq!(2, options.len());
                assert_eq!("prod", options[0].value);
                assert_eq!("Production", options[0].title);
            }
            other => panic!("expected single select, got {other:?}"),
        }
        match &field(form, "confirm").kind {
            AcpElicitationFieldKind::Boolean => {}
            other => panic!("expected boolean, got {other:?}"),
        }
        assert!(field(form, "env").kind.is_editable());
    }

    #[test]
    fn content_values_cover_protocol_scalars_and_drop_objects() {
        assert_eq!(
            Some(ElicitationContentValue::String("hi".to_string())),
            content_value(&json!("hi"))
        );
        assert_eq!(
            Some(ElicitationContentValue::Boolean(true)),
            content_value(&json!(true))
        );
        assert_eq!(
            Some(ElicitationContentValue::Integer(3)),
            content_value(&json!(3))
        );
        assert_eq!(
            Some(ElicitationContentValue::StringArray(vec!["a".to_string()])),
            content_value(&json!(["a"]))
        );
        assert_eq!(None, content_value(&json!({"nested": 1})));
        assert_eq!(None, content_value(&json!([1, 2])));
    }

    #[tokio::test]
    async fn elicitation_channel_delivers_request_and_returns_content() {
        let (provider, mut receiver) = acp_elicitation_channel();
        let pending = tokio::spawn(provider(acp_elicitation_request(form_request())));

        let AcpElicitationMessage::Requested(envelope) =
            receiver.recv().await.expect("elicitation request")
        else {
            panic!("expected elicitation request");
        };
        // 通道会给 request_id 加一段唯一前缀，避免同一视图上多个连接串答案。
        let request_id = envelope.request().request_id.clone();
        assert!(request_id.ends_with(":session:confirm,env"));
        let mut content = BTreeMap::new();
        content.insert("env".to_string(), json!("prod"));
        assert!(envelope.resolve(AcpElicitationOutcome::Accept(content)));

        assert_eq!(
            AcpElicitationOutcome::Accept(BTreeMap::from([("env".to_string(), json!("prod"))])),
            pending.await.expect("elicitation outcome")
        );
    }

    #[tokio::test]
    async fn decline_maps_to_the_decline_action() {
        let provider = Arc::new(|_: super::AcpElicitationRequest| {
            Box::pin(async move { AcpElicitationOutcome::Decline }) as super::AcpElicitationFuture
        });

        let response = resolve_acp_elicitation_request(Some(provider), form_request()).await;

        assert!(matches!(response.action, ElicitationAction::Decline));
    }

    #[tokio::test]
    async fn accept_maps_content_onto_the_protocol_action() {
        let provider = Arc::new(|_: super::AcpElicitationRequest| {
            Box::pin(async move {
                let mut content = BTreeMap::new();
                content.insert("env".to_string(), json!("prod"));
                content.insert("confirm".to_string(), json!(true));
                // 协议不认识的嵌套对象应被丢弃，而不是让整条响应失败。
                content.insert("nested".to_string(), json!({"a": 1}));
                AcpElicitationOutcome::Accept(content)
            }) as super::AcpElicitationFuture
        });

        let response = resolve_acp_elicitation_request(Some(provider), form_request()).await;

        let ElicitationAction::Accept(accept) = response.action else {
            panic!("expected accept");
        };
        let content = accept.content.expect("content");
        assert_eq!(2, content.len());
        assert_eq!(
            Some(&ElicitationContentValue::String("prod".to_string())),
            content.get("env")
        );
        assert!(!content.contains_key("nested"));
    }

    #[tokio::test]
    async fn request_without_provider_is_cancelled() {
        let response = resolve_acp_elicitation_request(None, form_request()).await;

        assert!(matches!(response.action, ElicitationAction::Cancel));
    }

    #[tokio::test]
    async fn timeout_notifies_view_and_returns_cancel() {
        let (provider, mut receiver) = acp_elicitation_channel_with_timeout(Duration::ZERO);
        let pending = tokio::spawn(provider(acp_elicitation_request(form_request())));

        let AcpElicitationMessage::Requested(envelope) =
            receiver.recv().await.expect("elicitation request")
        else {
            panic!("expected elicitation request");
        };
        let request_id = envelope.request().request_id.clone();
        let AcpElicitationMessage::Expired {
            request_id: expired_id,
        } = receiver.recv().await.expect("expiration notification")
        else {
            panic!("expected expiration notification");
        };

        assert_eq!(request_id, expired_id);
        assert!(!envelope.resolve(AcpElicitationOutcome::Cancel));
        assert_eq!(
            AcpElicitationOutcome::Cancel,
            pending.await.expect("elicitation outcome")
        );
    }

    fn names(form: &super::AcpElicitationForm) -> Vec<&str> {
        form.fields
            .iter()
            .map(|field| field.name.as_str())
            .collect()
    }

    fn field<'a>(
        form: &'a super::AcpElicitationForm,
        name: &str,
    ) -> &'a super::AcpElicitationField {
        form.fields
            .iter()
            .find(|field| field.name == name)
            .unwrap_or_else(|| panic!("missing field {name}"))
    }

    fn form_request() -> CreateElicitationRequest {
        let schema = ElicitationSchema::new()
            .title("Deploy target")
            .property(
                "env",
                StringPropertySchema::new()
                    .title("Environment")
                    .one_of(vec![
                        EnumOption::new("prod", "Production"),
                        EnumOption::new("staging", "Staging"),
                    ]),
                true,
            )
            .property("confirm", BooleanPropertySchema::new(), false);
        CreateElicitationRequest::new(
            ElicitationFormMode::new(ElicitationSessionScope::new("session"), schema),
            "Which environment should I deploy to?",
        )
    }
}
