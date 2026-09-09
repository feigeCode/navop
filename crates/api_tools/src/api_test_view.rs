//! 接口测试视图：左侧目录/请求树 + 请求编辑区 + 响应区。
//!
//! 参考 verve 的请求调试页（`src/ui/request_panel` + `src/scripting.rs`）实现：
//! 目录与请求持久化到应用配置目录；请求面板包含参数、路径变量、请求头、请求体、
//! 鉴权、Cookie、预执行脚本与 Tests 脚本；响应面板包含响应体、响应头与 Cookies。
//! 测试脚本用 boa_engine 执行 `apt.assert` 断言并展示通过/失败统计。

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_component::list::ListItem;
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::popover::Popover;
use gpui_component::resizable::{h_resizable, resizable_panel, v_resizable};
use gpui_component::scroll::{Scrollbar, ScrollbarShow};
use gpui_component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_component::spinner::Spinner;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::tag::Tag;
use gpui_component::tree::{Tree, TreeItem, TreeState};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, IndexPath, Sizable as _, WindowExt as _, h_flex,
    v_flex,
};
use one_ui::{ContentState, PanelHeader, PanelHeaderVariant, StatusBar, StatusPresentation};
use rust_i18n::t;

use crate::Protocol;
use crate::http::{
    AuthConfig, AuthTarget, AuthType, BodyType, HttpResponse, KeyValue, PreparedRequest,
    RawLanguage, RequestMethod,
};
use crate::request_debug::{actual_request_text, console_text, curl_command, response_cookie_pair};
use crate::request_store::{
    self, ApiEnvironment, ApiStore, RequestHistoryEntry, ResponseExample,
    ResponseExampleAutoSaveMode, StoredFolder, StoredRequest,
};
use crate::scripting::{self, ScriptResult};
use crate::tree_model::{ancestor_folder_ids, descendant_folder_ids};

mod send;
mod socket_io_state;
mod socket_io_view;
mod tcp_state;
mod tcp_view;
mod websocket_state;
mod websocket_view;

use socket_io_state::{SocketIoSession, SocketIoState};
use tcp_state::TcpSession;
use websocket_state::{WebSocketSession, WebSocketState};

const REQUEST_TIMEOUT_SECS: u64 = 30;
const REQUEST_TREE_WIDTH: f32 = 280.;
const REQUEST_TREE_COLLAPSED_WIDTH: f32 = 28.;
const TREE_ROOT_ID: &str = "api-requests-root";
const TREE_EMPTY_ID: &str = "api-requests-empty";

#[derive(Clone)]
struct ProtocolOption(Protocol);

impl SelectItem for ProtocolOption {
    type Value = Protocol;

    fn title(&self) -> SharedString {
        SharedString::from(self.0.label())
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }
}

/// 方法下拉选项（实现 gpui-component 的 `SelectItem`）。
#[derive(Clone)]
struct MethodOption(RequestMethod);

impl SelectItem for MethodOption {
    type Value = RequestMethod;

    fn title(&self) -> SharedString {
        SharedString::from(self.0.label())
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }
}

#[derive(Clone)]
struct BodyTypeOption(BodyType);

impl SelectItem for BodyTypeOption {
    type Value = BodyType;

    fn title(&self) -> SharedString {
        SharedString::from(self.0.label())
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }
}

#[derive(Clone)]
struct RawLanguageOption(RawLanguage);

impl SelectItem for RawLanguageOption {
    type Value = RawLanguage;

    fn title(&self) -> SharedString {
        SharedString::from(self.0.label())
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }
}

#[derive(Clone)]
struct AuthTypeOption(AuthType);

impl SelectItem for AuthTypeOption {
    type Value = AuthType;

    fn title(&self) -> SharedString {
        SharedString::from(self.0.label())
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }
}

#[derive(Clone)]
struct AuthTargetOption(AuthTarget);

impl SelectItem for AuthTargetOption {
    type Value = AuthTarget;

    fn title(&self) -> SharedString {
        SharedString::from(self.0.label())
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }
}

#[derive(Clone)]
struct EnvironmentOption {
    id: String,
    name: String,
}

impl SelectItem for EnvironmentOption {
    type Value = String;

    fn title(&self) -> SharedString {
        SharedString::from(self.name.clone())
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarMode {
    Collections,
    History,
}

#[derive(Clone, Copy)]
enum CollectionExport {
    PostmanJson,
    OpenApiJson,
    OpenApiYaml,
    SwaggerJson,
    SwaggerYaml,
}

impl CollectionExport {
    const ALL: [Self; 5] = [
        Self::PostmanJson,
        Self::OpenApiJson,
        Self::OpenApiYaml,
        Self::SwaggerJson,
        Self::SwaggerYaml,
    ];

    fn file_name(self) -> &'static str {
        match self {
            Self::PostmanJson => "navop-postman-collection.json",
            Self::OpenApiJson => "navop-openapi.json",
            Self::OpenApiYaml => "navop-openapi.yaml",
            Self::SwaggerJson => "navop-swagger.json",
            Self::SwaggerYaml => "navop-swagger.yaml",
        }
    }

    fn prompt(self) -> String {
        match self {
            Self::PostmanJson => t!("ApiTest.export_postman").to_string(),
            Self::OpenApiJson | Self::OpenApiYaml => t!("ApiTest.export_openapi").to_string(),
            Self::SwaggerJson | Self::SwaggerYaml => t!("ApiTest.export_swagger").to_string(),
        }
    }

    fn menu_label(self) -> String {
        let encoding = match self {
            Self::PostmanJson | Self::OpenApiJson | Self::SwaggerJson => "JSON",
            Self::OpenApiYaml | Self::SwaggerYaml => "YAML",
        };
        format!("{} · {encoding}", self.prompt())
    }

    fn serialize(self, store: &ApiStore) -> anyhow::Result<String> {
        match self {
            Self::PostmanJson => {
                crate::collection_io::export_postman_v2_1("Navop Collection", store)
            }
            Self::OpenApiJson => crate::schema_io::export_openapi(
                "Navop API",
                store,
                crate::schema_io::DocumentEncoding::Json,
            ),
            Self::OpenApiYaml => crate::schema_io::export_openapi(
                "Navop API",
                store,
                crate::schema_io::DocumentEncoding::Yaml,
            ),
            Self::SwaggerJson => crate::schema_io::export_swagger(
                "Navop API",
                store,
                crate::schema_io::DocumentEncoding::Json,
            ),
            Self::SwaggerYaml => crate::schema_io::export_swagger(
                "Navop API",
                store,
                crate::schema_io::DocumentEncoding::Yaml,
            ),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RequestEditorTab {
    Params,
    Path,
    Headers,
    Body,
    Auth,
    Cookies,
    PreRequest,
    Tests,
    Variables,
}

const REQUEST_EDITOR_TABS: [RequestEditorTab; 9] = [
    RequestEditorTab::Params,
    RequestEditorTab::Path,
    RequestEditorTab::Headers,
    RequestEditorTab::Body,
    RequestEditorTab::Auth,
    RequestEditorTab::Cookies,
    RequestEditorTab::PreRequest,
    RequestEditorTab::Tests,
    RequestEditorTab::Variables,
];

impl RequestEditorTab {
    fn title(self) -> String {
        match self {
            RequestEditorTab::Params => t!("ApiTest.params").to_string(),
            RequestEditorTab::Path => t!("ApiTest.path").to_string(),
            RequestEditorTab::Headers => t!("ApiTest.headers").to_string(),
            RequestEditorTab::Body => t!("ApiTest.body").to_string(),
            RequestEditorTab::Auth => t!("ApiTest.auth").to_string(),
            RequestEditorTab::Cookies => t!("ApiTest.cookies").to_string(),
            RequestEditorTab::PreRequest => t!("ApiTest.pre_request").to_string(),
            RequestEditorTab::Tests => t!("ApiTest.tests").to_string(),
            RequestEditorTab::Variables => t!("ApiTest.variables").to_string(),
        }
    }

    fn element_id(self) -> &'static str {
        match self {
            RequestEditorTab::Params => "api-editor-tab-params",
            RequestEditorTab::Path => "api-editor-tab-path",
            RequestEditorTab::Headers => "api-editor-tab-headers",
            RequestEditorTab::Body => "api-editor-tab-body",
            RequestEditorTab::Auth => "api-editor-tab-auth",
            RequestEditorTab::Cookies => "api-editor-tab-cookies",
            RequestEditorTab::PreRequest => "api-editor-tab-pre-request",
            RequestEditorTab::Tests => "api-editor-tab-tests",
            RequestEditorTab::Variables => "api-editor-tab-variables",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ResponseTab {
    Body,
    Headers,
    Cookies,
    ActualRequest,
    Curl,
    Console,
    Example,
}

const RESPONSE_TABS: [ResponseTab; 7] = [
    ResponseTab::Body,
    ResponseTab::Headers,
    ResponseTab::Cookies,
    ResponseTab::ActualRequest,
    ResponseTab::Curl,
    ResponseTab::Console,
    ResponseTab::Example,
];

impl ResponseTab {
    fn title(self) -> String {
        match self {
            ResponseTab::Body => t!("ApiTest.response_body").to_string(),
            ResponseTab::Headers => t!("ApiTest.response_headers").to_string(),
            ResponseTab::Cookies => t!("ApiTest.response_cookies").to_string(),
            ResponseTab::ActualRequest => t!("ApiTest.actual_request").to_string(),
            ResponseTab::Curl => "cURL".to_string(),
            ResponseTab::Console => t!("ApiTest.console").to_string(),
            ResponseTab::Example => t!("ApiTest.response_example").to_string(),
        }
    }

    fn element_id(self) -> &'static str {
        match self {
            ResponseTab::Body => "api-response-tab-body",
            ResponseTab::Headers => "api-response-tab-headers",
            ResponseTab::Cookies => "api-response-tab-cookies",
            ResponseTab::ActualRequest => "api-response-tab-actual-request",
            ResponseTab::Curl => "api-response-tab-curl",
            ResponseTab::Console => "api-response-tab-console",
            ResponseTab::Example => "api-response-tab-example",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum KvSection {
    Params,
    Path,
    Headers,
    Cookies,
    Body,
    Globals,
    GlobalParams,
    GlobalHeaders,
    GlobalCookies,
    Environment,
    EnvironmentParams,
    EnvironmentHeaders,
    EnvironmentCookies,
    FolderParams,
    FolderHeaders,
    FolderVariables,
    RequestVariables,
}

impl KvSection {
    fn element_id(self) -> &'static str {
        match self {
            KvSection::Params => "params",
            KvSection::Path => "path",
            KvSection::Headers => "headers",
            KvSection::Cookies => "cookies",
            KvSection::Body => "body",
            KvSection::Globals => "globals",
            KvSection::GlobalParams => "global-params",
            KvSection::GlobalHeaders => "global-headers",
            KvSection::GlobalCookies => "global-cookies",
            KvSection::Environment => "environment",
            KvSection::EnvironmentParams => "environment-params",
            KvSection::EnvironmentHeaders => "environment-headers",
            KvSection::EnvironmentCookies => "environment-cookies",
            KvSection::FolderParams => "folder-params",
            KvSection::FolderHeaders => "folder-headers",
            KvSection::FolderVariables => "folder-variables",
            KvSection::RequestVariables => "request-variables",
        }
    }
}

const KV_SECTIONS: [KvSection; 17] = [
    KvSection::Params,
    KvSection::Path,
    KvSection::Headers,
    KvSection::Cookies,
    KvSection::Body,
    KvSection::Globals,
    KvSection::GlobalParams,
    KvSection::GlobalHeaders,
    KvSection::GlobalCookies,
    KvSection::Environment,
    KvSection::EnvironmentParams,
    KvSection::EnvironmentHeaders,
    KvSection::EnvironmentCookies,
    KvSection::FolderParams,
    KvSection::FolderHeaders,
    KvSection::FolderVariables,
    KvSection::RequestVariables,
];

#[derive(Clone)]
struct KvRow {
    key: Entity<InputState>,
    value: Entity<InputState>,
    enabled: bool,
    field_type: crate::http::FieldType,
    file_path: Option<String>,
}

pub struct ApiTestView {
    name_input: Entity<InputState>,
    search_input: Entity<InputState>,
    protocol_select: Entity<SelectState<Vec<ProtocolOption>>>,
    method_select: Entity<SelectState<Vec<MethodOption>>>,
    url_input: Entity<InputState>,
    request_description_input: Entity<TextareaState>,
    websocket_message_input: Entity<TextareaState>,
    socket_io_message_input: Entity<TextareaState>,
    tcp_message_input: Entity<TextareaState>,
    body_input: Entity<TextareaState>,
    pre_script_input: Entity<TextareaState>,
    tests_input: Entity<TextareaState>,
    body_type_select: Entity<SelectState<Vec<BodyTypeOption>>>,
    raw_lang_select: Entity<SelectState<Vec<RawLanguageOption>>>,
    auth_type_select: Entity<SelectState<Vec<AuthTypeOption>>>,
    auth_target_select: Entity<SelectState<Vec<AuthTargetOption>>>,
    auth_token_input: Entity<InputState>,
    auth_username_input: Entity<InputState>,
    auth_password_input: Entity<InputState>,
    auth_key_input: Entity<InputState>,
    auth_value_input: Entity<InputState>,
    folder_base_url_input: Entity<InputState>,
    folder_description_input: Entity<TextareaState>,
    environment_base_url_input: Entity<InputState>,
    variables_environment_select: Entity<SelectState<Vec<EnvironmentOption>>>,
    folders: Vec<StoredFolder>,
    requests: Vec<StoredRequest>,
    param_rows: Vec<KvRow>,
    path_rows: Vec<KvRow>,
    header_rows: Vec<KvRow>,
    cookie_rows: Vec<KvRow>,
    body_rows: Vec<KvRow>,
    global_rows: Vec<KvRow>,
    global_param_rows: Vec<KvRow>,
    global_header_rows: Vec<KvRow>,
    global_cookie_rows: Vec<KvRow>,
    environment_rows: Vec<KvRow>,
    environment_param_rows: Vec<KvRow>,
    environment_header_rows: Vec<KvRow>,
    environment_cookie_rows: Vec<KvRow>,
    folder_param_rows: Vec<KvRow>,
    folder_header_rows: Vec<KvRow>,
    folder_variable_rows: Vec<KvRow>,
    variable_rows: Vec<KvRow>,
    kv_scroll_handles: BTreeMap<KvSection, ScrollHandle>,
    response_headers_scroll_handle: ScrollHandle,
    response_cookies_scroll_handle: ScrollHandle,
    // 各常显滚动区的独立句柄(新 gpui-component 需显式 Scrollbar 挂接)
    environment_switcher_scroll_handle: ScrollHandle,
    environment_list_scroll_handle: ScrollHandle,
    environment_settings_scroll_handle: ScrollHandle,
    history_scroll_handle: ScrollHandle,
    folder_editor_scroll_handle: ScrollHandle,
    globals_scroll_handle: ScrollHandle,
    response_body_scroll_handle: ScrollHandle,
    response_examples_scroll_handle: ScrollHandle,
    response_example_card_scroll_handle: ScrollHandle,
    response_console_scroll_handle: ScrollHandle,
    websocket_timeline_scroll_handle: ScrollHandle,
    socket_io_timeline_scroll_handle: ScrollHandle,
    tcp_timeline_scroll_handle: ScrollHandle,
    globals: Vec<KeyValue>,
    global_params: Vec<KeyValue>,
    global_headers: Vec<KeyValue>,
    global_cookies: Vec<KeyValue>,
    environments: Vec<ApiEnvironment>,
    active_environment_id: Option<String>,
    history: Vec<RequestHistoryEntry>,
    response_example_autosave: ResponseExampleAutoSaveMode,
    tree_state: Entity<TreeState>,
    active_request_id: Option<String>,
    active_folder_id: Option<String>,
    sidebar_mode: SidebarMode,
    sidebar_collapsed: bool,
    active_editor_tab: RequestEditorTab,
    active_response_tab: ResponseTab,
    sending: bool,
    stream_stop: Option<Arc<AtomicBool>>,
    websocket_state: WebSocketSession,
    websocket_commands: Option<tokio::sync::mpsc::Sender<crate::websocket::ConnectionCommand>>,
    websocket_cancel: Option<tokio::sync::oneshot::Sender<()>>,
    websocket_generation: u64,
    socket_io_state: SocketIoSession,
    socket_io_commands: Option<tokio::sync::mpsc::Sender<crate::websocket::ConnectionCommand>>,
    socket_io_cancel: Option<tokio::sync::oneshot::Sender<()>>,
    socket_io_generation: u64,
    tcp_state: TcpSession,
    tcp_commands: Option<tokio::sync::mpsc::Sender<crate::tcp::ConnectionCommand>>,
    tcp_cancel: Option<tokio::sync::oneshot::Sender<()>>,
    tcp_generation: u64,
    response: Option<HttpResponse>,
    prepared_request: Option<PreparedRequest>,
    pre_result: Option<ScriptResult>,
    test_result: Option<ScriptResult>,
    response_pretty: bool,
    request_generation: u64,
    notice: Option<String>,
    suppress_commit: bool,
    pub(crate) focus_handle: FocusHandle,
    _subs: Vec<Subscription>,
    _row_subs: Vec<Subscription>,
}

impl ApiTestView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let protocol_options = Protocol::ALL
            .iter()
            .map(|protocol| ProtocolOption(*protocol))
            .collect::<Vec<_>>();
        let protocol_select = cx.new(|cx| {
            SelectState::new(
                protocol_options,
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });
        let method_options = RequestMethod::ALL
            .iter()
            .map(|m| MethodOption(*m))
            .collect::<Vec<_>>();
        let method_select = cx.new(|cx| {
            SelectState::new(
                method_options,
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });
        let body_type_options = BodyType::ALL
            .iter()
            .map(|body_type| BodyTypeOption(*body_type))
            .collect::<Vec<_>>();
        let body_type_select = cx.new(|cx| {
            SelectState::new(
                body_type_options,
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });
        let raw_lang_options = RawLanguage::ALL
            .iter()
            .map(|lang| RawLanguageOption(*lang))
            .collect::<Vec<_>>();
        let raw_lang_select = cx.new(|cx| {
            SelectState::new(
                raw_lang_options,
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });
        let auth_type_options = AuthType::ALL
            .iter()
            .map(|auth_type| AuthTypeOption(*auth_type))
            .collect::<Vec<_>>();
        let auth_type_select = cx.new(|cx| {
            SelectState::new(
                auth_type_options,
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });
        let auth_target_options = AuthTarget::ALL
            .iter()
            .map(|target| AuthTargetOption(*target))
            .collect::<Vec<_>>();
        let auth_target_select = cx.new(|cx| {
            SelectState::new(
                auth_target_options,
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });
        let mut store = request_store::load_store();
        if store.environments.is_empty() {
            store.environments.push(ApiEnvironment::new(
                t!("ApiTest.default_environment").to_string(),
            ));
        }
        if store
            .active_environment_id
            .as_ref()
            .is_none_or(|active_id| !store.environments.iter().any(|env| &env.id == active_id))
        {
            store.active_environment_id = store.environments.first().map(|env| env.id.clone());
        }
        let environment_options = store
            .environments
            .iter()
            .map(|environment| EnvironmentOption {
                id: environment.id.clone(),
                name: environment.name.clone(),
            })
            .collect::<Vec<_>>();
        let selected_environment = store
            .active_environment_id
            .as_deref()
            .and_then(|id| {
                environment_options
                    .iter()
                    .position(|option| option.id == id)
            })
            .map(|row| IndexPath::default().row(row));
        let variables_environment_select =
            cx.new(|cx| SelectState::new(environment_options, selected_environment, window, cx));
        let active_environment = store
            .active_environment_id
            .as_deref()
            .and_then(|active_id| {
                store
                    .environments
                    .iter()
                    .find(|environment| environment.id == active_id)
            })
            .cloned();

        let name_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("ApiTest.request_name").to_string())
        });
        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("ApiTest.search_requests").to_string())
        });
        let url_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("ApiTest.url_placeholder").to_string())
        });
        let request_description_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 4)
                .placeholder(t!("ApiTest.request_description_placeholder").to_string())
        });
        let websocket_message_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 6)
                .placeholder(t!("ApiTest.websocket_message_placeholder").to_string())
        });
        let socket_io_message_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 6)
                .placeholder(t!("ApiTest.socketio_message_placeholder").to_string())
        });
        let tcp_message_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 6)
                .placeholder(t!("ApiTest.tcp_message_placeholder").to_string())
        });
        let body_input = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder(t!("ApiTest.body_placeholder").to_string())
        });
        let pre_script_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t!("ApiTest.pre_request_placeholder").to_string())
        });
        let tests_input = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder(t!("ApiTest.tests_placeholder").to_string())
        });
        let auth_token_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("ApiTest.auth_token").to_string())
        });
        let auth_username_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("ApiTest.auth_username").to_string())
        });
        let auth_password_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("ApiTest.auth_password").to_string())
        });
        let auth_key_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(t!("ApiTest.auth_key").to_string()));
        let auth_value_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("ApiTest.auth_value").to_string())
        });
        let folder_base_url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("ApiTest.folder_base_url_placeholder").to_string())
        });
        let folder_description_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 4)
                .placeholder(t!("ApiTest.folder_description_placeholder").to_string())
        });
        let environment_base_url = active_environment
            .as_ref()
            .and_then(|environment| environment.base_url.clone())
            .unwrap_or_default();
        let environment_base_url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(environment_base_url)
                .placeholder(t!("ApiTest.environment_base_url_placeholder").to_string())
        });

        // 基础输入失焦时回写当前请求。
        let mut subs = Vec::new();
        // 单行 InputState 输入失焦回写
        for input in [
            name_input.clone(),
            url_input.clone(),
            auth_token_input.clone(),
            auth_username_input.clone(),
            auth_password_input.clone(),
            auth_key_input.clone(),
            auth_value_input.clone(),
            folder_base_url_input.clone(),
            environment_base_url_input.clone(),
        ] {
            let sub = cx.subscribe(&input, move |this: &mut Self, _src, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change | InputEvent::Blur) {
                    this.commit_current_to_store(cx);
                }
            });
            subs.push(sub);
        }
        // 多行 TextareaState 输入失焦回写
        for textarea in [
            request_description_input.clone(),
            body_input.clone(),
            pre_script_input.clone(),
            tests_input.clone(),
            folder_description_input.clone(),
        ] {
            let sub = cx.subscribe(
                &textarea,
                move |this: &mut Self, _src, ev: &InputEvent, cx| {
                    if matches!(ev, InputEvent::Change | InputEvent::Blur) {
                        this.commit_current_to_store(cx);
                    }
                },
            );
            subs.push(sub);
        }
        let search_sub = cx.subscribe(
            &search_input,
            move |this: &mut Self, _src, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.rebuild_tree(cx);
                    cx.notify();
                }
            },
        );
        subs.push(search_sub);
        let websocket_message_sub = cx.subscribe_in(
            &websocket_message_input,
            window,
            |this, _src, ev: &InputEvent, window, cx| {
                if matches!(
                    ev,
                    InputEvent::PressEnter {
                        secondary: false,
                        shift: _
                    }
                ) {
                    this.send_websocket_message(window, cx);
                }
            },
        );
        subs.push(websocket_message_sub);
        let socket_io_message_sub = cx.subscribe_in(
            &socket_io_message_input,
            window,
            |this, _src, ev: &InputEvent, window, cx| {
                if matches!(
                    ev,
                    InputEvent::PressEnter {
                        secondary: false,
                        shift: _
                    }
                ) {
                    this.send_socket_io_message(window, cx);
                }
            },
        );
        subs.push(socket_io_message_sub);
        let tcp_message_sub = cx.subscribe_in(
            &tcp_message_input,
            window,
            |this, _src, ev: &InputEvent, window, cx| {
                if matches!(
                    ev,
                    InputEvent::PressEnter {
                        secondary: false,
                        shift: _
                    }
                ) {
                    this.send_tcp_message(window, cx);
                }
            },
        );
        subs.push(tcp_message_sub);
        let method_sub = cx.subscribe(
            &method_select,
            move |this: &mut Self, _src, _ev: &SelectEvent<Vec<MethodOption>>, cx| {
                this.commit_current_to_store(cx);
            },
        );
        subs.push(method_sub);
        let protocol_sub = cx.subscribe(
            &protocol_select,
            move |this: &mut Self, _src, _ev: &SelectEvent<Vec<ProtocolOption>>, cx| {
                if !this.suppress_commit {
                    this.request_generation = this.request_generation.wrapping_add(1);
                    this.cancel_stream();
                    this.cancel_websocket();
                    this.cancel_socket_io();
                    this.cancel_tcp();
                }
                this.commit_current_to_store(cx);
                cx.notify();
            },
        );
        subs.push(protocol_sub);
        let body_type_sub = cx.subscribe(
            &body_type_select,
            move |this: &mut Self, _src, _ev: &SelectEvent<Vec<BodyTypeOption>>, cx| {
                this.commit_current_to_store(cx);
            },
        );
        subs.push(body_type_sub);
        let raw_lang_sub = cx.subscribe(
            &raw_lang_select,
            move |this: &mut Self, _src, _ev: &SelectEvent<Vec<RawLanguageOption>>, cx| {
                this.commit_current_to_store(cx);
            },
        );
        subs.push(raw_lang_sub);
        let auth_type_sub = cx.subscribe(
            &auth_type_select,
            move |this: &mut Self, _src, _ev: &SelectEvent<Vec<AuthTypeOption>>, cx| {
                this.commit_current_to_store(cx);
            },
        );
        subs.push(auth_type_sub);
        let auth_target_sub = cx.subscribe(
            &auth_target_select,
            move |this: &mut Self, _src, _ev: &SelectEvent<Vec<AuthTargetOption>>, cx| {
                this.commit_current_to_store(cx);
            },
        );
        subs.push(auth_target_sub);
        let variables_environment_sub = cx.subscribe_in(
            &variables_environment_select,
            window,
            |this, _, event: &SelectEvent<Vec<EnvironmentOption>>, window, cx| {
                if let SelectEvent::Confirm(Some(id)) = event {
                    this.select_environment(id, window, cx);
                }
            },
        );
        subs.push(variables_environment_sub);
        let global_rows = Self::create_kv_rows(&store.globals, "Key", "Value", window, cx);
        let global_param_rows =
            Self::create_kv_rows(&store.global_params, "Key", "Value", window, cx);
        let global_header_rows =
            Self::create_kv_rows(&store.global_headers, "Header", "Value", window, cx);
        let global_cookie_rows =
            Self::create_kv_rows(&store.global_cookies, "Cookie", "Value", window, cx);
        let environment_values = active_environment
            .as_ref()
            .map(|environment| environment.variables.as_slice())
            .unwrap_or_default();
        let environment_param_values = active_environment
            .as_ref()
            .map(|environment| environment.params.as_slice())
            .unwrap_or_default();
        let environment_header_values = active_environment
            .as_ref()
            .map(|environment| environment.headers.as_slice())
            .unwrap_or_default();
        let environment_cookie_values = active_environment
            .as_ref()
            .map(|environment| environment.cookies.as_slice())
            .unwrap_or_default();
        let environment_rows = Self::create_kv_rows(environment_values, "Key", "Value", window, cx);
        let environment_param_rows =
            Self::create_kv_rows(environment_param_values, "Key", "Value", window, cx);
        let environment_header_rows =
            Self::create_kv_rows(environment_header_values, "Header", "Value", window, cx);
        let environment_cookie_rows =
            Self::create_kv_rows(environment_cookie_values, "Cookie", "Value", window, cx);
        let tree_state = cx.new(|cx| TreeState::new(cx));
        let first_id = store.requests.first().map(|r| r.id.clone());
        let kv_scroll_handles = KV_SECTIONS
            .into_iter()
            .map(|section| (section, ScrollHandle::new()))
            .collect();
        let mut this = Self {
            name_input,
            search_input,
            protocol_select,
            method_select,
            url_input,
            request_description_input,
            websocket_message_input,
            socket_io_message_input,
            tcp_message_input,
            body_input,
            pre_script_input,
            tests_input,
            body_type_select,
            raw_lang_select,
            auth_type_select,
            auth_target_select,
            auth_token_input,
            auth_username_input,
            auth_password_input,
            auth_key_input,
            auth_value_input,
            folder_base_url_input,
            folder_description_input,
            environment_base_url_input,
            variables_environment_select,
            folders: store.folders,
            requests: store.requests,
            param_rows: Vec::new(),
            path_rows: Vec::new(),
            header_rows: Vec::new(),
            cookie_rows: Vec::new(),
            body_rows: Vec::new(),
            global_rows,
            global_param_rows,
            global_header_rows,
            global_cookie_rows,
            environment_rows,
            environment_param_rows,
            environment_header_rows,
            environment_cookie_rows,
            folder_param_rows: Vec::new(),
            folder_header_rows: Vec::new(),
            folder_variable_rows: Vec::new(),
            variable_rows: Vec::new(),
            kv_scroll_handles,
            response_headers_scroll_handle: ScrollHandle::new(),
            response_cookies_scroll_handle: ScrollHandle::new(),
            environment_switcher_scroll_handle: ScrollHandle::new(),
            environment_list_scroll_handle: ScrollHandle::new(),
            environment_settings_scroll_handle: ScrollHandle::new(),
            history_scroll_handle: ScrollHandle::new(),
            folder_editor_scroll_handle: ScrollHandle::new(),
            globals_scroll_handle: ScrollHandle::new(),
            response_body_scroll_handle: ScrollHandle::new(),
            response_examples_scroll_handle: ScrollHandle::new(),
            response_example_card_scroll_handle: ScrollHandle::new(),
            response_console_scroll_handle: ScrollHandle::new(),
            websocket_timeline_scroll_handle: ScrollHandle::new(),
            socket_io_timeline_scroll_handle: ScrollHandle::new(),
            tcp_timeline_scroll_handle: ScrollHandle::new(),
            globals: store.globals,
            global_params: store.global_params,
            global_headers: store.global_headers,
            global_cookies: store.global_cookies,
            environments: store.environments,
            active_environment_id: store.active_environment_id,
            history: store.history,
            response_example_autosave: store.response_example_autosave,
            tree_state,
            active_request_id: None,
            active_folder_id: None,
            sidebar_mode: SidebarMode::Collections,
            sidebar_collapsed: false,
            active_editor_tab: RequestEditorTab::Params,
            active_response_tab: ResponseTab::Body,
            sending: false,
            stream_stop: None,
            websocket_state: WebSocketSession::new(),
            websocket_commands: None,
            websocket_cancel: None,
            websocket_generation: 0,
            socket_io_state: SocketIoSession::new(),
            socket_io_commands: None,
            socket_io_cancel: None,
            socket_io_generation: 0,
            tcp_state: TcpSession::new(),
            tcp_commands: None,
            tcp_cancel: None,
            tcp_generation: 0,
            response: None,
            prepared_request: None,
            pre_result: None,
            test_result: None,
            response_pretty: true,
            request_generation: 0,
            notice: None,
            suppress_commit: false,
            focus_handle: cx.focus_handle(),
            _subs: subs,
            _row_subs: Vec::new(),
        };
        if let Some(id) = first_id {
            this.load_request(&id, window, cx);
        } else {
            this.subscribe_editor_inputs(cx);
            this.rebuild_tree(cx);
        }
        this
    }

    /// 解析「每行一个 `Key: Value`」的请求头文本。
    fn parse_headers(text: &str) -> Result<Vec<KeyValue>, String> {
        let mut headers = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                return Err(t!("ApiTest.header_error", line = line.to_string()).to_string());
            };
            headers.push(KeyValue::new(key.trim(), value.trim()));
        }
        Ok(headers)
    }

    fn current_method(&self, cx: &App) -> RequestMethod {
        self.method_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(RequestMethod::Get)
    }

    fn current_protocol(&self, cx: &App) -> Protocol {
        self.protocol_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or_default()
    }

    fn request_label(&self, request: &StoredRequest) -> String {
        if request.name.trim().is_empty() {
            if request.url.trim().is_empty() {
                t!("ApiTest.new_request").to_string()
            } else {
                request.url.trim().to_string()
            }
        } else {
            request.name.trim().to_string()
        }
    }

    fn request_badge_label(protocol: Protocol, method: RequestMethod) -> &'static str {
        protocol.badge_label().unwrap_or(method.badge_label())
    }

    fn child_tree_items(&self, parent_id: Option<&str>, query: &str) -> Vec<TreeItem> {
        let mut children = Vec::new();
        for folder in self
            .folders
            .iter()
            .filter(|folder| folder.parent_id.as_deref() == parent_id)
        {
            let mut item = TreeItem::new(folder.id.clone(), folder.name.clone()).expanded(true);
            let grandchildren = self.child_tree_items(Some(folder.id.as_str()), query);
            let folder_matches = folder.name.to_lowercase().contains(query);
            if !grandchildren.is_empty() {
                item = item.children(grandchildren);
            }
            if query.is_empty() || folder_matches || !item.children.is_empty() {
                children.push(item);
            }
        }
        for request in self
            .requests
            .iter()
            .filter(|request| request.folder_id.as_deref() == parent_id)
        {
            let label = self.request_label(request);
            let searchable = format!(
                "{} {} {} {}",
                label,
                request.protocol.label(),
                request.method,
                request.url
            )
            .to_lowercase();
            if query.is_empty() || searchable.contains(query) {
                children.push(TreeItem::new(request.id.clone(), label));
            }
        }
        children
    }

    fn tree_items(&self, cx: &App) -> Vec<TreeItem> {
        let query = self.search_input.read(cx).value().trim().to_lowercase();
        let children = self.child_tree_items(None, &query);
        let root = TreeItem::new(TREE_ROOT_ID, t!("ApiTest.request_list").to_string())
            .expanded(true)
            .children(if children.is_empty() {
                vec![
                    TreeItem::new(TREE_EMPTY_ID, t!("ApiTest.empty_list").to_string())
                        .disabled(true),
                ]
            } else {
                children
            });
        vec![root]
    }

    fn visible_tree_index(items: &[TreeItem], target_id: &str) -> Option<usize> {
        fn walk(items: &[TreeItem], target_id: &str, index: &mut usize) -> Option<usize> {
            for item in items {
                let current = *index;
                *index += 1;
                if item.id == target_id {
                    return Some(current);
                }
                if item.is_expanded() && !item.children.is_empty() {
                    if let Some(found) = walk(&item.children, target_id, index) {
                        return Some(found);
                    }
                }
            }
            None
        }

        walk(items, target_id, &mut 0)
    }

    fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        let items = self.tree_items(cx);
        let selected_id = self
            .active_folder_id
            .clone()
            .or_else(|| self.active_request_id.clone());
        let selected_ix = selected_id
            .as_deref()
            .and_then(|id| Self::visible_tree_index(&items, id));

        self.tree_state.update(cx, |state, cx| {
            state.set_items(items, cx);
            state.set_selected_index(selected_ix, cx);
        });
    }

    fn create_kv_rows(
        kvs: &[KeyValue],
        key_placeholder: &'static str,
        value_placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<KvRow> {
        kvs.iter()
            .map(|kv| {
                let key = cx.new(|cx| {
                    let mut input = InputState::new(window, cx).placeholder(key_placeholder);
                    input.set_value(kv.key.clone(), window, cx);
                    input
                });
                let value = cx.new(|cx| {
                    let mut input = InputState::new(window, cx).placeholder(value_placeholder);
                    input.set_value(kv.value.clone(), window, cx);
                    input
                });
                KvRow {
                    key,
                    value,
                    enabled: kv.enabled,
                    field_type: kv.field_type,
                    file_path: kv.file_path.clone(),
                }
            })
            .collect()
    }

    fn rows_for_section(&self, section: KvSection) -> &[KvRow] {
        match section {
            KvSection::Params => &self.param_rows,
            KvSection::Path => &self.path_rows,
            KvSection::Headers => &self.header_rows,
            KvSection::Cookies => &self.cookie_rows,
            KvSection::Body => &self.body_rows,
            KvSection::Globals => &self.global_rows,
            KvSection::GlobalParams => &self.global_param_rows,
            KvSection::GlobalHeaders => &self.global_header_rows,
            KvSection::GlobalCookies => &self.global_cookie_rows,
            KvSection::Environment => &self.environment_rows,
            KvSection::EnvironmentParams => &self.environment_param_rows,
            KvSection::EnvironmentHeaders => &self.environment_header_rows,
            KvSection::EnvironmentCookies => &self.environment_cookie_rows,
            KvSection::FolderParams => &self.folder_param_rows,
            KvSection::FolderHeaders => &self.folder_header_rows,
            KvSection::FolderVariables => &self.folder_variable_rows,
            KvSection::RequestVariables => &self.variable_rows,
        }
    }

    fn rows_for_section_mut(&mut self, section: KvSection) -> &mut Vec<KvRow> {
        match section {
            KvSection::Params => &mut self.param_rows,
            KvSection::Path => &mut self.path_rows,
            KvSection::Headers => &mut self.header_rows,
            KvSection::Cookies => &mut self.cookie_rows,
            KvSection::Body => &mut self.body_rows,
            KvSection::Globals => &mut self.global_rows,
            KvSection::GlobalParams => &mut self.global_param_rows,
            KvSection::GlobalHeaders => &mut self.global_header_rows,
            KvSection::GlobalCookies => &mut self.global_cookie_rows,
            KvSection::Environment => &mut self.environment_rows,
            KvSection::EnvironmentParams => &mut self.environment_param_rows,
            KvSection::EnvironmentHeaders => &mut self.environment_header_rows,
            KvSection::EnvironmentCookies => &mut self.environment_cookie_rows,
            KvSection::FolderParams => &mut self.folder_param_rows,
            KvSection::FolderHeaders => &mut self.folder_header_rows,
            KvSection::FolderVariables => &mut self.folder_variable_rows,
            KvSection::RequestVariables => &mut self.variable_rows,
        }
    }

    fn keyvalues_from_rows(&self, section: KvSection, cx: &App) -> Vec<KeyValue> {
        self.rows_for_section(section)
            .iter()
            .map(|row| KeyValue {
                key: row.key.read(cx).value().to_string(),
                value: row.value.read(cx).value().to_string(),
                enabled: row.enabled,
                field_type: row.field_type,
                file_path: row.file_path.clone(),
            })
            .collect()
    }

    fn kv_text(kvs: &[KeyValue]) -> String {
        kvs.iter()
            .filter(|kv| !kv.key.trim().is_empty())
            .map(|kv| format!("{}: {}", kv.key.trim(), kv.value.trim()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn add_kv_row(&mut self, section: KvSection, window: &mut Window, cx: &mut Context<Self>) {
        let mut row = Self::create_kv_rows(&[KeyValue::default()], "Key", "Value", window, cx)
            .pop()
            .expect("one row");
        row.enabled = true;
        self.rows_for_section_mut(section).push(row.clone());
        self.subscribe_row_inputs(std::slice::from_ref(&row), cx);
        self.commit_current_to_store(cx);
        cx.notify();
    }

    fn remove_kv_row(&mut self, section: KvSection, index: usize, cx: &mut Context<Self>) {
        let rows = self.rows_for_section_mut(section);
        if index < rows.len() {
            rows.remove(index);
        }
        self.commit_current_to_store(cx);
        cx.notify();
    }

    fn set_kv_row_enabled(
        &mut self,
        section: KvSection,
        index: usize,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.rows_for_section_mut(section).get_mut(index) {
            row.enabled = enabled;
        }
        self.commit_current_to_store(cx);
        cx.notify();
    }

    fn set_kv_row_field_type(
        &mut self,
        section: KvSection,
        index: usize,
        field_type: crate::http::FieldType,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.rows_for_section_mut(section).get_mut(index) {
            row.field_type = field_type;
            if field_type == crate::http::FieldType::Text {
                row.file_path = None;
            }
        }
        self.commit_current_to_store(cx);
        cx.notify();
    }

    fn choose_kv_file(
        &mut self,
        section: KvSection,
        index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t!("ApiTest.choose_file").to_string().into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = future.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(row) = this.rows_for_section_mut(section).get_mut(index) {
                    row.field_type = crate::http::FieldType::File;
                    row.file_path = Some(path.display().to_string());
                }
                this.commit_current_to_store(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn subscribe_row_inputs(&mut self, rows: &[KvRow], cx: &mut Context<Self>) {
        let entities = rows
            .iter()
            .flat_map(|row| [row.key.clone(), row.value.clone()])
            .collect::<Vec<_>>();
        for input in entities {
            let sub = cx.subscribe(&input, move |this: &mut Self, _src, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change | InputEvent::Blur) {
                    this.commit_current_to_store(cx);
                }
            });
            self._row_subs.push(sub);
        }
    }

    fn current_body_type(&self, cx: &App) -> BodyType {
        self.body_type_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(BodyType::None)
    }

    fn current_raw_language(&self, cx: &App) -> RawLanguage {
        self.raw_lang_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(RawLanguage::Json)
    }

    fn current_auth_type(&self, cx: &App) -> AuthType {
        self.auth_type_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(AuthType::None)
    }

    fn current_auth_target(&self, cx: &App) -> AuthTarget {
        self.auth_target_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(AuthTarget::Header)
    }

    fn snapshot_request(&self, cx: &App) -> StoredRequest {
        let id = self
            .active_request_id
            .clone()
            .unwrap_or_else(|| StoredRequest::new("", RequestMethod::Get).id);
        let existing = self
            .active_request_id
            .as_deref()
            .and_then(|active_id| self.requests.iter().find(|request| request.id == active_id));
        StoredRequest {
            id,
            name: self.name_input.read(cx).value().to_string(),
            description: self.request_description_input.read(cx).value().to_string(),
            method: self.current_method(cx),
            protocol: self.current_protocol(cx),
            url: self.url_input.read(cx).value().to_string(),
            folder_id: existing.and_then(|request| request.folder_id.clone()),
            base_url_override: existing.and_then(|request| request.base_url_override.clone()),
            headers: Self::kv_text(&self.keyvalues_from_rows(KvSection::Headers, cx)),
            params: self.keyvalues_from_rows(KvSection::Params, cx),
            path_vars: self.keyvalues_from_rows(KvSection::Path, cx),
            variables: self.keyvalues_from_rows(KvSection::RequestVariables, cx),
            header_rows: self.keyvalues_from_rows(KvSection::Headers, cx),
            cookies: self.keyvalues_from_rows(KvSection::Cookies, cx),
            body: self.body_input.read(cx).value().to_string(),
            body_type: self.current_body_type(cx),
            raw_language: self.current_raw_language(cx),
            body_rows: self.keyvalues_from_rows(KvSection::Body, cx),
            auth: AuthConfig {
                auth_type: self.current_auth_type(cx),
                token: self.auth_token_input.read(cx).value().to_string(),
                username: self.auth_username_input.read(cx).value().to_string(),
                password: self.auth_password_input.read(cx).value().to_string(),
                key: self.auth_key_input.read(cx).value().to_string(),
                value: self.auth_value_input.read(cx).value().to_string(),
                add_to: self.current_auth_target(cx),
            },
            pre_script: self.pre_script_input.read(cx).value().to_string(),
            tests: self.tests_input.read(cx).value().to_string(),
            mock: existing.and_then(|request| request.mock.clone()),
            last_response: existing.and_then(|request| request.last_response.clone()),
            success_example: existing.and_then(|request| request.success_example.clone()),
            fail_examples: existing
                .map(|request| request.fail_examples.clone())
                .unwrap_or_default(),
        }
    }

    fn save_store(&self) {
        request_store::save_store(&ApiStore {
            folders: self.folders.clone(),
            requests: self.requests.clone(),
            globals: self.globals.clone(),
            global_params: self.global_params.clone(),
            global_headers: self.global_headers.clone(),
            global_cookies: self.global_cookies.clone(),
            environments: self.environments.clone(),
            active_environment_id: self.active_environment_id.clone(),
            history: self.history.clone(),
            response_example_autosave: self.response_example_autosave,
        });
    }

    /// 把当前输入框内容回写进活动目录/请求并保存。
    fn commit_current_to_store(&mut self, cx: &mut Context<Self>) {
        if self.suppress_commit {
            return;
        }
        self.globals = self.keyvalues_from_rows(KvSection::Globals, cx);
        self.global_params = self.keyvalues_from_rows(KvSection::GlobalParams, cx);
        self.global_headers = self.keyvalues_from_rows(KvSection::GlobalHeaders, cx);
        self.global_cookies = self.keyvalues_from_rows(KvSection::GlobalCookies, cx);
        if let Some(active_id) = self.active_environment_id.clone() {
            let base_url = self
                .environment_base_url_input
                .read(cx)
                .value()
                .trim()
                .to_string();
            let params = self.keyvalues_from_rows(KvSection::EnvironmentParams, cx);
            let headers = self.keyvalues_from_rows(KvSection::EnvironmentHeaders, cx);
            let cookies = self.keyvalues_from_rows(KvSection::EnvironmentCookies, cx);
            let variables = self.keyvalues_from_rows(KvSection::Environment, cx);
            if let Some(environment) = self
                .environments
                .iter_mut()
                .find(|environment| environment.id == active_id)
            {
                environment.base_url = (!base_url.is_empty()).then_some(base_url);
                environment.params = params;
                environment.headers = headers;
                environment.cookies = cookies;
                environment.variables = variables;
            }
        }
        let mut tree_label_changed = false;
        if let Some(id) = self.active_folder_id.clone() {
            let name = self.name_input.read(cx).value().trim().to_string();
            let description = self.folder_description_input.read(cx).value().to_string();
            let base_url = self
                .folder_base_url_input
                .read(cx)
                .value()
                .trim()
                .to_string();
            let params = self.keyvalues_from_rows(KvSection::FolderParams, cx);
            let headers = self.keyvalues_from_rows(KvSection::FolderHeaders, cx);
            let variables = self.keyvalues_from_rows(KvSection::FolderVariables, cx);
            if let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == id) {
                tree_label_changed = folder.name != name;
                folder.name = name;
                folder.description = description;
                folder.base_url = (!base_url.is_empty()).then_some(base_url);
                folder.params = params;
                folder.headers = headers;
                folder.variables = variables;
            }
        } else if let Some(id) = self.active_request_id.clone() {
            let snapshot = self.snapshot_request(cx);
            if let Some(req) = self.requests.iter_mut().find(|r| r.id == id) {
                tree_label_changed = req.name != snapshot.name
                    || req.method != snapshot.method
                    || req.protocol != snapshot.protocol;
                *req = snapshot;
            }
        }
        self.save_store();
        if tree_label_changed {
            self.rebuild_tree(cx);
        }
    }

    fn subscribe_editor_inputs(&mut self, cx: &mut Context<Self>) {
        self._row_subs.clear();
        let rows = [
            self.param_rows.as_slice(),
            self.path_rows.as_slice(),
            self.header_rows.as_slice(),
            self.cookie_rows.as_slice(),
            self.body_rows.as_slice(),
            self.global_rows.as_slice(),
            self.global_param_rows.as_slice(),
            self.global_header_rows.as_slice(),
            self.global_cookie_rows.as_slice(),
            self.environment_rows.as_slice(),
            self.environment_param_rows.as_slice(),
            self.environment_header_rows.as_slice(),
            self.environment_cookie_rows.as_slice(),
            self.folder_param_rows.as_slice(),
            self.folder_header_rows.as_slice(),
            self.folder_variable_rows.as_slice(),
            self.variable_rows.as_slice(),
        ]
        .concat();
        self.subscribe_row_inputs(&rows, cx);
    }

    /// 选中请求并把内容载入编辑器。
    fn load_request(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(req) = self.requests.iter().find(|r| r.id == id).cloned() else {
            return;
        };
        self.suppress_commit = true;
        self.active_request_id = Some(req.id.clone());
        self.active_folder_id = None;
        self.protocol_select
            .update(cx, |s, cx| s.set_selected_value(&req.protocol, window, cx));
        self.method_select
            .update(cx, |s, cx| s.set_selected_value(&req.method, window, cx));
        self.body_type_select
            .update(cx, |s, cx| s.set_selected_value(&req.body_type, window, cx));
        self.raw_lang_select.update(cx, |s, cx| {
            s.set_selected_value(&req.raw_language, window, cx)
        });
        self.auth_type_select.update(cx, |s, cx| {
            s.set_selected_value(&req.auth.auth_type, window, cx)
        });
        self.auth_target_select.update(cx, |s, cx| {
            s.set_selected_value(&req.auth.add_to, window, cx)
        });
        self.name_input
            .update(cx, |s, cx| s.set_value(req.name.clone(), window, cx));
        self.url_input
            .update(cx, |s, cx| s.set_value(req.url.clone(), window, cx));
        self.request_description_input
            .update(cx, |s, cx| s.set_value(req.description.clone(), window, cx));
        self.body_input
            .update(cx, |s, cx| s.set_value(req.body.clone(), window, cx));
        self.pre_script_input
            .update(cx, |s, cx| s.set_value(req.pre_script.clone(), window, cx));
        self.tests_input
            .update(cx, |s, cx| s.set_value(req.tests.clone(), window, cx));
        self.auth_token_input
            .update(cx, |s, cx| s.set_value(req.auth.token.clone(), window, cx));
        self.auth_username_input.update(cx, |s, cx| {
            s.set_value(req.auth.username.clone(), window, cx)
        });
        self.auth_password_input.update(cx, |s, cx| {
            s.set_value(req.auth.password.clone(), window, cx)
        });
        self.auth_key_input
            .update(cx, |s, cx| s.set_value(req.auth.key.clone(), window, cx));
        self.auth_value_input
            .update(cx, |s, cx| s.set_value(req.auth.value.clone(), window, cx));
        self.folder_description_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.folder_base_url_input
            .update(cx, |s, cx| s.set_value("", window, cx));

        let header_rows = if req.header_rows.is_empty() && !req.headers.trim().is_empty() {
            Self::parse_headers(&req.headers).unwrap_or_default()
        } else {
            req.header_rows.clone()
        };
        self.param_rows = Self::create_kv_rows(&req.params, "Key", "Value", window, cx);
        self.path_rows = Self::create_kv_rows(&req.path_vars, "Key", "Value", window, cx);
        self.variable_rows = Self::create_kv_rows(&req.variables, "Key", "Value", window, cx);
        self.header_rows = Self::create_kv_rows(&header_rows, "Header", "Value", window, cx);
        self.cookie_rows = Self::create_kv_rows(&req.cookies, "Cookie", "Value", window, cx);
        self.body_rows = Self::create_kv_rows(&req.body_rows, "Key", "Value", window, cx);
        self.folder_param_rows.clear();
        self.folder_header_rows.clear();
        self.folder_variable_rows.clear();

        self.request_generation = self.request_generation.wrapping_add(1);
        self.cancel_stream();
        self.cancel_websocket();
        self.cancel_socket_io();
        self.cancel_tcp();
        self.sending = false;
        self.response = req.last_response.clone();
        self.prepared_request = None;
        self.pre_result = None;
        self.test_result = None;
        self.notice = None;
        self.subscribe_editor_inputs(cx);
        self.suppress_commit = false;
        self.rebuild_tree(cx);
        cx.notify();
    }

    fn current_parent_folder_id(&self) -> Option<String> {
        self.active_folder_id.clone().or_else(|| {
            self.active_request_id.as_deref().and_then(|active_id| {
                self.requests
                    .iter()
                    .find(|request| request.id == active_id)
                    .and_then(|request| request.folder_id.clone())
            })
        })
    }

    /// 把选中的目录载入编辑器。
    fn load_folder(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(folder) = self.folders.iter().find(|folder| folder.id == id).cloned() else {
            return;
        };
        self.suppress_commit = true;
        self.active_request_id = None;
        self.active_folder_id = Some(folder.id.clone());
        self.request_description_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.name_input
            .update(cx, |s, cx| s.set_value(folder.name, window, cx));
        self.folder_description_input
            .update(cx, |s, cx| s.set_value(folder.description, window, cx));
        self.folder_base_url_input.update(cx, |s, cx| {
            s.set_value(folder.base_url.unwrap_or_default(), window, cx)
        });
        self.folder_param_rows = Self::create_kv_rows(&folder.params, "Key", "Value", window, cx);
        self.folder_header_rows =
            Self::create_kv_rows(&folder.headers, "Header", "Value", window, cx);
        self.folder_variable_rows =
            Self::create_kv_rows(&folder.variables, "Key", "Value", window, cx);
        self.request_generation = self.request_generation.wrapping_add(1);
        self.cancel_stream();
        self.cancel_websocket();
        self.cancel_socket_io();
        self.cancel_tcp();
        self.sending = false;
        self.response = None;
        self.prepared_request = None;
        self.pre_result = None;
        self.test_result = None;
        self.notice = None;
        self.subscribe_editor_inputs(cx);
        self.suppress_commit = false;
        self.rebuild_tree(cx);
        cx.notify();
    }

    /// 新建一个目录并选中。
    fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_current_to_store(cx);
        let parent_id = self.current_parent_folder_id();
        let index = self.folders.len() + 1;
        let folder =
            StoredFolder::new(format!("{} {}", t!("ApiTest.new_folder"), index), parent_id);
        let folder_id = folder.id.clone();
        self.folders.push(folder);
        self.save_store();
        self.load_folder(&folder_id, window, cx);
    }

    fn delete_folder(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let removed_set = descendant_folder_ids(&self.folders, id);
        self.folders
            .retain(|folder| !removed_set.contains(&folder.id));
        self.requests.retain(|request| {
            request
                .folder_id
                .as_ref()
                .map(|folder_id| !removed_set.contains(folder_id))
                .unwrap_or(true)
        });
        let active_request_removed = self
            .active_request_id
            .as_deref()
            .is_some_and(|active_id| !self.requests.iter().any(|request| request.id == active_id));
        let active_folder_removed = self
            .active_folder_id
            .as_ref()
            .is_some_and(|folder_id| removed_set.contains(folder_id));
        if active_folder_removed {
            self.active_folder_id = None;
        }
        self.save_store();
        if active_request_removed || active_folder_removed {
            self.active_request_id = None;
            if let Some(next_id) = self.requests.first().map(|request| request.id.clone()) {
                self.load_request(&next_id, window, cx);
            } else {
                self.request_generation = self.request_generation.wrapping_add(1);
                self.cancel_stream();
                self.cancel_websocket();
                self.cancel_socket_io();
                self.cancel_tcp();
                self.sending = false;
                self.response = None;
                self.prepared_request = None;
                self.pre_result = None;
                self.test_result = None;
                self.notice = None;
                self.folder_description_input
                    .update(cx, |s, cx| s.set_value("", window, cx));
                self.folder_param_rows.clear();
                self.folder_header_rows.clear();
                self.folder_variable_rows.clear();
                self.folder_base_url_input
                    .update(cx, |s, cx| s.set_value("", window, cx));
                self.name_input
                    .update(cx, |s, cx| s.set_value("", window, cx));
                self.subscribe_editor_inputs(cx);
                self.rebuild_tree(cx);
                cx.notify();
            }
        } else {
            self.rebuild_tree(cx);
            cx.notify();
        }
    }

    /// 新建指定协议的请求并选中。
    fn new_request_with_protocol(
        &mut self,
        protocol: Protocol,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_current_to_store(cx);
        let index = self.requests.len() + 1;
        let mut req = StoredRequest::new(
            format!("{} {}", t!("ApiTest.new_request"), index),
            RequestMethod::Get,
        );
        req.protocol = protocol;
        req.folder_id = self.current_parent_folder_id();
        let id = req.id.clone();
        self.requests.push(req);
        self.save_store();
        self.load_request(&id, window, cx);
    }

    /// 删除请求；删除活动请求后载入相邻请求。
    fn delete_request(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.requests.iter().position(|r| r.id == id) else {
            return;
        };
        let was_active = self.active_request_id.as_deref() == Some(id);
        self.requests.remove(index);
        self.save_store();
        if !was_active {
            self.rebuild_tree(cx);
            cx.notify();
            return;
        }
        let next_id = self
            .requests
            .get(index)
            .or_else(|| index.checked_sub(1).and_then(|i| self.requests.get(i)))
            .map(|r| r.id.clone());
        match next_id {
            Some(id) => self.load_request(&id, window, cx),
            None => {
                self.active_request_id = None;
                self.request_generation = self.request_generation.wrapping_add(1);
                self.cancel_stream();
                self.cancel_websocket();
                self.cancel_socket_io();
                self.cancel_tcp();
                self.sending = false;
                self.response = None;
                self.prepared_request = None;
                self.pre_result = None;
                self.test_result = None;
                self.rebuild_tree(cx);
                cx.notify();
            }
        }
    }

    /// 树节点点击：先回写当前请求，再载入目标请求。
    fn select_request(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_request_id.as_deref() == Some(id) {
            return;
        }
        self.commit_current_to_store(cx);
        self.load_request(id, window, cx);
    }

    fn select_folder(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.folders.iter().any(|folder| folder.id == id) {
            return;
        }
        if self.active_folder_id.as_deref() == Some(id) {
            return;
        }
        self.commit_current_to_store(cx);
        self.load_folder(id, window, cx);
    }

    fn restore_history(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut request) = self
            .history
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.request.clone())
        else {
            return;
        };
        self.commit_current_to_store(cx);
        request.id = uuid::Uuid::new_v4().simple().to_string();
        request.folder_id = None;
        request.name = format!(
            "{} ({})",
            self.request_label(&request),
            t!("ApiTest.history")
        );
        let request_id = request.id.clone();
        self.requests.push(request);
        self.sidebar_mode = SidebarMode::Collections;
        self.save_store();
        self.load_request(&request_id, window, cx);
        self.notice = Some(t!("ApiTest.restored_from_history").to_string());
        cx.notify();
    }

    fn clear_history(&mut self, cx: &mut Context<Self>) {
        self.history.clear();
        self.save_store();
        cx.notify();
    }

    fn effective_vars(&self, request: &StoredRequest) -> BTreeMap<String, String> {
        let environment = self
            .active_environment()
            .map(|environment| environment.variables.as_slice())
            .unwrap_or_default();
        let folder_scopes = ancestor_folder_ids(&self.folders, request.folder_id.as_deref())
            .into_iter()
            .filter_map(|folder_id| {
                self.folders
                    .iter()
                    .find(|folder| folder.id == folder_id)
                    .map(|folder| folder.variables.as_slice())
            })
            .collect::<Vec<_>>();
        merge_variable_scopes(
            &self.globals,
            environment,
            &folder_scopes,
            &request.path_vars,
            &request.variables,
        )
    }

    fn active_environment(&self) -> Option<&ApiEnvironment> {
        let active_id = self.active_environment_id.as_deref()?;
        self.environments
            .iter()
            .find(|environment| environment.id == active_id)
    }

    fn persist_environment_effects(&mut self, result: &ScriptResult) -> bool {
        let Some(active_id) = self.active_environment_id.as_deref() else {
            return false;
        };
        let Some(environment) = self
            .environments
            .iter_mut()
            .find(|environment| environment.id == active_id)
        else {
            return false;
        };
        apply_environment_effects(&mut environment.variables, result)
    }

    fn refresh_environment_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let environment = self.active_environment().cloned();
        self.suppress_commit = true;
        self.environment_base_url_input.update(cx, |input, cx| {
            input.set_value(
                environment
                    .as_ref()
                    .and_then(|environment| environment.base_url.clone())
                    .unwrap_or_default(),
                window,
                cx,
            );
        });
        self.environment_param_rows = Self::create_kv_rows(
            environment
                .as_ref()
                .map(|environment| environment.params.as_slice())
                .unwrap_or_default(),
            "Key",
            "Value",
            window,
            cx,
        );
        self.environment_header_rows = Self::create_kv_rows(
            environment
                .as_ref()
                .map(|environment| environment.headers.as_slice())
                .unwrap_or_default(),
            "Header",
            "Value",
            window,
            cx,
        );
        self.environment_cookie_rows = Self::create_kv_rows(
            environment
                .as_ref()
                .map(|environment| environment.cookies.as_slice())
                .unwrap_or_default(),
            "Cookie",
            "Value",
            window,
            cx,
        );
        let variables = environment
            .as_ref()
            .map(|environment| environment.variables.as_slice())
            .unwrap_or_default();
        self.environment_rows = Self::create_kv_rows(&variables, "Key", "Value", window, cx);
        self.subscribe_editor_inputs(cx);
        self.suppress_commit = false;
    }

    fn environment_options(&self) -> Vec<EnvironmentOption> {
        self.environments
            .iter()
            .map(|environment| EnvironmentOption {
                id: environment.id.clone(),
                name: environment.name.clone(),
            })
            .collect()
    }

    fn rebuild_environment_select(&self, window: &mut Window, cx: &mut Context<Self>) {
        let options = self.environment_options();
        let active_id = self.active_environment_id.clone();
        self.variables_environment_select.update(cx, |state, cx| {
            state.set_items(options, window, cx);
            if let Some(active_id) = active_id {
                state.set_selected_value(&active_id, window, cx);
            }
        });
    }

    fn select_environment(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_environment_id.as_deref() == Some(id) {
            return;
        }
        self.commit_current_to_store(cx);
        if !self
            .environments
            .iter()
            .any(|environment| environment.id == id)
        {
            return;
        }
        self.active_environment_id = Some(id.to_string());
        self.rebuild_environment_select(window, cx);
        self.refresh_environment_settings(window, cx);
        self.save_store();
        cx.notify();
    }

    fn create_environment_named(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_current_to_store(cx);
        let environment = ApiEnvironment::new(name);
        self.active_environment_id = Some(environment.id.clone());
        self.environments.push(environment);
        self.rebuild_environment_select(window, cx);
        self.refresh_environment_settings(window, cx);
        self.save_store();
        cx.notify();
    }

    fn rename_active_environment_named(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_current_to_store(cx);
        let Some(active_id) = self.active_environment_id.as_deref() else {
            return;
        };
        let Some(environment) = self
            .environments
            .iter_mut()
            .find(|environment| environment.id == active_id)
        else {
            return;
        };
        environment.name = name;
        self.rebuild_environment_select(window, cx);
        self.save_store();
        cx.notify();
    }

    fn prompt_new_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("ApiTest.environment_name_placeholder").to_string())
        });
        let input_for_focus = input.clone();
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_for_ok = input.clone();
            let view_for_ok = view.clone();
            dialog
                .title(t!("ApiTest.new_environment").to_string())
                .w(px(400.))
                .confirm()
                .on_ok(move |_, window, cx| {
                    let name = input_for_ok.read(cx).value().trim().to_owned();
                    if name.is_empty() {
                        let message = t!("ApiTest.environment_name_required").to_string();
                        view_for_ok.update(cx, |view, cx| {
                            view.notice = Some(message);
                            cx.notify();
                        });
                        return false;
                    }
                    view_for_ok.update(cx, |view, cx| {
                        view.notice = None;
                        view.create_environment_named(name, window, cx);
                    });
                    true
                })
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            div()
                                .text_sm()
                                .child(t!("ApiTest.environment_name").to_string()),
                        )
                        .child(Input::new(&input).w_full()),
                )
        });
        window.defer(cx, move |window, cx| {
            input_for_focus.update(cx, |input, cx| input.focus(window, cx));
        });
    }

    fn prompt_rename_active_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current_name) = self
            .active_environment()
            .map(|environment| environment.name.clone())
        else {
            return;
        };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(current_name)
                .placeholder(t!("ApiTest.environment_name_placeholder").to_string())
        });
        let input_for_focus = input.clone();
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_for_ok = input.clone();
            let view_for_ok = view.clone();
            dialog
                .title(t!("ApiTest.rename_environment").to_string())
                .w(px(400.))
                .confirm()
                .on_ok(move |_, window, cx| {
                    let name = input_for_ok.read(cx).value().trim().to_owned();
                    if name.is_empty() {
                        let message = t!("ApiTest.environment_name_required").to_string();
                        view_for_ok.update(cx, |view, cx| {
                            view.notice = Some(message);
                            cx.notify();
                        });
                        return false;
                    }
                    view_for_ok.update(cx, |view, cx| {
                        view.notice = None;
                        view.rename_active_environment_named(name, window, cx);
                    });
                    true
                })
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            div()
                                .text_sm()
                                .child(t!("ApiTest.environment_name").to_string()),
                        )
                        .child(Input::new(&input).w_full()),
                )
        });
        window.defer(cx, move |window, cx| {
            input_for_focus.update(cx, |input, cx| input.focus(window, cx));
        });
    }

    fn delete_active_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.environments.len() <= 1 {
            self.notice = Some(t!("ApiTest.cannot_delete_last_environment").to_string());
            cx.notify();
            return;
        }
        self.commit_current_to_store(cx);
        let Some(active_id) = self.active_environment_id.clone() else {
            return;
        };
        self.environments
            .retain(|environment| environment.id != active_id);
        self.active_environment_id = self
            .environments
            .first()
            .map(|environment| environment.id.clone());
        self.rebuild_environment_select(window, cx);
        self.refresh_environment_settings(window, cx);
        self.save_store();
        cx.notify();
    }

    fn import_collection_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t!("ApiTest.import_collection").to_string().into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = future.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let result = cx
                .background_spawn(async move {
                    let text = std::fs::read_to_string(path)?;
                    crate::schema_io::import_collection(&text)
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(collection) => this.merge_imported_collection(collection, window, cx),
                Err(error) => {
                    this.notice =
                        Some(t!("ApiTest.import_failed", error = error.to_string()).to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn merge_imported_collection(
        &mut self,
        mut imported: crate::collection_io::ImportedCollection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_current_to_store(cx);
        let root = StoredFolder::new(imported.name.clone(), None);
        for folder in &mut imported.folders {
            if folder.parent_id.is_none() {
                folder.parent_id = Some(root.id.clone());
            }
        }
        for request in &mut imported.requests {
            if request.folder_id.is_none() {
                request.folder_id = Some(root.id.clone());
            }
        }
        let request_count = imported.requests.len();
        let first_request_id = imported.requests.first().map(|request| request.id.clone());
        self.folders.push(root);
        self.folders.append(&mut imported.folders);
        self.requests.append(&mut imported.requests);
        if let Some(environment) = imported.environment {
            self.active_environment_id = Some(environment.id.clone());
            self.environments.push(environment);
            self.rebuild_environment_select(window, cx);
            self.refresh_environment_settings(window, cx);
        }
        self.save_store();
        self.rebuild_tree(cx);
        if let Some(request_id) = first_request_id {
            self.load_request(&request_id, window, cx);
        }
        self.notice = Some(t!("ApiTest.import_success", count = request_count).to_string());
        cx.notify();
    }

    fn store_snapshot(&mut self, cx: &mut Context<Self>) -> ApiStore {
        self.commit_current_to_store(cx);
        ApiStore {
            folders: self.folders.clone(),
            requests: self.requests.clone(),
            globals: self.globals.clone(),
            global_params: self.global_params.clone(),
            global_headers: self.global_headers.clone(),
            global_cookies: self.global_cookies.clone(),
            environments: self.environments.clone(),
            active_environment_id: self.active_environment_id.clone(),
            history: self.history.clone(),
            response_example_autosave: self.response_example_autosave,
        }
    }

    fn export_collection(&mut self, kind: CollectionExport, cx: &mut Context<Self>) {
        let store = self.store_snapshot(cx);
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(kind.prompt().into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = future.await else {
                return;
            };
            let Some(directory) = paths.into_iter().next() else {
                return;
            };
            let output_path = directory.join(kind.file_name());
            let path_for_write = output_path.clone();
            let result = cx
                .background_spawn(async move {
                    std::fs::write(path_for_write, kind.serialize(&store)?)?;
                    Ok::<(), anyhow::Error>(())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.notice = Some(match result {
                    Ok(()) => t!(
                        "ApiTest.export_success",
                        path = output_path.display().to_string()
                    )
                    .to_string(),
                    Err(error) => {
                        t!("ApiTest.export_failed", error = error.to_string()).to_string()
                    }
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn render_export_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        Button::new("api-export-collection")
            .ghost()
            .small()
            .w_full()
            .justify_start()
            .icon(IconName::Export)
            .label(t!("ApiTest.export").to_string())
            .tooltip(t!("ApiTest.export").to_string())
            .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, _| {
                for kind in CollectionExport::ALL {
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(kind.menu_label())
                            .icon(IconName::Export)
                            .on_click(move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.export_collection(kind, cx);
                                });
                            }),
                    );
                }
                menu
            })
    }

    fn protocol_icon(protocol: Protocol) -> IconName {
        match protocol {
            Protocol::Http => IconName::Globe,
            Protocol::Graphql => IconName::Query,
            Protocol::Sse => IconName::Sync,
            Protocol::WebSocket => IconName::Network,
            Protocol::Tcp => IconName::Terminal,
            Protocol::GrpcWeb => IconName::Server,
            Protocol::SocketIo => IconName::Network,
        }
    }

    fn render_new_request_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        Button::new("api-new-request")
            .primary()
            .small()
            .flex_shrink_0()
            .icon(IconName::Plus)
            .tooltip(t!("ApiTest.new_request").to_string())
            .dropdown_menu(move |mut menu, window, _| {
                for protocol in Protocol::ALL {
                    let view = view.clone();
                    let protocol = *protocol;
                    menu = menu.item(
                        PopupMenuItem::new(protocol.label())
                            .icon(Self::protocol_icon(protocol))
                            .on_click(window.listener_for(&view, move |this, _, window, cx| {
                                this.new_request_with_protocol(protocol, window, cx);
                            })),
                    );
                }
                menu
            })
    }

    fn format_size(size: u64) -> String {
        if size >= 1024 * 1024 {
            format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
        } else if size >= 1024 {
            format!("{:.1} KB", size as f64 / 1024.0)
        } else {
            format!("{size} B")
        }
    }
}

fn insert_enabled_variables(vars: &mut BTreeMap<String, String>, rows: &[KeyValue]) {
    for row in rows
        .iter()
        .filter(|row| row.enabled && !row.key.trim().is_empty())
    {
        vars.insert(row.key.trim().to_string(), row.value.clone());
    }
}

fn merge_variable_scopes(
    globals: &[KeyValue],
    environment: &[KeyValue],
    folders: &[&[KeyValue]],
    path: &[KeyValue],
    request: &[KeyValue],
) -> BTreeMap<String, String> {
    let mut vars = BTreeMap::new();
    insert_enabled_variables(&mut vars, globals);
    insert_enabled_variables(&mut vars, environment);
    for folder in folders {
        insert_enabled_variables(&mut vars, folder);
    }
    insert_enabled_variables(&mut vars, path);
    insert_enabled_variables(&mut vars, request);
    vars
}

fn apply_environment_effects(variables: &mut Vec<KeyValue>, result: &ScriptResult) -> bool {
    let mut changed = false;
    for effect in &result.effects {
        let scripting::SideEffect::SetVariable {
            scope: scripting::VarScope::Environment,
            name,
            value,
        } = effect
        else {
            continue;
        };
        if let Some(variable) = variables.iter_mut().find(|variable| variable.key == *name) {
            if variable.value != *value || !variable.enabled {
                variable.value = value.clone();
                variable.enabled = true;
                changed = true;
            }
        } else {
            variables.push(KeyValue::new(name.clone(), value.clone()));
            changed = true;
        }
    }
    changed
}

impl Render for ApiTestView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let protocol = self.current_protocol(cx);
        let method = self.current_method(cx);
        let request_meta_bar = self.render_request_meta_bar(cx);
        let request_bar = self.active_folder_id.is_none().then(|| {
            self.render_request_bar(protocol, method, cx)
                .into_any_element()
        });
        let notice = self.notice.clone().map(|notice| {
            h_flex()
                .flex_shrink_0()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .border_b_1()
                .border_color(theme.danger.opacity(0.25))
                .bg(theme.danger.opacity(0.08))
                .text_sm()
                .text_color(theme.danger)
                .child(Icon::new(IconName::TriangleAlert).small())
                .child(notice)
                .into_any_element()
        });

        let tree_sidebar = self.render_sidebar(cx);
        let editor_pane = self.render_editor_pane(cx);
        let request_panes = if self.active_folder_id.is_some() {
            div()
                .size_full()
                .min_h_0()
                .min_w_0()
                .child(editor_pane)
                .into_any_element()
        } else {
            let response_pane = self.render_response(cx);
            v_resizable("api-test-vertical")
                .child(
                    resizable_panel()
                        .size(px(340.))
                        .size_range(px(180.)..px(900.))
                        .child(editor_pane),
                )
                .child(
                    resizable_panel()
                        .size_range(px(180.)..Pixels::MAX)
                        .child(response_pane),
                )
                .into_any_element()
        };
        let request_workspace = v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .bg(theme.background)
            .child(request_meta_bar)
            .when_some(request_bar, |workspace, bar| workspace.child(bar))
            .when_some(notice, |workspace, notice| workspace.child(notice))
            .child(div().flex_1().min_h_0().min_w_0().child(request_panes));
        let request_content = if self.sidebar_collapsed {
            h_flex()
                .size_full()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .child(
                    v_flex()
                        .id("api-request-sidebar-collapsed")
                        .relative()
                        .w(px(REQUEST_TREE_COLLAPSED_WIDTH))
                        .h_full()
                        .flex_shrink_0()
                        .border_r_1()
                        .border_color(theme.sidebar_border)
                        .bg(theme.sidebar)
                        .child(self.render_sidebar_toggle_handle(true, cx)),
                )
                .child(request_workspace)
                .into_any_element()
        } else {
            h_resizable("api-test-horizontal")
                .child(
                    resizable_panel()
                        .size(px(REQUEST_TREE_WIDTH))
                        .size_range(px(200.)..px(480.))
                        .flex_none()
                        .child(tree_sidebar),
                )
                .child(resizable_panel().child(request_workspace))
                .into_any_element()
        };

        v_flex()
            .id("api-test-root")
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .bg(theme.background)
            .child(request_content)
    }
}

impl ApiTestView {
    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    fn render_sidebar_toggle_handle(
        &self,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = cx.theme().clone();

        div()
            .id("api-sidebar-toggle")
            .absolute()
            .right(px(5.))
            .top_0()
            .bottom_0()
            .w(px(18.))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("api-sidebar-toggle-button")
                    .w(px(18.))
                    .h(px(52.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(9.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .shadow_sm()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.muted))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.toggle_sidebar(cx);
                        }),
                    )
                    .child(
                        Icon::new(if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronLeft
                        })
                        .xsmall()
                        .text_color(theme.muted_foreground),
                    ),
            )
    }

    fn render_request_meta_bar(&self, cx: &mut Context<Self>) -> PanelHeader {
        let theme = cx.theme().clone();
        let actions = h_flex()
            .items_center()
            .gap_1()
            .child(self.render_environment_manager(cx));

        PanelHeader::new("api-request-meta-bar")
            .variant(PanelHeaderVariant::Toolbar)
            .background(theme.background)
            .border_bottom(true)
            .title(
                h_flex()
                    .w(px(320.))
                    .flex_shrink_0()
                    .min_w_0()
                    .gap_2()
                    .child(
                        Icon::new(if self.active_folder_id.is_some() {
                            IconName::FolderOpen
                        } else {
                            IconName::File
                        })
                        .small(),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.name_input).small().w_full()),
                    ),
            )
            .trailing(actions)
    }

    fn render_environment_manager(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let active_id = self.active_environment_id.clone();
        let active_name = self
            .active_environment()
            .map(|environment| environment.name.clone())
            .unwrap_or_else(|| t!("ApiTest.environment").to_string());

        let switcher_theme = theme.clone();
        let switcher_environments = self.environments.clone();
        let switcher_active_id = active_id.clone();
        let switcher_view = cx.entity();
        // Popover 内容闭包要求 'static,提前克隆滚动句柄
        let switcher_scroll_handle = self.environment_switcher_scroll_handle.clone();
        let switcher_trigger = Button::new("api-environment-switcher-trigger")
            .outline()
            .small()
            .w(px(176.))
            .flex_shrink_0()
            .justify_start()
            .icon(IconName::Globe)
            .label(active_name.clone())
            .dropdown_caret(true)
            .tooltip(t!("ApiTest.environment").to_string());
        let environment_switcher = Popover::new("api-environment-switcher")
            .anchor(Anchor::TopRight)
            .p_0()
            .trigger(switcher_trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity();
                let environment_options =
                    switcher_environments
                        .iter()
                        .enumerate()
                        .map(|(index, environment)| {
                            let environment_id = environment.id.clone();
                            let is_active =
                                switcher_active_id.as_deref() == Some(environment.id.as_str());
                            let view = switcher_view.clone();
                            let popover = popover.clone();

                            h_flex()
                                .id(format!("api-environment-switch-option-{index}"))
                                .w_full()
                                .h(px(38.))
                                .flex_shrink_0()
                                .items_center()
                                .gap_2()
                                .px_2()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .text_color(switcher_theme.popover_foreground)
                                .when(is_active, |row| {
                                    row.bg(switcher_theme.list_active)
                                        .border_1()
                                        .border_color(switcher_theme.list_active_border)
                                })
                                .hover(|style| style.bg(switcher_theme.list_hover))
                                .on_click(move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.select_environment(&environment_id, window, cx);
                                    });
                                    popover.update(cx, |state, cx| {
                                        state.dismiss(window, cx);
                                    });
                                })
                                .child(Icon::new(IconName::Globe).xsmall().text_color(
                                    if is_active {
                                        switcher_theme.primary
                                    } else {
                                        switcher_theme.muted_foreground
                                    },
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .font_weight(if is_active {
                                            FontWeight::SEMIBOLD
                                        } else {
                                            FontWeight::NORMAL
                                        })
                                        .text_color(switcher_theme.popover_foreground)
                                        .child(environment.name.clone()),
                                )
                                .when(is_active, |row| {
                                    row.child(
                                        Icon::new(IconName::Check)
                                            .xsmall()
                                            .flex_shrink_0()
                                            .text_color(switcher_theme.primary),
                                    )
                                })
                        })
                        .collect::<Vec<_>>();
                let new_environment_view = switcher_view.clone();
                let new_environment_popover = popover.clone();

                v_flex()
                    .id("api-environment-switcher-content")
                    .w(px(288.))
                    .min_h_0()
                    .overflow_hidden()
                    .bg(switcher_theme.popover)
                    .text_color(switcher_theme.popover_foreground)
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .border_b_1()
                            .border_color(switcher_theme.border)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(switcher_theme.muted_foreground)
                                    .child(t!("ApiTest.environments").to_string()),
                            )
                            .child(
                                Tag::secondary()
                                    .small()
                                    .child(switcher_environments.len().to_string()),
                            ),
                    )
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .max_h(px(280.))
                            .child(
                                div()
                                    .id("api-environment-switcher-scroll")
                                    .w_full()
                                    .max_h(px(280.))
                                    .overflow_y_scroll()
                                    .track_scroll(&switcher_scroll_handle)
                                    .p_1()
                                    .child(
                                        v_flex().w_full().gap_0p5().children(environment_options),
                                    ),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .right_0()
                                    .bottom_0()
                                    .w(px(12.))
                                    .child(
                                        Scrollbar::vertical(&switcher_scroll_handle)
                                            .scrollbar_show(ScrollbarShow::Always),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .p_1()
                            .border_t_1()
                            .border_color(switcher_theme.border)
                            .child(
                                Button::new("api-new-environment-quick")
                                    .ghost()
                                    .small()
                                    .w_full()
                                    .justify_start()
                                    .icon(IconName::Plus)
                                    .label(t!("ApiTest.new_environment").to_string())
                                    .on_click(move |_, window, cx| {
                                        new_environment_popover.update(cx, |state, cx| {
                                            state.dismiss(window, cx);
                                        });
                                        new_environment_view.update(cx, |this, cx| {
                                            this.prompt_new_environment(window, cx);
                                        });
                                    }),
                            ),
                    )
            });

        let manager_trigger = Button::new("api-environment-manager-trigger")
            .outline()
            .small()
            .w(px(32.))
            .px_0()
            .flex_shrink_0()
            .justify_center()
            .icon(IconName::Settings2)
            .tooltip(t!("ApiTest.manage_environments").to_string());
        let manager_view = cx.entity();
        let manager_trigger = manager_trigger.on_click(move |_, window, cx| {
            let manager_view = manager_view.clone();
            window.open_dialog(cx, move |dialog, _, cx| {
                let manager_content =
                    manager_view.update(cx, |this, cx| this.render_environment_manager_dialog(cx));
                dialog
                    .w(px(1000.))
                    .h(px(680.))
                    .p_0()
                    .close_button(false)
                    .overlay(true)
                    .overlay_closable(true)
                    .child(manager_content)
            });
        });

        div()
            .id("api-environment-select")
            .flex_shrink_0()
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(environment_switcher)
                    .child(manager_trigger),
            )
            .into_any_element()
    }

    fn render_environment_manager_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let active_id = self.active_environment_id.clone();
        let active_name = self
            .active_environment()
            .map(|environment| environment.name.clone())
            .unwrap_or_else(|| t!("ApiTest.environment").to_string());
        let environment_count = self.environments.len();
        let manager_environment_options =
            self.environments
                .iter()
                .enumerate()
                .map(|(index, environment)| {
                    let environment_id = environment.id.clone();
                    let is_active = active_id.as_deref() == Some(environment.id.as_str());
                    h_flex()
                        .id(format!("api-environment-option-{index}"))
                        .w_full()
                        .h(px(42.))
                        .flex_shrink_0()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .rounded(px(7.))
                        .cursor_pointer()
                        .text_color(theme.sidebar_foreground)
                        .when(is_active, |row| {
                            row.bg(theme.sidebar_accent)
                                .text_color(theme.sidebar_accent_foreground)
                        })
                        .hover(|style| {
                            style
                                .bg(theme.sidebar_accent)
                                .text_color(theme.sidebar_accent_foreground)
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_environment(&environment_id, window, cx);
                        }))
                        .child(
                            div()
                                .size(px(26.))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(7.))
                                .bg(if is_active {
                                    theme.sidebar_primary
                                } else {
                                    theme.muted
                                })
                                .child(Icon::new(IconName::Globe).xsmall().text_color(
                                    if is_active {
                                        theme.sidebar_primary_foreground
                                    } else {
                                        theme.muted_foreground
                                    },
                                )),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .font_weight(if is_active {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .child(environment.name.clone()),
                        )
                        .when(is_active, |row| {
                            row.child(
                                Icon::new(IconName::Check)
                                    .xsmall()
                                    .flex_shrink_0()
                                    .text_color(theme.sidebar_accent_foreground),
                            )
                        })
                })
                .collect::<Vec<_>>();
        let mut settings_section =
            |id: &'static str, title: String, hint: String, section: KvSection| {
                v_flex()
                    .id(id)
                    .w_full()
                    .min_w_0()
                    .gap_3()
                    .p_4()
                    .bg(theme.popover)
                    .border_1()
                    .border_color(theme.border)
                    .rounded_lg()
                    .child(
                        h_flex()
                            .w_full()
                            .flex_shrink_0()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .size(px(7.))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .bg(theme.primary),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.popover_foreground)
                                    .child(title),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(px(220.))
                            .min_h_0()
                            .child(self.render_kv_editor(section, cx)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(hint),
                    )
            };
        let environment_variables_section = settings_section(
            "api-environment-variables",
            t!("ApiTest.environment_variables").to_string(),
            t!("ApiTest.environment_variables_hint").to_string(),
            KvSection::Environment,
        );
        let environment_headers_section = settings_section(
            "api-environment-headers",
            t!("ApiTest.environment_headers").to_string(),
            t!("ApiTest.environment_headers_hint").to_string(),
            KvSection::EnvironmentHeaders,
        );
        let environment_params_section = settings_section(
            "api-environment-params",
            t!("ApiTest.environment_params").to_string(),
            t!("ApiTest.environment_params_hint").to_string(),
            KvSection::EnvironmentParams,
        );
        let environment_cookies_section = settings_section(
            "api-environment-cookies",
            t!("ApiTest.environment_cookies").to_string(),
            t!("ApiTest.environment_cookies_hint").to_string(),
            KvSection::EnvironmentCookies,
        );
        drop(settings_section);

        h_flex()
            .id("api-environment-manager-content")
            .w_full()
            .h_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .child(
                v_flex()
                    .w(px(232.))
                    .h_full()
                    .flex_shrink_0()
                    .min_h_0()
                    .border_r_1()
                    .border_color(theme.sidebar_border)
                    .bg(theme.sidebar)
                    .child(
                        v_flex()
                            .w_full()
                            .flex_shrink_0()
                            .gap_0p5()
                            .p_3()
                            .border_b_1()
                            .border_color(theme.sidebar_border)
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.sidebar_foreground)
                                            .child(t!("ApiTest.manage_environments").to_string()),
                                    )
                                    .child(
                                        Tag::secondary()
                                            .small()
                                            .child(environment_count.to_string()),
                                    ),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(t!("ApiTest.environments").to_string()),
                            ),
                    )
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .flex_1()
                            .min_h_0()
                            .child(
                                div()
                                    .id("api-environment-list-scroll")
                                    .size_full()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.environment_list_scroll_handle)
                                    .p_2()
                                    .child(
                                        v_flex()
                                            .w_full()
                                            .gap_1()
                                            .children(manager_environment_options),
                                    ),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .right_0()
                                    .bottom_0()
                                    .w(px(12.))
                                    .child(
                                        Scrollbar::vertical(&self.environment_list_scroll_handle)
                                            .scrollbar_show(ScrollbarShow::Always),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex_shrink_0()
                            .p_3()
                            .border_t_1()
                            .border_color(theme.sidebar_border)
                            .child(
                                Button::new("api-new-environment")
                                    .secondary()
                                    .small()
                                    .w_full()
                                    .icon(IconName::Plus)
                                    .label(t!("ApiTest.new_environment").to_string())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.prompt_new_environment(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(56.))
                            .flex_shrink_0()
                            .items_center()
                            .gap_2()
                            .px_5()
                            .border_b_1()
                            .border_color(theme.border)
                            .bg(theme.popover)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.popover_foreground)
                                    .child(t!("ApiTest.environment_settings").to_string()),
                            )
                            .child(
                                Button::new("api-environment-manager-close")
                                    .ghost()
                                    .small()
                                    .w(px(32.))
                                    .px_0()
                                    .justify_center()
                                    .icon(IconName::Close)
                                    .on_click(|_, window, cx| {
                                        window.close_dialog(cx);
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .child(
                                div()
                                    .id("api-environment-settings-scroll")
                                    .size_full()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.environment_settings_scroll_handle)
                                    .child(
                                v_flex()
                                    .w_full()
                                    .min_w_0()
                                    .gap_4()
                                    .p_5()
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .items_center()
                                            .gap_3()
                                            .p_4()
                                            .bg(theme.secondary)
                                            .border_1()
                                            .border_color(theme.border)
                                            .rounded_lg()
                                            .child(
                                                div()
                                                    .size(px(36.))
                                                    .flex_shrink_0()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .rounded(px(9.))
                                                    .bg(theme.sidebar_primary)
                                                    .child(
                                                        Icon::new(IconName::Globe)
                                                            .small()
                                                            .text_color(
                                                                theme.sidebar_primary_foreground,
                                                            ),
                                                    ),
                                            )
                                            .child(
                                                v_flex()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .text_base()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(theme.secondary_foreground)
                                                            .child(active_name),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(theme.muted_foreground)
                                                            .child(
                                                                t!(
                                                                    "ApiTest.environment_settings_hint"
                                                                )
                                                                .to_string(),
                                                            ),
                                                    ),
                                            )
                                            .child(
                                                Button::new("api-rename-environment")
                                                    .outline()
                                                    .small()
                                                    .icon(IconName::Edit)
                                                    .label(
                                                        t!("ApiTest.rename_environment").to_string(),
                                                    )
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.prompt_rename_active_environment(
                                                                window, cx,
                                                            );
                                                        },
                                                    )),
                                            )
                                            .child(
                                                Button::new("api-delete-environment")
                                                    .ghost()
                                                    .small()
                                                    .w(px(32.))
                                                    .px_0()
                                                    .justify_center()
                                                    .icon(IconName::Delete)
                                                    .disabled(environment_count <= 1)
                                                    .tooltip(if environment_count <= 1 {
                                                        t!(
                                                            "ApiTest.cannot_delete_last_environment"
                                                        )
                                                        .to_string()
                                                    } else {
                                                        t!("ApiTest.delete_environment").to_string()
                                                    })
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.delete_active_environment(
                                                                window, cx,
                                                            );
                                                        },
                                                    )),
                                            ),
                                    )
                                    .child(
                                        v_flex()
                                            .id("api-environment-base-url")
                                            .w_full()
                                            .gap_3()
                                            .p_4()
                                            .bg(theme.popover)
                                            .border_1()
                                            .border_color(theme.border)
                                            .rounded_lg()
                                            .child(
                                                h_flex()
                                                    .w_full()
                                                    .items_center()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .text_sm()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(theme.popover_foreground)
                                                            .child(
                                                                t!("ApiTest.environment_base_url")
                                                                    .to_string(),
                                                            ),
                                                    )
                                                    .child(Tag::secondary().small().child(
                                                        t!("ApiTest.inherited").to_string(),
                                                    )),
                                            )
                                            .child(
                                                Input::new(&self.environment_base_url_input)
                                                    .small()
                                                    .w_full(),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(theme.muted_foreground)
                                                    .child(
                                                        t!("ApiTest.environment_base_url_hint")
                                                            .to_string(),
                                                    ),
                                            ),
                                    )
                                    .child(environment_variables_section)
                                    .child(environment_headers_section)
                                    .child(environment_params_section)
                                    .child(environment_cookies_section),
                            ),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .right_0()
                                    .bottom_0()
                                    .w(px(12.))
                                    .child(
                                        Scrollbar::vertical(
                                            &self.environment_settings_scroll_handle,
                                        )
                                        .scrollbar_show(ScrollbarShow::Always),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_request_bar(
        &self,
        protocol: Protocol,
        method: RequestMethod,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = cx.theme().clone();
        let controls = h_flex()
            .flex_1()
            .min_w_0()
            .flex_shrink_0()
            .gap_2()
            .items_center()
            .child(
                div()
                    .id("api-protocol-select")
                    .w(px(104.))
                    .flex_shrink_0()
                    .child(Select::new(&self.protocol_select).small().appearance(true)),
            )
            .when(protocol.uses_http_method(), |bar| {
                bar.child(self.render_method_select(method, cx))
            })
            .child(
                div()
                    .id("api-url-input")
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.url_input).small().w_full()),
            )
            .child(self.render_send_button(protocol, cx));

        div()
            .id("api-request-bar")
            .w_full()
            .flex_shrink_0()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(controls)
    }

    fn render_method_select(&self, method: RequestMethod, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("api-method-select")
            .w(px(92.))
            .flex_shrink_0()
            .child(
                Select::new(&self.method_select)
                    .small()
                    .appearance(true)
                    .font_weight(FontWeight::BOLD)
                    .text_color(crate::method_badge_color(method, cx)),
            )
    }

    fn render_send_button(&self, protocol: Protocol, cx: &mut Context<Self>) -> Button {
        let theme = cx.theme().clone();
        let method = self.current_method(cx);
        let streaming = self.stream_stop.is_some();
        let websocket_active =
            protocol == Protocol::WebSocket && self.websocket_state.state.is_active();
        let socket_io_active =
            protocol == Protocol::SocketIo && self.socket_io_state.state.is_active();
        let tcp_active = protocol == Protocol::Tcp && self.tcp_state.state.is_active();
        let active = streaming || websocket_active || socket_io_active || tcp_active;
        let interactive_protocol = matches!(
            protocol,
            Protocol::Tcp | Protocol::WebSocket | Protocol::SocketIo
        );
        let fill = if protocol.uses_http_method() || protocol == Protocol::GrpcWeb {
            crate::method_fill_color(method, cx)
        } else {
            theme.primary
        };
        let hover = if theme.is_dark() {
            gpui_component::Colorize::lighten(&fill, 0.08)
        } else {
            gpui_component::Colorize::darken(&fill, 0.08)
        };
        let active_fill = if theme.is_dark() {
            gpui_component::Colorize::lighten(&fill, 0.14)
        } else {
            gpui_component::Colorize::darken(&fill, 0.14)
        };
        Button::new("api-send")
            .custom(
                ButtonCustomVariant::new(cx)
                    .color(fill)
                    .foreground(theme.primary_foreground)
                    .hover(hover)
                    .active(active_fill)
                    .shadow(true),
            )
            .small()
            .flex_shrink_0()
            .min_w(px(96.))
            .icon(if active {
                IconName::Pause
            } else {
                IconName::Play
            })
            .label(if streaming {
                t!("ApiTest.stop").to_string()
            } else if websocket_active || socket_io_active || tcp_active {
                t!("ApiTest.disconnect").to_string()
            } else if interactive_protocol {
                t!("ApiTest.connect").to_string()
            } else {
                t!("ApiTest.send").to_string()
            })
            .loading(self.sending && !streaming && !interactive_protocol)
            .disabled(self.sending && !streaming && !interactive_protocol)
            .on_click(cx.listener(|this, _ev, window, cx| {
                if this.current_protocol(cx) == Protocol::Tcp && this.tcp_state.state.is_active() {
                    this.disconnect_tcp(cx);
                } else if this.current_protocol(cx) == Protocol::WebSocket
                    && this.websocket_state.state.is_active()
                {
                    this.disconnect_websocket(cx);
                } else if this.current_protocol(cx) == Protocol::SocketIo
                    && this.socket_io_state.state.is_active()
                {
                    this.disconnect_socket_io(cx);
                } else if this.stream_stop.is_some() {
                    this.stop_stream(cx);
                } else {
                    this.send(window, cx);
                }
            }))
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme().clone();
        let collections_active = self.sidebar_mode == SidebarMode::Collections;
        let content = match self.sidebar_mode {
            SidebarMode::Collections => self.render_collections_sidebar(cx).into_any_element(),
            SidebarMode::History => self.render_history_sidebar(cx).into_any_element(),
        };
        v_flex()
            .id("api-request-sidebar")
            .relative()
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .border_r_1()
            .border_color(theme.sidebar_border)
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .child(
                PanelHeader::new("api-sidebar-mode-bar")
                    .variant(PanelHeaderVariant::Sidebar)
                    .background(theme.sidebar)
                    .title(
                        h_flex()
                            .min_w_0()
                            .gap_1()
                            .child(
                                Button::new("api-sidebar-collections")
                                    .xsmall()
                                    .label(t!("ApiTest.collections").to_string())
                                    .when(collections_active, |button| button.primary())
                                    .when(!collections_active, |button| button.ghost())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.sidebar_mode = SidebarMode::Collections;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("api-sidebar-history")
                                    .xsmall()
                                    .label(t!("ApiTest.history").to_string())
                                    .when(!collections_active, |button| button.primary())
                                    .when(collections_active, |button| button.ghost())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.sidebar_mode = SidebarMode::History;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(div().flex_1().min_h_0().min_w_0().child(content))
            .child(self.render_sidebar_toggle_handle(false, cx))
    }

    /// 左侧请求列表树（gpui-component 的可折叠 Tree）。
    fn render_collections_sidebar(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme().clone();
        let weak = cx.weak_entity();
        let active_request_id = self.active_request_id.clone();
        let active_folder_id = self.active_folder_id.clone();
        let request_meta_by_id = self
            .requests
            .iter()
            .map(|request| (request.id.clone(), request.protocol, request.method))
            .collect::<Vec<_>>();
        let folder_ids = self
            .folders
            .iter()
            .map(|folder| folder.id.clone())
            .collect::<Vec<_>>();

        let tree = Tree::new(
            &self.tree_state,
            move |ix, entry, _selected, _window, cx| {
                let theme = cx.theme();
                let item = entry.item();
                let depth = entry.depth();
                let is_request = request_meta_by_id
                    .iter()
                    .any(|(id, _, _)| id == item.id.as_str());
                let is_folder = item.id == TREE_ROOT_ID
                    || folder_ids.iter().any(|id| id == item.id.as_str())
                    || item.is_folder();
                let is_empty = item.id == TREE_EMPTY_ID;
                let is_active = active_request_id.as_deref() == Some(item.id.as_str())
                    || (!is_request && active_folder_id.as_deref() == Some(item.id.as_str()));

                let (protocol, method) = request_meta_by_id
                    .iter()
                    .find(|(id, _, _)| id == item.id.as_str())
                    .map(|(_, protocol, method)| (*protocol, *method))
                    .unwrap_or((Protocol::Http, RequestMethod::Get));

                let mut row = ListItem::new(("api-request-tree-row", ix))
                    .pl(px(8.) + px(14.) * depth)
                    .h(px(32.))
                    .w_full()
                    .cursor_pointer();

                if is_active {
                    row = row
                        .bg(theme.primary.opacity(0.12))
                        .border_l_2()
                        .border_color(theme.primary);
                }

                if is_folder {
                    row = row
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .items_center()
                                .gap_1()
                                .child(
                                    h_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .items_center()
                                        .gap_1()
                                        .child(
                                            Icon::new(if entry.is_expanded() {
                                                IconName::ChevronDown
                                            } else {
                                                IconName::ChevronRight
                                            })
                                            .xsmall()
                                            .flex_shrink_0()
                                            .text_color(theme.muted_foreground),
                                        )
                                        .child(
                                            Icon::new(if entry.is_expanded() {
                                                IconName::FolderOpen
                                            } else {
                                                IconName::FolderClosed
                                            })
                                            .xsmall()
                                            .flex_shrink_0()
                                            .text_color(if item.id == TREE_ROOT_ID {
                                                theme.primary
                                            } else {
                                                theme.muted_foreground
                                            }),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_ellipsis()
                                                .text_sm()
                                                .font_weight(if item.id == TREE_ROOT_ID {
                                                    FontWeight::SEMIBOLD
                                                } else {
                                                    FontWeight::NORMAL
                                                })
                                                .child(item.label.clone()),
                                        ),
                                )
                                .when(
                                    item.id != TREE_ROOT_ID && item.id != TREE_EMPTY_ID,
                                    |content| {
                                        content.child(
                                            Button::new(("api-folder-delete", ix))
                                                .ghost()
                                                .xsmall()
                                                .flex_shrink_0()
                                                .icon(IconName::Delete)
                                                .tooltip(t!("ApiTest.delete_folder").to_string())
                                                .on_click({
                                                    let folder_id = item.id.clone();
                                                    let weak = weak.clone();
                                                    move |_event, window, cx| {
                                                        cx.stop_propagation();
                                                        let _ = weak.update(cx, |this, cx| {
                                                            this.delete_folder(
                                                                &folder_id, window, cx,
                                                            );
                                                        });
                                                    }
                                                }),
                                        )
                                    },
                                ),
                        )
                        .on_click({
                            let folder_id = item.id.clone();
                            let weak = weak.clone();
                            move |_event, window, cx| {
                                if folder_id != TREE_ROOT_ID && folder_id != TREE_EMPTY_ID {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.select_folder(&folder_id, window, cx);
                                    });
                                }
                            }
                        });
                } else if is_empty {
                    row = row.child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(item.label.clone()),
                    );
                } else if is_request {
                    row = row
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .items_center()
                                .gap_1()
                                .child(
                                    h_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .items_center()
                                        .gap_2()
                                        .child(Self::render_request_badge(
                                            protocol,
                                            method,
                                            if protocol.uses_http_method() {
                                                crate::method_badge_color(method, cx)
                                            } else {
                                                theme.primary
                                            },
                                        ))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_ellipsis()
                                                .text_sm()
                                                .child(item.label.clone()),
                                        ),
                                )
                                .child(
                                    Button::new(("api-request-delete", ix))
                                        .ghost()
                                        .xsmall()
                                        .flex_shrink_0()
                                        .icon(IconName::Delete)
                                        .tooltip(t!("ApiTest.delete_request").to_string())
                                        .on_click({
                                            let request_id = item.id.clone();
                                            let weak = weak.clone();
                                            move |_event, window, cx| {
                                                cx.stop_propagation();
                                                let _ = weak.update(cx, |this, cx| {
                                                    this.delete_request(&request_id, window, cx);
                                                });
                                            }
                                        }),
                                ),
                        )
                        .on_click({
                            let request_id = item.id.clone();
                            let weak = weak.clone();
                            move |_event, window, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.select_request(&request_id, window, cx);
                                });
                            }
                        });
                }

                row
            },
        );

        v_flex()
            .id("api-collections-sidebar")
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.sidebar_border)
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.search_input)
                                .small()
                                .w_full()
                                .prefix(IconName::Search)
                                .cleanable(true),
                        ),
                    )
                    .child(self.render_new_request_button(cx)),
            )
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(theme.sidebar_border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("ApiTest.request_list").to_string()),
                    )
                    .child(
                        h_flex().flex_shrink_0().items_center().gap_0p5().child(
                            Button::new("api-new-folder")
                                .ghost()
                                .xsmall()
                                .icon(IconName::NewFolder)
                                .tooltip(t!("ApiTest.new_folder").to_string())
                                .on_click(
                                    cx.listener(|this, _ev, window, cx| {
                                        this.new_folder(window, cx)
                                    }),
                                ),
                        ),
                    ),
            )
            .child(
                div()
                    .id("api-request-tree")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(tree),
            )
            .child(
                v_flex()
                    .w_full()
                    .flex_shrink_0()
                    .gap_1()
                    .p_2()
                    .border_t_1()
                    .border_color(theme.sidebar_border)
                    .child(
                        Button::new("api-import-collection")
                            .ghost()
                            .small()
                            .w_full()
                            .justify_start()
                            .icon(IconName::Upload)
                            .label(t!("ApiTest.import_collection").to_string())
                            .tooltip(t!("ApiTest.import_collection").to_string())
                            .on_click(cx.listener(|this, _ev, window, cx| {
                                this.import_collection_file(window, cx);
                            })),
                    )
                    .child(self.render_export_button(cx)),
            )
    }

    fn render_history_sidebar(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme().clone();
        let rows = self
            .history
            .iter()
            .enumerate()
            .map(|(index, entry)| self.render_history_row(index, entry, cx))
            .collect::<Vec<_>>();
        v_flex()
            .id("api-history-list")
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!(
                                "{} · {}",
                                t!("ApiTest.history"),
                                self.history.len()
                            )),
                    )
                    .child(
                        Button::new("api-clear-history")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Delete)
                            .label(t!("ApiTest.clear_history").to_string())
                            .disabled(self.history.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.clear_history(cx))),
                    ),
            )
            .child(if rows.is_empty() {
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(IconName::Inbox))
                    .child(t!("ApiTest.history_empty").to_string())
                    .into_any_element()
            } else {
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("api-history-scroll")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.history_scroll_handle)
                            .child(v_flex().w_full().p_2().gap_1().children(rows)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .w(px(12.))
                            .child(
                                Scrollbar::vertical(&self.history_scroll_handle)
                                    .scrollbar_show(ScrollbarShow::Always),
                            ),
                    )
                    .into_any_element()
            })
    }

    fn render_history_row(
        &self,
        index: usize,
        entry: &RequestHistoryEntry,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = cx.theme().clone();
        let history_id = entry.id.clone();
        let status_color = if entry.error.is_some() || entry.status < 200 || entry.status >= 500 {
            theme.danger
        } else if entry.status >= 300 {
            theme.warning
        } else {
            theme.success
        };
        v_flex()
            .id(("api-history-entry", index))
            .w_full()
            .gap_1()
            .p_2()
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|style| style.bg(theme.muted.opacity(0.35)))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(Self::render_request_badge(
                        entry.request.protocol,
                        entry.method,
                        if entry.request.protocol.uses_http_method() {
                            crate::method_badge_color(entry.method, cx)
                        } else {
                            theme.primary
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(entry.request_name.clone()),
                    )
                    .child(
                        Tag::custom(
                            status_color.opacity(0.12),
                            status_color,
                            status_color.opacity(0.35),
                        )
                        .small()
                        .rounded_full()
                        .child(if entry.status == 0 {
                            t!("ApiTest.error").to_string()
                        } else {
                            entry.status.to_string()
                        }),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(entry.url.clone()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(Self::history_time(entry.sent_at))
                    .child(format!("{} ms", entry.time_ms))
                    .child(Self::format_size(entry.size)),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.restore_history(&history_id, window, cx);
            }))
    }

    fn history_time(sent_at: i64) -> String {
        chrono::DateTime::<chrono::Utc>::from_timestamp_millis(sent_at)
            .map(|time| time.format("%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "—".to_string())
    }

    fn render_request_badge(protocol: Protocol, method: RequestMethod, color: Hsla) -> Tag {
        Tag::custom(color.opacity(0.12), color, color.opacity(0.35))
            .small()
            .flex_shrink_0()
            .w(px(82.))
            .justify_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .rounded(px(4.))
            .font_weight(FontWeight::BOLD)
            .child(Self::request_badge_label(protocol, method))
    }

    /// 请求编辑区：页签切换，单个面板占满剩余区域，避免输入框互相挤压。
    fn render_editor_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.active_folder_id.is_some() {
            return self.render_folder_editor(cx);
        }
        let theme = cx.theme().clone();
        let active_tab = self.active_editor_tab;
        let tabs = TabBar::new("api-editor-tabs")
            .small()
            .underline()
            .menu(true)
            .selected_index(active_tab as usize)
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                if let Some(tab) = REQUEST_EDITOR_TABS.get(*ix).copied() {
                    this.active_editor_tab = tab;
                    cx.notify();
                }
            }))
            .children(REQUEST_EDITOR_TABS.into_iter().map(|tab| {
                Tab::new()
                    .prefix(
                        div()
                            .id(tab.element_id())
                            .absolute()
                            .size(px(0.))
                            .overflow_hidden(),
                    )
                    .label(tab.title())
            }));

        let content: AnyElement = match active_tab {
            RequestEditorTab::Params => self
                .render_kv_editor(KvSection::Params, cx)
                .into_any_element(),
            RequestEditorTab::Path => self
                .render_kv_editor(KvSection::Path, cx)
                .into_any_element(),
            RequestEditorTab::Headers => self
                .render_kv_editor(KvSection::Headers, cx)
                .into_any_element(),
            RequestEditorTab::Body => self.render_body_editor(cx).into_any_element(),
            RequestEditorTab::Auth => self.render_auth_editor(cx).into_any_element(),
            RequestEditorTab::Cookies => self
                .render_kv_editor(KvSection::Cookies, cx)
                .into_any_element(),
            RequestEditorTab::PreRequest => self
                .render_script_editor(
                    &self.pre_script_input,
                    t!("ApiTest.pre_request_hint").to_string(),
                    cx,
                )
                .into_any_element(),
            RequestEditorTab::Tests => self
                .render_script_editor(&self.tests_input, t!("ApiTest.tests_hint").to_string(), cx)
                .into_any_element(),
            RequestEditorTab::Variables => self.render_variables_editor(cx).into_any_element(),
        };

        v_flex()
            .id("api-editor-pane")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .bg(theme.background)
            .child(
                tabs.w_full()
                    .px_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .bg(theme.background),
            )
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1p5()
                    .border_b_1()
                    .border_color(theme.border)
                    .bg(theme.muted.opacity(0.04))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child(t!("ApiTest.request_description").to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Textarea::new(&self.request_description_input).w_full()),
                    ),
            )
            .child(
                v_flex()
                    .id("api-active-editor")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .bg(theme.background)
                    .p_3()
                    .child(content),
            )
            .into_any_element()
    }

    fn render_folder_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let folder_name = self.name_input.clone();
        // 外层定位容器 + 显式常显滚动条(新 gpui-component 的 Scrollable 无 per-instance 模式)
        div()
            .relative()
            .size_full()
            .min_h_0()
            .min_w_0()
            .child(
                v_flex()
                    .id("api-folder-editor")
                    .size_full()
                    .min_h_0()
                    .min_w_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.folder_editor_scroll_handle)
                    .gap_3()
                    .p_4()
                    .bg(theme.muted.opacity(0.06))
                    .child(
                        h_flex()
                            .w_full()
                            .items_start()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(40.))
                                    .rounded(px(10.))
                                    .bg(theme.primary.opacity(0.12))
                                    .text_color(theme.primary)
                                    .child(Icon::new(IconName::FolderOpen).size_5()),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xl()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(t!("ApiTest.folder_settings").to_string()),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(theme.muted_foreground)
                                            .child(t!("ApiTest.folder_settings_hint").to_string()),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(260.))
                                    .flex_shrink_0()
                                    .child(Input::new(&folder_name).small().w_full()),
                            ),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .gap_3()
                            .p_3()
                            .id("api-folder-settings-card")
                            .border_1()
                            .border_color(theme.border)
                            .rounded(px(8.))
                            .bg(theme.background)
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(t!("ApiTest.folder_description").to_string()),
                                    )
                                    .child(Textarea::new(&self.folder_description_input).w_full())
                                    .child(
                                        div().text_xs().text_color(theme.muted_foreground).child(
                                            t!("ApiTest.folder_description_hint").to_string(),
                                        ),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_2()
                                    .pt_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(t!("ApiTest.folder_base_url").to_string()),
                                    )
                                    .child(
                                        div().id("api-folder-base-url").child(
                                            Input::new(&self.folder_base_url_input)
                                                .small()
                                                .w_full()
                                                .prefix(IconName::Globe),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child(t!("ApiTest.folder_base_url_hint").to_string()),
                                    ),
                            ),
                    )
                    .child(
                        v_flex()
                            .id("api-folder-params")
                            .w_full()
                            .h(px(220.))
                            .gap_2()
                            .p_3()
                            .border_1()
                            .border_color(theme.border)
                            .rounded(px(8.))
                            .bg(theme.background)
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(t!("ApiTest.folder_params").to_string()),
                                    )
                                    .child(
                                        Tag::secondary()
                                            .small()
                                            .flex_shrink_0()
                                            .child(t!("ApiTest.inherited").to_string()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .child(self.render_kv_editor(KvSection::FolderParams, cx)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(t!("ApiTest.folder_params_hint").to_string()),
                            ),
                    )
                    .child(
                        v_flex()
                            .id("api-folder-headers")
                            .w_full()
                            .h(px(220.))
                            .gap_2()
                            .p_3()
                            .border_1()
                            .border_color(theme.border)
                            .rounded(px(8.))
                            .bg(theme.background)
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(t!("ApiTest.folder_headers").to_string()),
                                    )
                                    .child(
                                        Tag::secondary()
                                            .small()
                                            .flex_shrink_0()
                                            .child(t!("ApiTest.inherited").to_string()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .child(self.render_kv_editor(KvSection::FolderHeaders, cx)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(t!("ApiTest.folder_headers_hint").to_string()),
                            ),
                    )
                    .child(
                        v_flex()
                            .id("api-folder-variables")
                            .w_full()
                            .h(px(230.))
                            .gap_2()
                            .p_3()
                            .border_1()
                            .border_color(theme.border)
                            .rounded(px(8.))
                            .bg(theme.background)
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(t!("ApiTest.folder_variables").to_string()),
                                    )
                                    .child(
                                        Tag::secondary()
                                            .small()
                                            .flex_shrink_0()
                                            .child(t!("ApiTest.inherited").to_string()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .child(self.render_kv_editor(KvSection::FolderVariables, cx)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(t!("ApiTest.folder_variables_hint").to_string()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(t!("ApiTest.folder_variable_precedence").to_string()),
                            ),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .w(px(12.))
                    .child(
                        Scrollbar::vertical(&self.folder_editor_scroll_handle)
                            .scrollbar_show(ScrollbarShow::Always),
                    ),
            )
            .into_any_element()
    }

    fn render_variables_editor(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let section = |header: AnyElement, hint: Option<String>, editor: Div| {
            v_flex()
                .w_full()
                .h(px(190.))
                .min_h(px(150.))
                .gap_2()
                .p_3()
                .border_1()
                .border_color(theme.border)
                .rounded(px(8.))
                .bg(theme.background)
                .child(header)
                .child(div().flex_1().min_h_0().child(editor))
                .when_some(hint, |this, hint| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(hint),
                    )
                })
        };
        let title = |text: String| {
            div()
                .flex_shrink_0()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.foreground)
                .child(text)
                .into_any_element()
        };

        div()
            .relative()
            .size_full()
            .min_h_0()
            .child(
                div()
                    .id("api-globals-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.globals_scroll_handle)
                    .child(
                        v_flex()
                            .w_full()
                            .gap_4()
                            .child(section(
                                title(t!("ApiTest.global_variables").to_string()),
                                None,
                                self.render_kv_editor(KvSection::Globals, cx),
                            ))
                            .child(section(
                                title(t!("ApiTest.global_params").to_string()),
                                Some(t!("ApiTest.global_params_hint").to_string()),
                                self.render_kv_editor(KvSection::GlobalParams, cx),
                            ))
                            .child(section(
                                title(t!("ApiTest.global_headers").to_string()),
                                Some(t!("ApiTest.global_headers_hint").to_string()),
                                self.render_kv_editor(KvSection::GlobalHeaders, cx),
                            ))
                            .child(section(
                                title(t!("ApiTest.global_cookies").to_string()),
                                Some(t!("ApiTest.global_cookies_hint").to_string()),
                                self.render_kv_editor(KvSection::GlobalCookies, cx),
                            ))
                            .child(section(
                                title(t!("ApiTest.request_variables").to_string()),
                                None,
                                self.render_kv_editor(KvSection::RequestVariables, cx),
                            )),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .w(px(12.))
                    .child(
                        Scrollbar::vertical(&self.globals_scroll_handle)
                            .scrollbar_show(ScrollbarShow::Always),
                    ),
            )
    }

    fn render_kv_editor(&self, section: KvSection, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let rows = self.rows_for_section(section);
        let has_rows = !rows.is_empty();
        let section_for_closure = section;
        let section_id = section.element_id();
        let scroll_handle = self
            .kv_scroll_handles
            .get(&section)
            .expect("every key-value section must have a scroll handle")
            .clone();
        let is_form_data =
            section == KvSection::Body && self.current_body_type(cx) == BodyType::FormData;

        let row_elements = rows
            .iter()
            .enumerate()
            .map(|(ix, row)| {
                let key = row.key.clone();
                let value = row.value.clone();
                let enabled = row.enabled;
                let field_type = row.field_type;
                let file_path = row.file_path.clone();
                h_flex()
                    .id(format!("api-kv-row-{section_id}-{ix}"))
                    .w_full()
                    .h(px(38.))
                    .min_w_0()
                    .flex_shrink_0()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border.opacity(0.55))
                    .bg(theme.background)
                    .hover(|style| style.bg(theme.muted.opacity(0.16)))
                    .child(
                        div()
                            .w(px(36.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Checkbox::new(format!("api-kv-enabled-{section_id}-{ix}"))
                                    .checked(enabled)
                                    .on_click(cx.listener(move |this, &checked, _, cx| {
                                        this.set_kv_row_enabled(
                                            section_for_closure,
                                            ix,
                                            checked,
                                            cx,
                                        );
                                    })),
                            ),
                    )
                    .when(is_form_data, |row| {
                        row.child(
                            h_flex()
                                .w(px(120.))
                                .flex_shrink_0()
                                .gap_0p5()
                                .px_2()
                                .border_l_1()
                                .border_color(theme.border.opacity(0.55))
                                .child(
                                    Button::new(format!("api-form-field-text-{section_id}-{ix}"))
                                        .xsmall()
                                        .flex_1()
                                        .label(t!("ApiTest.text").to_string())
                                        .when(
                                            field_type == crate::http::FieldType::Text,
                                            |button| button.primary(),
                                        )
                                        .when(
                                            field_type != crate::http::FieldType::Text,
                                            |button| button.ghost(),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.set_kv_row_field_type(
                                                section_for_closure,
                                                ix,
                                                crate::http::FieldType::Text,
                                                cx,
                                            );
                                        })),
                                )
                                .child(
                                    Button::new(format!("api-form-field-file-{section_id}-{ix}"))
                                        .xsmall()
                                        .flex_1()
                                        .label(t!("ApiTest.file").to_string())
                                        .when(
                                            field_type == crate::http::FieldType::File,
                                            |button| button.primary(),
                                        )
                                        .when(
                                            field_type != crate::http::FieldType::File,
                                            |button| button.ghost(),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.set_kv_row_field_type(
                                                section_for_closure,
                                                ix,
                                                crate::http::FieldType::File,
                                                cx,
                                            );
                                        })),
                                ),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_2()
                            .border_l_1()
                            .border_color(theme.border.opacity(0.55))
                            .child(Input::new(&key).small().w_full().appearance(false)),
                    )
                    .child(
                        if is_form_data && field_type == crate::http::FieldType::File {
                            let full_path = file_path.clone().unwrap_or_default();
                            let label = file_path
                                .as_deref()
                                .and_then(|path| std::path::Path::new(path).file_name())
                                .and_then(|name| name.to_str())
                                .map(ToOwned::to_owned)
                                .unwrap_or_else(|| t!("ApiTest.choose_file").to_string());
                            div()
                                .flex_1()
                                .min_w_0()
                                .px_2()
                                .border_l_1()
                                .border_color(theme.border.opacity(0.55))
                                .child(
                                    Button::new(format!("api-form-file-picker-{section_id}-{ix}"))
                                        .ghost()
                                        .small()
                                        .w_full()
                                        .justify_start()
                                        .icon(IconName::File)
                                        .label(label)
                                        .tooltip(if full_path.is_empty() {
                                            t!("ApiTest.no_file_selected").to_string()
                                        } else {
                                            full_path
                                        })
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.choose_kv_file(
                                                section_for_closure,
                                                ix,
                                                window,
                                                cx,
                                            );
                                        })),
                                )
                                .into_any_element()
                        } else {
                            div()
                                .flex_1()
                                .min_w_0()
                                .px_2()
                                .border_l_1()
                                .border_color(theme.border.opacity(0.55))
                                .child(Input::new(&value).small().w_full().appearance(false))
                                .into_any_element()
                        },
                    )
                    .child(
                        div()
                            .w(px(36.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .border_l_1()
                            .border_color(theme.border.opacity(0.55))
                            .child(
                                Button::new(format!("api-kv-delete-{section_id}-{ix}"))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Delete)
                                    .tooltip(t!("ApiTest.delete_row").to_string())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_kv_row(section_for_closure, ix, cx);
                                    })),
                            ),
                    )
            })
            .collect::<Vec<_>>();

        v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .border_1()
            .border_color(theme.border)
            .rounded(px(6.))
            .bg(theme.background)
            .child(
                h_flex()
                    .w_full()
                    .min_h(px(36.))
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .bg(theme.muted.opacity(0.14))
                    .child(
                        div()
                            .w(px(36.))
                            .flex_shrink_0()
                            .text_center()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child(""),
                    )
                    .when(is_form_data, |header| {
                        header.child(
                            div()
                                .w(px(120.))
                                .flex_shrink_0()
                                .px_2()
                                .border_l_1()
                                .border_color(theme.border.opacity(0.55))
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.muted_foreground)
                                .child(t!("ApiTest.text").to_string()),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_2()
                            .border_l_1()
                            .border_color(theme.border.opacity(0.55))
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child(t!("ApiTest.key").to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_2()
                            .border_l_1()
                            .border_color(theme.border.opacity(0.55))
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child(t!("ApiTest.value").to_string()),
                    )
                    .child(
                        div()
                            .w(px(36.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .border_l_1()
                            .border_color(theme.border.opacity(0.55))
                            .child(
                                Button::new(format!("api-kv-add-{section_id}"))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Plus)
                                    .tooltip(t!("ApiTest.add_row").to_string())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.add_kv_row(section_for_closure, window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .relative()
                    .overflow_hidden()
                    .child(
                        div()
                            .id(format!("api-kv-scroll-view-{section_id}"))
                            .size_full()
                            .min_h_0()
                            .min_w_0()
                            .overflow_y_scroll()
                            .track_scroll(&scroll_handle)
                            .when(has_rows, |body| {
                                body.child(
                                    v_flex()
                                        .w_full()
                                        .flex_shrink_0()
                                        .children(row_elements)
                                        .child(
                                            h_flex()
                                                .id(format!("api-kv-add-footer-{section_id}"))
                                                .w_full()
                                                .h(px(34.))
                                                .flex_shrink_0()
                                                .items_center()
                                                .justify_center()
                                                .gap_1()
                                                .cursor_pointer()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .hover(|style| {
                                                    style
                                                        .bg(theme.muted.opacity(0.12))
                                                        .text_color(theme.foreground)
                                                })
                                                .child(Icon::new(IconName::Plus).xsmall())
                                                .child(t!("ApiTest.add_row").to_string())
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.add_kv_row(
                                                            section_for_closure,
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                )),
                                        ),
                                )
                            })
                            .when(!has_rows, |body| {
                                body.child(
                                    v_flex()
                                        .id(format!("api-kv-empty-{section_id}"))
                                        .size_full()
                                        .min_h(px(96.))
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .text_color(theme.muted_foreground)
                                        .bg(theme.muted.opacity(0.04))
                                        .hover(|style| style.bg(theme.muted.opacity(0.1)))
                                        .child(
                                            h_flex()
                                                .items_center()
                                                .gap_1p5()
                                                .px_3()
                                                .py_1p5()
                                                .border_1()
                                                .border_color(theme.border)
                                                .rounded(px(6.))
                                                .bg(theme.background)
                                                .text_color(theme.foreground)
                                                .child(Icon::new(IconName::Plus).small())
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .child(t!("ApiTest.add_row").to_string()),
                                                ),
                                        )
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.add_kv_row(section_for_closure, window, cx);
                                        })),
                                )
                            }),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .w(px(12.))
                            .child(
                                Scrollbar::vertical(&scroll_handle)
                                    .id(format!("api-kv-scrollbar-{section_id}"))
                                    .scrollbar_show(ScrollbarShow::Always),
                            ),
                    ),
            )
    }

    fn render_body_editor(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let body_type = self.current_body_type(cx);

        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .w(px(220.))
                            .child(Select::new(&self.body_type_select).small()),
                    )
                    .when(body_type == BodyType::Raw, |this| {
                        this.child(
                            div()
                                .w(px(140.))
                                .child(Select::new(&self.raw_lang_select).small()),
                        )
                    }),
            )
            .child(match body_type {
                BodyType::None => div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.muted_foreground)
                    .text_sm()
                    .child(t!("ApiTest.body_none").to_string())
                    .into_any_element(),
                BodyType::Raw => div()
                    .flex_1()
                    .min_h_0()
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(6.))
                    .overflow_hidden()
                    .child(
                        Textarea::new(&self.body_input)
                            .h_full()
                            .font_family(theme.mono_font_family.clone()),
                    )
                    .into_any_element(),
                BodyType::FormData | BodyType::Urlencoded => self
                    .render_kv_editor(KvSection::Body, cx)
                    .into_any_element(),
            })
    }

    fn render_auth_editor(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let auth_type = self.current_auth_type(cx);
        let auth_target = self.current_auth_target(cx);

        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .child(
                h_flex().child(
                    div()
                        .w(px(220.))
                        .child(Select::new(&self.auth_type_select).small()),
                ),
            )
            .child(match auth_type {
                AuthType::None => div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.muted_foreground)
                    .text_sm()
                    .child(t!("ApiTest.auth_none").to_string())
                    .into_any_element(),
                AuthType::Bearer => v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child(t!("ApiTest.auth_token").to_string()),
                    )
                    .child(Input::new(&self.auth_token_input).small().w_full())
                    .into_any_element(),
                AuthType::Basic => v_flex()
                    .gap_2()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.muted_foreground)
                                    .child(t!("ApiTest.auth_username").to_string()),
                            )
                            .child(Input::new(&self.auth_username_input).small().w_full()),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.muted_foreground)
                                    .child(t!("ApiTest.auth_password").to_string()),
                            )
                            .child(Input::new(&self.auth_password_input).small().w_full()),
                    )
                    .into_any_element(),
                AuthType::ApiKey => v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&self.auth_key_input).small().w_full()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&self.auth_value_input).small().w_full()),
                            )
                            .child(
                                div()
                                    .w(px(130.))
                                    .child(Select::new(&self.auth_target_select).small()),
                            ),
                    )
                    .child(div().text_xs().text_color(theme.muted_foreground).child(
                        if auth_target == AuthTarget::Header {
                            t!("ApiTest.auth_target_header").to_string()
                        } else {
                            t!("ApiTest.auth_target_query").to_string()
                        },
                    ))
                    .into_any_element(),
            })
    }

    fn render_script_editor(
        &self,
        input: &Entity<TextareaState>,
        hint: String,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme().clone();
        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(hint),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(6.))
                    .overflow_hidden()
                    .child(
                        Textarea::new(input)
                            .h_full()
                            .font_family(theme.mono_font_family.clone()),
                    ),
            )
    }

    fn render_response(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        match self.current_protocol(cx) {
            Protocol::Tcp => return self.render_tcp_response(cx),
            Protocol::WebSocket => return self.render_websocket_response(cx),
            Protocol::SocketIo => return self.render_socket_io_response(cx),
            _ => {}
        }
        let theme = cx.theme().clone();
        let active_tab = self.active_response_tab;
        let tabs = self.render_response_tabs(active_tab, cx);
        let status_bar = (self.sending || self.stream_stop.is_some() || self.response.is_some())
            .then(|| self.render_response_status_bar(cx));

        let body_text = self
            .response
            .as_ref()
            .map(|response| {
                if self.response_pretty {
                    response.body.clone()
                } else {
                    response.raw_body.clone()
                }
            })
            .unwrap_or_default();
        let actual_request = self
            .prepared_request
            .as_ref()
            .map(actual_request_text)
            .unwrap_or_default();
        let curl = self
            .prepared_request
            .as_ref()
            .map(curl_command)
            .unwrap_or_default();
        let console = console_text(self.pre_result.as_ref(), self.test_result.as_ref());
        let headers = self
            .response
            .as_ref()
            .map(|r| r.headers.clone())
            .unwrap_or_default();
        let cookies = headers
            .iter()
            .filter(|header| header.key.eq_ignore_ascii_case("set-cookie"))
            .map(|header| response_cookie_pair(&header.value))
            .collect::<Vec<_>>();

        let response_content: AnyElement = if active_tab == ResponseTab::Example {
            self.render_response_examples(&theme)
        } else if self.sending && self.stream_stop.is_none() {
            div()
                .id("api-response-loading")
                .flex_1()
                .min_h_0()
                .child(ContentState::loading(t!("ApiTest.sending").to_string()))
                .into_any_element()
        } else if self.response.is_none()
            && self.prepared_request.is_none()
            && self.pre_result.is_none()
        {
            div()
                .id("api-response-empty")
                .flex_1()
                .min_h_0()
                .child(ContentState::empty(t!("ApiTest.no_response").to_string()))
                .into_any_element()
        } else {
            match active_tab {
                ResponseTab::Body => {
                    let body_for_copy = body_text.clone();
                    v_flex()
                        .id("api-response-body")
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .overflow_hidden()
                        .p_3()
                        .child(
                            h_flex()
                                .w_full()
                                .flex_shrink_0()
                                .items_center()
                                .justify_between()
                                .pb_2()
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .child(
                                            Button::new("api-response-pretty")
                                                .xsmall()
                                                .label(t!("ApiTest.pretty").to_string())
                                                .when(self.response_pretty, |button| {
                                                    button.primary()
                                                })
                                                .when(!self.response_pretty, |button| {
                                                    button.ghost()
                                                })
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.response_pretty = true;
                                                    cx.notify();
                                                })),
                                        )
                                        .child(
                                            Button::new("api-response-raw")
                                                .xsmall()
                                                .label(t!("ApiTest.raw").to_string())
                                                .when(!self.response_pretty, |button| {
                                                    button.primary()
                                                })
                                                .when(self.response_pretty, |button| button.ghost())
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.response_pretty = false;
                                                    cx.notify();
                                                })),
                                        ),
                                )
                                .child(
                                    Button::new("api-copy-response-body")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Copy)
                                        .label(t!("ApiTest.copy").to_string())
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                body_for_copy.clone(),
                                            ));
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_h_0()
                                .min_w_0()
                                .rounded(px(6.))
                                .bg(theme.muted.opacity(0.14))
                                .child(
                                    div()
                                        .id("api-response-body-scroll")
                                        .size_full()
                                        .overflow_scroll()
                                        .track_scroll(&self.response_body_scroll_handle)
                                        .p_3()
                                        .font_family(theme.mono_font_family.clone())
                                        .text_sm()
                                        .text_color(theme.foreground)
                                        .child(body_text),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .right_0()
                                        .bottom_0()
                                        .w(px(12.))
                                        .child(
                                            Scrollbar::vertical(&self.response_body_scroll_handle)
                                                .scrollbar_show(ScrollbarShow::Always),
                                        ),
                                ),
                        )
                        .into_any_element()
                }
                ResponseTab::Headers => Self::render_readonly_kv(
                    &headers,
                    &self.response_headers_scroll_handle,
                    t!("ApiTest.no_response_headers").to_string(),
                    "api-copy-response-header",
                    theme.border,
                    theme.muted_foreground,
                    theme.foreground,
                    theme.mono_font_family.clone(),
                ),
                ResponseTab::Cookies => Self::render_readonly_kv(
                    &cookies,
                    &self.response_cookies_scroll_handle,
                    t!("ApiTest.no_response_cookies").to_string(),
                    "api-copy-response-cookie",
                    theme.border,
                    theme.muted_foreground,
                    theme.foreground,
                    theme.mono_font_family.clone(),
                ),
                ResponseTab::ActualRequest => self.render_text_response(
                    "api-actual-request",
                    "api-copy-actual-request",
                    actual_request,
                    t!("ApiTest.no_response").to_string(),
                    &theme,
                ),
                ResponseTab::Curl => self.render_text_response(
                    "api-curl-command",
                    "api-copy-curl",
                    curl,
                    t!("ApiTest.no_response").to_string(),
                    &theme,
                ),
                ResponseTab::Console => self.render_text_response(
                    "api-script-console",
                    "api-copy-console",
                    console,
                    t!("ApiTest.no_console_output").to_string(),
                    &theme,
                ),
                ResponseTab::Example => unreachable!("response examples are rendered above"),
            }
        };

        v_flex()
            .id("api-response-pane")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .bg(theme.background)
            .child(tabs)
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .bg(theme.background)
                    .child(response_content),
            )
            .when_some(status_bar, |pane, status_bar| pane.child(status_bar))
    }

    fn render_response_examples(&self, theme: &gpui_component::theme::Theme) -> AnyElement {
        let request = self
            .active_request_id
            .as_deref()
            .and_then(|active_id| self.requests.iter().find(|request| request.id == active_id));
        let success_example = request.and_then(|request| request.success_example.as_ref());
        let fail_examples = request
            .map(|request| request.fail_examples.as_slice())
            .unwrap_or_default();

        if success_example.is_none() && fail_examples.is_empty() {
            return v_flex()
                .id("api-response-examples-empty")
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(Icon::new(IconName::Inbox))
                .child(t!("ApiTest.response_example_empty").to_string())
                .child(
                    div()
                        .text_xs()
                        .child(t!("ApiTest.response_example_autosave_hint").to_string()),
                )
                .into_any_element();
        }

        let mut sections = Vec::new();
        if let Some(example) = success_example {
            sections.push(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .text_color(theme.success)
                            .child(Icon::new(IconName::CircleCheck).small())
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t!("ApiTest.response_example_success").to_string()),
                            ),
                    )
                    .child(self.render_response_example_card(
                        example,
                        true,
                        "api-copy-success-example".to_string(),
                        theme,
                    ))
                    .into_any_element(),
            );
        }

        if !fail_examples.is_empty() {
            sections.push(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .text_color(theme.danger)
                            .child(Icon::new(IconName::CircleX).small())
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t!("ApiTest.response_example_failures").to_string()),
                            ),
                    )
                    .children(fail_examples.iter().enumerate().map(|(index, example)| {
                        self.render_response_example_card(
                            example,
                            false,
                            format!("api-copy-failure-example-{index}"),
                            theme,
                        )
                    }))
                    .into_any_element(),
            );
        }

        div()
            .relative()
            .size_full()
            .min_h_0()
            .child(
                div()
                    .id("api-response-examples")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.response_examples_scroll_handle)
                    .child(v_flex().w_full().gap_4().p_3().children(sections)),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .w(px(12.))
                    .child(
                        Scrollbar::vertical(&self.response_examples_scroll_handle)
                            .scrollbar_show(ScrollbarShow::Always),
                    ),
            )
            .into_any_element()
    }

    fn render_response_example_card(
        &self,
        example: &ResponseExample,
        success: bool,
        copy_id: String,
        theme: &gpui_component::theme::Theme,
    ) -> AnyElement {
        let color = if success { theme.success } else { theme.danger };
        let status = if example.status == 0 {
            t!("ApiTest.error").to_string()
        } else if example.status_text.trim().is_empty() {
            example.status.to_string()
        } else {
            format!("{} {}", example.status, example.status_text)
        };
        let body = example.body.clone();
        let body_for_copy = body.clone();
        let saved_at = if example.saved_at.trim().is_empty() {
            "—".to_string()
        } else {
            t!("ApiTest.saved_at", time = example.saved_at.clone()).to_string()
        };

        v_flex()
            .w_full()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(theme.border)
            .rounded(px(8.))
            .bg(theme.background)
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        h_flex()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .child(
                                Tag::custom(color.opacity(0.12), color, color.opacity(0.35))
                                    .small()
                                    .rounded_full()
                                    .child(status),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(saved_at),
                            ),
                    )
                    .child(
                        Button::new(copy_id)
                            .ghost()
                            .xsmall()
                            .flex_shrink_0()
                            .icon(IconName::Copy)
                            .label(t!("ApiTest.copy").to_string())
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    body_for_copy.clone(),
                                ));
                            }),
                    ),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .min_h(px(52.))
                    .max_h(px(260.))
                    .rounded(px(6.))
                    .bg(theme.muted.opacity(0.28))
                    .child(
                        div()
                            .id("api-response-example-card-scroll")
                            .size_full()
                            .max_h(px(260.))
                            .overflow_scroll()
                            .track_scroll(&self.response_example_card_scroll_handle)
                            .p_3()
                            .font_family(theme.mono_font_family.clone())
                            .text_sm()
                            .child(body),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .w(px(12.))
                            .child(
                                Scrollbar::vertical(&self.response_example_card_scroll_handle)
                                    .scrollbar_show(ScrollbarShow::Always),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_response_tabs(&self, active_tab: ResponseTab, cx: &mut Context<Self>) -> TabBar {
        let header_count = self
            .response
            .as_ref()
            .map(|response| response.headers.len())
            .unwrap_or_default();
        let cookie_count = self
            .response
            .as_ref()
            .map(|response| {
                response
                    .headers
                    .iter()
                    .filter(|header| header.key.eq_ignore_ascii_case("set-cookie"))
                    .count()
            })
            .unwrap_or_default();

        TabBar::new("api-response-tabs")
            .small()
            .underline()
            .menu(true)
            .w_full()
            .px_3()
            .bg(cx.theme().muted.opacity(0.14))
            .border_b_1()
            .border_color(cx.theme().border)
            .selected_index(active_tab as usize)
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                if let Some(tab) = RESPONSE_TABS.get(*ix).copied() {
                    this.active_response_tab = tab;
                    cx.notify();
                }
            }))
            .children(RESPONSE_TABS.into_iter().map(|tab| {
                let label = match tab {
                    ResponseTab::Headers => format!("{} ({header_count})", tab.title()),
                    ResponseTab::Cookies => format!("{} ({cookie_count})", tab.title()),
                    _ => tab.title(),
                };
                Tab::new()
                    .prefix(
                        div()
                            .id(tab.element_id())
                            .absolute()
                            .size(px(0.))
                            .overflow_hidden(),
                    )
                    .label(label)
            }))
    }

    fn response_presentation(&self) -> StatusPresentation {
        if self.sending || self.stream_stop.is_some() {
            return StatusPresentation::Progress;
        }
        let Some(response) = &self.response else {
            return StatusPresentation::Neutral;
        };
        if response.error.is_some() || response.status == 0 {
            StatusPresentation::Error
        } else if (200..300).contains(&response.status) {
            StatusPresentation::Success
        } else if (300..500).contains(&response.status) {
            StatusPresentation::Warning
        } else {
            StatusPresentation::Error
        }
    }

    fn render_response_status_bar(&self, cx: &mut Context<Self>) -> StatusBar {
        StatusBar::new("api-response-status")
            .presentation(self.response_presentation())
            .leading(self.render_response_status_leading(cx))
            .center(self.render_response_status_center(cx))
            .trailing(self.render_response_metrics(cx))
            .muted_background()
    }

    fn render_response_status_leading(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.sending && self.stream_stop.is_none() {
            return h_flex()
                .gap_1()
                .child(
                    Spinner::new()
                        .animation_id("api-response-status-spinner")
                        .small(),
                )
                .child(t!("ApiTest.sending").to_string())
                .into_any_element();
        }
        let Some(response) = &self.response else {
            return div().into_any_element();
        };
        Self::response_status_tag(response, cx.theme()).into_any_element()
    }

    fn render_response_status_center(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(error) = self
            .response
            .as_ref()
            .and_then(|response| response.error.as_ref())
        {
            return div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(cx.theme().danger)
                .child(format!("{}: {error}", t!("ApiTest.error")))
                .into_any_element();
        }
        if self.stream_stop.is_some()
            || self
                .response
                .as_ref()
                .is_some_and(|response| response.streaming)
        {
            return h_flex()
                .gap_1()
                .child(
                    Spinner::new()
                        .animation_id("api-response-streaming-spinner")
                        .small(),
                )
                .child(t!("ApiTest.streaming").to_string())
                .into_any_element();
        }
        div().into_any_element()
    }

    fn render_response_metrics(&self, _cx: &mut Context<Self>) -> AnyElement {
        let Some(response) = &self.response else {
            return div().into_any_element();
        };
        let tests = self.test_result.as_ref().map(|result| {
            t!(
                "ApiTest.tests_passed",
                passed = result.assertions_passed,
                failed = result.assertions_failed
            )
            .to_string()
        });
        h_flex()
            .items_center()
            .gap_3()
            .child(format!("{} ms", response.time_ms))
            .child(Self::format_size(response.size))
            .when_some(tests, |metrics, tests| metrics.child(tests))
            .into_any_element()
    }

    fn response_status_tag(response: &HttpResponse, theme: &gpui_component::theme::Theme) -> Tag {
        let color = if response.error.is_some() || response.status < 200 || response.status >= 500 {
            theme.danger
        } else if response.status >= 300 {
            theme.warning
        } else {
            theme.success
        };
        let label = if response.status == 0 {
            "—".to_string()
        } else if response.status_text.trim().is_empty() {
            response.status.to_string()
        } else {
            format!("{} {}", response.status, response.status_text)
        };
        Tag::custom(color.opacity(0.12), color, color.opacity(0.35))
            .small()
            .rounded_full()
            .child(label)
    }

    fn render_text_response(
        &self,
        panel_id: &'static str,
        copy_id: &'static str,
        text: String,
        placeholder: String,
        theme: &gpui_component::theme::Theme,
    ) -> AnyElement {
        let text_for_copy = text.clone();
        let is_empty = text.trim().is_empty();
        let display_text = if is_empty { placeholder } else { text };
        v_flex()
            .id(panel_id)
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .p_3()
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .justify_end()
                    .pb_2()
                    .child(
                        Button::new(copy_id)
                            .ghost()
                            .xsmall()
                            .icon(IconName::Copy)
                            .label(t!("ApiTest.copy").to_string())
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    text_for_copy.clone(),
                                ));
                            }),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .rounded(px(6.))
                    .bg(theme.muted.opacity(0.14))
                    .child(
                        div()
                            .id("api-response-console-scroll")
                            .size_full()
                            .overflow_scroll()
                            .track_scroll(&self.response_console_scroll_handle)
                            .p_3()
                            .text_sm()
                            .text_color(if is_empty {
                                theme.muted_foreground
                            } else {
                                theme.foreground
                            })
                            .when(!is_empty, |content| {
                                content.font_family(theme.mono_font_family.clone())
                            })
                            .child(display_text),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .w(px(12.))
                            .child(
                                Scrollbar::vertical(&self.response_console_scroll_handle)
                                    .scrollbar_show(ScrollbarShow::Always),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_readonly_kv(
        rows: &[KeyValue],
        scroll_handle: &ScrollHandle,
        empty_text: String,
        copy_id_prefix: &'static str,
        border: Hsla,
        muted: Hsla,
        foreground: Hsla,
        mono_font: SharedString,
    ) -> AnyElement {
        if rows.is_empty() {
            return div()
                .size_full()
                .min_h_0()
                .min_w_0()
                .child(ContentState::empty(empty_text))
                .into_any_element();
        }

        let rows = rows.iter().enumerate().map(|(index, row)| {
            let copy_text = format!("{}: {}", row.key, row.value);
            h_flex()
                .w_full()
                .h(px(36.))
                .min_w_0()
                .flex_shrink_0()
                .gap_2()
                .items_center()
                .px_2()
                .py_1()
                .border_b_1()
                .border_color(border.opacity(0.7))
                .child(
                    div()
                        .w(px(180.))
                        .flex_shrink_0()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family(mono_font.clone())
                        .text_sm()
                        .text_color(muted)
                        .child(row.key.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family(mono_font.clone())
                        .text_sm()
                        .text_color(foreground)
                        .child(row.value.clone()),
                )
                .child(
                    Button::new(format!("{copy_id_prefix}-{index}"))
                        .ghost()
                        .xsmall()
                        .flex_shrink_0()
                        .icon(IconName::Copy)
                        .tooltip(t!("ApiTest.copy").to_string())
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                        }),
                )
        });

        div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .relative()
            .overflow_hidden()
            .child(
                div()
                    .id(format!("{copy_id_prefix}-scroll-view"))
                    .size_full()
                    .min_h_0()
                    .min_w_0()
                    .overflow_y_scroll()
                    .track_scroll(scroll_handle)
                    .p_3()
                    .child(
                        v_flex()
                            .w_full()
                            .flex_shrink_0()
                            .border_1()
                            .border_color(border)
                            .rounded(px(6.))
                            .overflow_hidden()
                            .child(
                                h_flex()
                                    .w_full()
                                    .h(px(32.))
                                    .flex_shrink_0()
                                    .gap_2()
                                    .items_center()
                                    .px_2()
                                    .border_b_1()
                                    .border_color(border)
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(muted)
                                    .child(
                                        div()
                                            .w(px(180.))
                                            .flex_shrink_0()
                                            .child(t!("ApiTest.key").to_string()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(t!("ApiTest.value").to_string()),
                                    )
                                    .child(div().w(px(28.)).flex_shrink_0()),
                            )
                            .child(v_flex().w_full().flex_shrink_0().children(rows)),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .w(px(12.))
                    .child(
                        Scrollbar::vertical(scroll_handle)
                            .id(format!("{copy_id_prefix}-scrollbar"))
                            .scrollbar_show(ScrollbarShow::Always),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod render_contract_tests {
    use super::{KeyValue, apply_environment_effects, merge_variable_scopes};
    use crate::scripting::{ScriptResult, SideEffect, VarScope};

    #[test]
    fn variable_scopes_follow_verve_precedence_and_ignore_disabled_rows() {
        let globals = vec![
            KeyValue::new("base", "global"),
            KeyValue::new("global_only", "1"),
            KeyValue {
                key: "disabled".to_string(),
                value: "ignored".to_string(),
                enabled: false,
                ..KeyValue::default()
            },
        ];
        let environment = vec![
            KeyValue::new("base", "environment"),
            KeyValue::new("environment_only", "2"),
            KeyValue::new(" ", "ignored"),
        ];
        let parent_folder = vec![
            KeyValue::new("base", "parent-folder"),
            KeyValue::new("folder_level", "parent"),
            KeyValue::new("parent_only", "parent"),
        ];
        let child_folder = vec![
            KeyValue::new("base", "child-folder"),
            KeyValue::new("folder_level", "child"),
            KeyValue::new("child_only", "child"),
            KeyValue {
                key: "disabled_folder".to_string(),
                value: "ignored".to_string(),
                enabled: false,
                ..KeyValue::default()
            },
        ];
        let path = vec![
            KeyValue::new("base", "path"),
            KeyValue::new("path_only", "3"),
        ];
        let request = vec![
            KeyValue::new("base", "request"),
            KeyValue::new("request_only", "4"),
        ];
        let folders = [parent_folder.as_slice(), child_folder.as_slice()];

        let vars = merge_variable_scopes(&globals, &environment, &folders, &path, &request);

        assert_eq!(vars.get("base").map(String::as_str), Some("request"));
        assert_eq!(vars.get("global_only").map(String::as_str), Some("1"));
        assert_eq!(vars.get("environment_only").map(String::as_str), Some("2"));
        assert_eq!(vars.get("folder_level").map(String::as_str), Some("child"));
        assert_eq!(vars.get("parent_only").map(String::as_str), Some("parent"));
        assert_eq!(vars.get("child_only").map(String::as_str), Some("child"));
        assert_eq!(vars.get("path_only").map(String::as_str), Some("3"));
        assert_eq!(vars.get("request_only").map(String::as_str), Some("4"));
        assert!(!vars.contains_key("disabled"));
        assert!(!vars.contains_key("disabled_folder"));
        assert!(!vars.contains_key(""));
    }

    #[test]
    fn environment_script_effects_update_existing_rows_and_append_new_rows() {
        let mut variables = vec![
            KeyValue {
                key: "token".to_string(),
                value: "old".to_string(),
                enabled: false,
                ..KeyValue::default()
            },
            KeyValue::new("unchanged", "value"),
        ];
        let result = ScriptResult {
            effects: vec![
                SideEffect::SetVariable {
                    scope: VarScope::Environment,
                    name: "token".to_string(),
                    value: "new".to_string(),
                },
                SideEffect::SetVariable {
                    scope: VarScope::Environment,
                    name: "base_url".to_string(),
                    value: "https://example.test".to_string(),
                },
                SideEffect::SetVariable {
                    scope: VarScope::Request,
                    name: "request_only".to_string(),
                    value: "ignored".to_string(),
                },
            ],
            ..Default::default()
        };

        assert!(apply_environment_effects(&mut variables, &result));
        assert_eq!(
            variables
                .iter()
                .find(|variable| variable.key == "token")
                .map(|variable| (variable.value.as_str(), variable.enabled)),
            Some(("new", true))
        );
        assert_eq!(
            variables
                .iter()
                .find(|variable| variable.key == "base_url")
                .map(|variable| variable.value.as_str()),
            Some("https://example.test")
        );
        assert!(
            !variables
                .iter()
                .any(|variable| variable.key == "request_only")
        );
    }

    #[test]
    fn api_test_renderer_has_sidebar_tree_and_tabbed_editor() {
        let source = include_str!("api_test_view.rs");
        let send_behavior = include_str!("api_test_view/send.rs");
        let websocket_render = include_str!("api_test_view/websocket_view.rs");
        let socket_io_render = include_str!("api_test_view/socket_io_view.rs");
        let tcp_render = include_str!("api_test_view/tcp_view.rs");
        let assert_always_visible_scrollbars = |scope_source: &str, scope_name: &str| {
            let wrapped_scrollbar_count = [
                ".overflow_scrollbar()",
                ".overflow_x_scrollbar()",
                ".overflow_y_scrollbar()",
            ]
            .into_iter()
            .map(|call| scope_source.matches(call).count())
            .sum::<usize>();
            let explicit_scrollbar_count = scope_source.matches("Scrollbar::vertical(").count();
            let scrollbar_count = wrapped_scrollbar_count + explicit_scrollbar_count;
            let always_visible_count = scope_source
                .matches(".scrollbar_show(ScrollbarShow::Always)")
                .count();

            assert!(
                scrollbar_count > 0,
                "{scope_name} must contain at least one scrollable surface"
            );
            assert_eq!(
                always_visible_count, scrollbar_count,
                "{scope_name} scrollable surfaces must keep their scrollbars visible"
            );
        };
        let render_start = source
            .find("impl Render for ApiTestView")
            .expect("api test render impl");
        let render_end = source[render_start..]
            .find("\n#[cfg(test)]")
            .map_or(source.len(), |offset| render_start + offset);
        let production_source = &source[..render_end];
        let behavior = &source[..render_start];
        let render = &source[render_start..render_end];
        let environment_manager_start = production_source
            .find("fn render_environment_manager")
            .expect("environment manager renderer");
        let environment_manager_end = production_source[environment_manager_start..]
            .find("\n    fn render_request_bar")
            .map(|offset| environment_manager_start + offset)
            .expect("environment manager renderer end");
        let environment_manager =
            &production_source[environment_manager_start..environment_manager_end];
        let response_renderer_start = production_source
            .find("fn render_response(&mut self")
            .expect("response renderer");
        let response_renderer_end = production_source[response_renderer_start..]
            .find("\n    fn render_response_examples")
            .map(|offset| response_renderer_start + offset)
            .expect("response renderer end");
        let response_renderer = &production_source[response_renderer_start..response_renderer_end];
        let response_tabs_start = production_source
            .find("fn render_response_tabs")
            .expect("response tabs renderer");
        let response_tabs_end = production_source[response_tabs_start..]
            .find("\n    fn response_presentation")
            .map(|offset| response_tabs_start + offset)
            .expect("response tabs renderer end");
        let response_tabs = &production_source[response_tabs_start..response_tabs_end];
        let kv_editor_start = production_source
            .find("fn render_kv_editor")
            .expect("key-value editor renderer");
        let kv_editor_end = production_source[kv_editor_start..]
            .find("\n    fn render_body_editor")
            .map(|offset| kv_editor_start + offset)
            .expect("key-value editor renderer end");
        let kv_editor = &production_source[kv_editor_start..kv_editor_end];
        let readonly_kv_start = production_source
            .find("fn render_readonly_kv")
            .expect("read-only key-value renderer");
        let readonly_kv = &production_source[readonly_kv_start..];

        assert!(
            render.contains("Tree::new"),
            "the request sidebar must use the gpui-component tree"
        );
        assert!(
            render.contains("api-request-sidebar") && render.contains("api-request-tree"),
            "the sidebar tree must have explicit layout boundaries"
        );
        assert!(
            render.contains("api-editor-tabs") && render.contains("api-active-editor"),
            "request panels must render as tabs with one full-size editor"
        );
        for panel in [
            "api-editor-tab-params",
            "api-editor-tab-path",
            "api-editor-tab-headers",
            "api-editor-tab-body",
            "api-editor-tab-auth",
            "api-editor-tab-cookies",
            "api-editor-tab-pre-request",
            "api-editor-tab-tests",
            "api-editor-tab-variables",
        ] {
            assert!(behavior.contains(panel), "missing request panel: {panel}");
        }
        for panel in [
            "api-response-tab-body",
            "api-response-tab-headers",
            "api-response-tab-cookies",
            "api-response-tab-actual-request",
            "api-response-tab-curl",
            "api-response-tab-console",
            "api-response-tab-example",
        ] {
            assert!(behavior.contains(panel), "missing response panel: {panel}");
        }
        assert!(
            production_source.contains("render_response_examples")
                && production_source.contains("success_example")
                && production_source.contains("fail_examples"),
            "saved response examples must remain visible from the response tabs"
        );
        assert!(
            !response_tabs.contains("ApiTest.response"),
            "the response tab bar must not render a standalone response label before its tabs"
        );
        assert!(
            !production_source.contains("t!(\"ApiTest.response\")"),
            "the response area must not render a standalone response label outside its tabs"
        );
        assert!(
            response_renderer.contains("api-response-body")
                && response_renderer.contains("Scrollbar::vertical(")
                && response_renderer.contains(".scrollbar_show(ScrollbarShow::Always)")
                && response_renderer.contains(".text_color(theme.foreground)")
                && response_renderer.contains(".child(body_text)"),
            "the response body must remain visible in a foreground-colored scroll container"
        );
        for (scope_name, scroll_renderer) in [
            ("request key-value editor", kv_editor),
            ("response key-value viewer", readonly_kv),
        ] {
            assert!(
                scroll_renderer.contains(".overflow_y_scroll()")
                    && scroll_renderer.contains(".track_scroll(")
                    && scroll_renderer.contains("Scrollbar::vertical(")
                    && scroll_renderer.contains(".scrollbar_show(ScrollbarShow::Always)")
                    && scroll_renderer.contains(".flex_shrink_0()"),
                "{scope_name} must use a tracked scroll handle and non-shrinking rows"
            );
        }
        assert!(
            behavior.contains("kv_scroll_handles: BTreeMap<KvSection, ScrollHandle>")
                && behavior.contains("response_headers_scroll_handle: ScrollHandle")
                && behavior.contains("response_cookies_scroll_handle: ScrollHandle")
                && response_renderer.contains("&self.response_headers_scroll_handle")
                && response_renderer.contains("&self.response_cookies_scroll_handle"),
            "request and response key-value surfaces must keep stable independent scroll handles"
        );
        assert_always_visible_scrollbars(production_source, "the API request and response panes");
        // 新 gpui-component 中多行编辑器迁移到 Textarea(内建滚动),raw body 与脚本编辑器保持多行输入
        assert!(
            production_source.contains("Textarea::new(&self.body_input)")
                && production_source.contains("fn render_script_editor")
                && production_source.contains("Textarea::new(input)"),
            "raw request bodies and request scripts must stay multi-line editors with scrolling"
        );
        for (scope_name, protocol_render) in [
            ("WebSocket", websocket_render),
            ("Socket.IO", socket_io_render),
            ("TCP", tcp_render),
        ] {
            assert_always_visible_scrollbars(protocol_render, scope_name);
            assert!(
                protocol_render.contains("Textarea::new(&self.websocket_message_input)")
                    || protocol_render.contains("Textarea::new(&self.socket_io_message_input)")
                    || protocol_render.contains("Textarea::new(&self.tcp_message_input)"),
                "{scope_name} message editors must stay multi-line editors with scrolling"
            );
        }
        assert!(
            production_source.contains("ButtonCustomVariant")
                && production_source.contains("Tag::custom")
                && production_source.contains("method_badge_color")
                && production_source.contains("method_fill_color"),
            "the API client must keep theme-aware method and status styling"
        );
        assert!(
            render.contains("api-new-folder") && render.contains("new_folder"),
            "the sidebar must expose folder creation"
        );
        assert!(
            production_source.contains("api-folder-editor")
                && production_source.contains("api-folder-base-url")
                && production_source.contains("FolderVariables"),
            "folder selection must expose Base URL and inherited variable settings"
        );
        assert!(
            production_source.contains("IconName::ChevronDown")
                && production_source.contains("IconName::ChevronRight")
                && production_source.contains("IconName::FolderOpen")
                && production_source.contains("IconName::FolderClosed"),
            "the request tree must use standard folder and disclosure icons"
        );
        assert!(
            production_source.contains("IconName::Delete")
                && production_source.contains("IconName::Plus")
                && !production_source.contains(".label(\"×\")")
                && !production_source.contains(".label(\"+\")"),
            "tree and key-value actions must use standard icons instead of raw glyph labels"
        );
        assert!(
            render.contains(".min_h_0()") && render.contains(".overflow_hidden()"),
            "the root layout must clip intrinsic editor content"
        );
        assert!(
            render.contains(".flex_1()") && render.contains(".h_full()"),
            "the active multi-line editor must fill its pane"
        );
        assert!(
            render.contains("h_resizable(\"api-test-horizontal\")")
                && render.contains("v_resizable(\"api-test-vertical\")"),
            "request tree, editor, and response areas must be resizable"
        );
        assert!(
            render.contains("search_input") && render.contains("IconName::Search"),
            "the request tree must expose search"
        );
        assert!(
            behavior.contains("render_new_request_button")
                && behavior.contains("Protocol::ALL")
                && behavior.contains("new_request_with_protocol")
                && behavior.contains("req.protocol = protocol"),
            "the new-request popover must create the selected protocol"
        );
        assert!(
            environment_manager.contains("Popover::new(\"api-environment-switcher\")")
                && environment_manager.contains("api-environment-switcher-content")
                && environment_manager.contains("api-new-environment-quick")
                && environment_manager.contains("window.open_dialog")
                && environment_manager.contains(".overlay(true)")
                && environment_manager.contains(".overlay_closable(true)")
                && environment_manager.contains("api-environment-manager-content")
                && environment_manager.contains("api-environment-manager-close")
                && environment_manager.contains("theme.sidebar_accent_foreground")
                && environment_manager.contains("theme.popover_foreground")
                && environment_manager.contains("api-environment-list-scroll")
                && environment_manager.contains("api-environment-settings-scroll")
                && environment_manager.matches("Scrollbar::vertical(").count() >= 3
                && !environment_manager.contains("Popover::new(\"api-environment-manager\")")
                && environment_manager.contains("this.select_environment")
                && behavior.contains("prompt_new_environment")
                && behavior.contains("prompt_rename_active_environment")
                && behavior.contains("create_environment_named")
                && behavior.contains("rename_active_environment_named")
                && behavior.contains("window.open_dialog")
                && behavior.contains("rebuild_environment_select")
                && behavior.contains("refresh_environment_settings")
                && behavior.contains("environment_base_url_input")
                && behavior.contains("EnvironmentParams")
                && behavior.contains("EnvironmentHeaders")
                && behavior.contains("EnvironmentCookies"),
            "environment management must support switching, lifecycle actions, and scoped request settings"
        );
        assert!(
            environment_manager.contains("settings_section")
                && environment_manager.contains(".h(px(220.))")
                && environment_manager.contains("self.render_kv_editor(section, cx)")
                && environment_manager.contains(".child(environment_variables_section)")
                && environment_manager.contains(".child(environment_headers_section)")
                && environment_manager.contains(".child(environment_params_section)")
                && environment_manager.contains(".child(environment_cookies_section)"),
            "environment settings must use full-width stacked key-value editors"
        );
        for editor in [
            "\"api-environment-variables\"",
            "\"api-environment-params\"",
            "\"api-environment-headers\"",
            "\"api-environment-cookies\"",
            "Input::new(&self.environment_base_url_input)",
        ] {
            assert_eq!(
                production_source.matches(editor).count(),
                1,
                "environment input entities must only be mounted once: {editor}"
            );
        }
        assert_eq!(
            production_source
                .matches("Button::new(\"api-delete-environment\")")
                .count(),
            1,
            "the environment delete action must not duplicate its element id"
        );
        assert!(
            render.contains("api-kv-empty-")
                && render.contains("api-kv-add-footer-")
                && render.contains(".appearance(false)")
                && render.contains("this.add_kv_row(section_for_closure, window, cx)"),
            "the key-value table must use integrated cells and expose real add-row actions"
        );
        for contract in [
            "api-protocol-select",
            "api-sidebar-toggle",
            "api-sidebar-collections",
            "api-sidebar-history",
            "api-history-list",
            "api-environment-select",
            "api-environment-switcher",
            "api-environment-switcher-trigger",
            "api-environment-manager-content",
            "api-environment-manager-trigger",
            "api-environment-manager-close",
            "api-new-environment",
            "api-new-environment-quick",
            "api-rename-environment",
            "api-delete-environment",
            "api-import-collection",
            "api-export-collection",
            "api-form-file-picker",
        ] {
            assert!(
                production_source.contains(contract),
                "missing API client capability contract: {contract}"
            );
        }
        assert!(
            behavior.contains("sidebar_collapsed")
                && render.contains("if self.sidebar_collapsed")
                && render.contains("render_sidebar_toggle_handle")
                && render.contains("REQUEST_TREE_COLLAPSED_WIDTH")
                && render.contains("IconName::ChevronLeft")
                && render.contains("IconName::ChevronRight"),
            "the request sidebar must remain collapsible through an edge handle on narrow windows"
        );
        assert!(
            behavior.contains("prompt_for_paths")
                && behavior.contains("schema_io::import_collection")
                && behavior.contains("schema_io::export_openapi")
                && behavior.contains("schema_io::export_swagger")
                && behavior.contains("OpenApiYaml")
                && behavior.contains("SwaggerYaml")
                && production_source.contains("dropdown_menu_with_anchor"),
            "file import/export and file upload must use the existing IO capabilities"
        );
        assert!(
            behavior.contains("request_generation = self.request_generation.wrapping_add(1)"),
            "request changes must invalidate in-flight responses"
        );
        assert!(
            behavior.contains("current_protocol")
                && render.contains("uses_http_method")
                && behavior.contains("ProtocolOption"),
            "the request bar must select a protocol and only show methods for HTTP-shaped protocols"
        );
        assert!(
            behavior.contains("cx.spawn_in(window") && behavior.contains("this.update_in(cx"),
            "responses must update through the window-aware async context"
        );
        assert!(
            behavior.contains("stream_stop")
                && send_behavior.contains("send_sse")
                && render.contains("ApiTest.stop"),
            "SSE must expose incremental streaming and a stop action"
        );
        assert!(
            behavior.contains("websocket_state")
                && behavior.contains("websocket_generation")
                && send_behavior.contains("connect_websocket")
                && render.contains("render_websocket_response")
                && websocket_render.contains("api-websocket-timeline")
                && websocket_render.contains("api-websocket-message-input")
                && websocket_render.contains("api-websocket-send-message")
                && render.contains("ApiTest.connect")
                && render.contains("ApiTest.disconnect"),
            "WebSocket must expose a generation-safe connection lifecycle and message timeline"
        );
        assert!(
            behavior.contains("socket_io_state")
                && behavior.contains("socket_io_generation")
                && send_behavior.contains("connect_socket_io")
                && render.contains("render_socket_io_response")
                && socket_io_render.contains("api-socketio-timeline")
                && socket_io_render.contains("api-socketio-message-input")
                && socket_io_render.contains("api-socketio-send-message")
                && render.contains("ApiTest.connect")
                && render.contains("ApiTest.disconnect"),
            "Socket.IO must expose an EIO4-aware, generation-safe lifecycle and event timeline"
        );
        assert!(
            behavior.contains("request.protocol")
                && behavior.contains("badge_label")
                && render.contains("render_request_badge"),
            "request tree and history badges must distinguish non-HTTP protocols"
        );
        assert!(
            behavior.matches("refresh_environment_settings").count() >= 3,
            "environment changes and script effects must refresh the complete environment editor"
        );
        assert!(
            response_renderer.contains("r.headers.clone()")
                && response_renderer.contains("eq_ignore_ascii_case(\"set-cookie\")")
                && response_renderer.contains("response_cookie_pair")
                && response_renderer.contains("ApiTest.no_response_headers")
                && response_renderer.contains("ApiTest.no_response_cookies"),
            "response header and cookie tabs must read transport headers and expose explicit empty states"
        );
    }
}
