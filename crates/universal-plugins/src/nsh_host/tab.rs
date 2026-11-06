//! 承载一个扩展 nsh app 的 tab。
//!
//! 装载分两段，和 gpui-shell 那侧同构：先**租**provider（激活后端 runtime，
//! 拿到 activation lease），租到手再挂页面。顺序不能倒 —— 页面 `mount` 的
//! 第一行就可能 `host.call` 要数据，provider 没起来就只能拿到错误。
//!
//! 所以 tab 先以"激活中"的样子开出来，激活结果回到主线程后，下一次
//! `render` 才真正建 `AppView`。租约跟着 tab 走：tab 关掉（Drop）就还回去，
//! 中间任何一步失败也把已经租到的那些还掉。

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use extension_host::CancellationToken;
use extension_plugin_adapter::ActivationHandle;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Task, Window, div,
};
use gpui_component::{ActiveTheme as _, Theme};
use nsh_engine::HostChannel;
use nsh_platform_gpui::{AppOptions, AppView, Locale};
use serde_json::{Map, Value, json};

use super::{Trust, abi, bundle, session::NshSession, theme};

/// 一个扩展的 nsh app 实例（一个 `AppView` + 它的 manifest 元数据 + 初始路由）。
pub struct NshTab {
    /// 激活 lease 用：还租约要在 tokio 上 await。
    service: crate::UniversalPluginService,
    tokio: tokio::runtime::Handle,
    /// 签名包验签要信任库；它是主线程独占的（`Rc<RefCell<_>>`），所以
    /// 装载动作只在本结构所在线程上做。
    trust: Trust,
    contribution: extension_runtime::RegisteredShellAppContribution,
    /// 页面的扩展身份：host 会话按它建，关闭与排障也要它。
    extension_id: SharedString,
    /// 打开这个 tab 时 app 的初始路由（`host.params.route`）。
    route: Option<SharedString>,
    title: SharedString,
    view: Option<Entity<AppView>>,
    /// 装载失败时的原因：tab 照样开出来，把错误写在页面上，而不是静默失败。
    error: Option<String>,
    /// 激活结果；`Some` 表示 provider 那一段已经有了结论，等下一次 render 落地。
    prepared: Option<Result<Vec<ActivationHandle>, String>>,
    /// 当前持有的 provider lease。tab 关掉时一次性还回去。
    activations: Vec<ActivationHandle>,
    /// 激活任务的取消位（tab 在激活完成前就被关掉时用）。
    cancel: CancellationToken,
    /// 激活任务的句柄：留着它，任务才不会被立刻丢弃。
    _prepare: Option<Task<()>>,
    /// 首帧健康检查（签名包两阶段装载的第二步）。
    _accept: Option<Task<()>>,
    /// 主题/明暗变化时把新 token 推给页面。
    _theme: Subscription,
    /// 页面可见的宿主能力（`host.call("navop.*")`）。关闭时置取消位。
    abi: Option<Arc<abi::HostAbi>>,
    /// 与页面通行的 `host.call` 通道。
    ///
    /// 在 `open` 就建好、显式交给 `AppOptions.host`，而不是等 `AppView`
    /// 起来再取 `AppView::host()` —— 页面 `mount` 第一行就可能调宿主，
    /// handler 必须先于页面就位。这也让调用方（含台子）能在挂载前挂 handler。
    channel: HostChannel,
    /// provider 事件流 → `navop.event` 推送的泵（v2 的唯一推送平面）。
    pump: Option<Arc<EventPump>>,
    /// 能力平面会话：provider 句柄记账与取消。页面关闭即 drop，句柄随之回收。
    session: Option<Arc<NshSession>>,
    /// 引擎线程 → 主线程的桥（`navop.dev` 这类操作要回主线程跑）。
    dispatcher: super::dispatch::MainThread,
    /// 连接型页面：provider 资源由本 tab 打开并持有（`owned: true`）。
    connection: Option<super::connection::NshConnectionLaunch>,
    /// 本页是不是连接型页面（连接启动参数在准备阶段就被消费掉了，
    /// 重装时要靠这个标记区分"能重来"和"要重开连接"）。
    is_connection: bool,
    /// 连接资源准备好了但还没交给会话（建会话要等窗口）。
    prepared_connection: Option<super::connection::PreparedNshConnection>,
    /// 插件 AI 侧栏（plugin-ai-tools.md §9）：每 tab 一个实例，默认收起。
    /// 页面装载成功后创建；会话 v1 只在内存里，tab 关掉即丢。
    plugin_ai: Option<Entity<super::plugin_ai_panel::PluginAiPanel>>,
}

impl NshTab {
    pub(crate) fn open(
        contribution: extension_runtime::RegisteredShellAppContribution,
        route: Option<String>,
        service: crate::UniversalPluginService,
        trust: Trust,
        dispatcher: super::dispatch::MainThread,
        cx: &mut App,
    ) -> Entity<Self> {
        Self::create(
            contribution,
            route,
            service,
            trust,
            dispatcher,
            None,
            None,
            cx,
        )
    }

    /// 连接型页面：先用连接配置打开 provider 资源，再把页面挂在这个会话上。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open_connection(
        contribution: extension_runtime::RegisteredShellAppContribution,
        route: Option<String>,
        service: crate::UniversalPluginService,
        trust: Trust,
        dispatcher: super::dispatch::MainThread,
        connection: super::connection::NshConnectionLaunch,
        title: SharedString,
        cx: &mut App,
    ) -> Entity<Self> {
        Self::create(
            contribution,
            route,
            service,
            trust,
            dispatcher,
            Some(connection),
            Some(title),
            cx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create(
        contribution: extension_runtime::RegisteredShellAppContribution,
        route: Option<String>,
        service: crate::UniversalPluginService,
        trust: Trust,
        dispatcher: super::dispatch::MainThread,
        connection: Option<super::connection::NshConnectionLaunch>,
        connection_title: Option<SharedString>,
        cx: &mut App,
    ) -> Entity<Self> {
        let title: SharedString = connection_title
            .clone()
            .unwrap_or_else(|| contribution.title.clone().into());
        let extension_id: SharedString = contribution.extension_id.clone().into();
        let route: Option<SharedString> = route.map(Into::into);
        let tokio = one_core::gpui_tokio::Tokio::handle(cx);
        let channel = HostChannel::default();

        let tab: Entity<Self> = cx.new(|cx| {
            let _theme = cx.observe_global::<Theme>(move |this: &mut Self, cx| {
                // v2 的主题走 nsh 原生（`set_theme` / `set_appearance`）：
                // 不再向页面推 `navop.theme.changed`，页面也不读 navop.theme.*。
                if let Some(view) = &this.view {
                    theme::push(view, cx);
                }
            });
            Self {
                service,
                tokio,
                trust,
                contribution,
                extension_id,
                route,
                title,
                channel,
                pump: None,
                view: None,
                error: None,
                prepared: None,
                activations: Vec::new(),
                cancel: CancellationToken::new(),
                _prepare: None,
                _accept: None,
                _theme,
                abi: None,
                session: None,
                dispatcher,
                is_connection: connection.is_some(),
                connection,
                prepared_connection: None,
                plugin_ai: None,
            }
        });
        tab.update(cx, |tab, cx| tab.start_prepare(cx));
        tab
    }

    /// 第一段：把页面声明的后端一个个激活（= 向激活管理器要租约）。
    fn start_prepare(&mut self, cx: &mut Context<Self>) {
        let backends: Vec<String> = self.contribution.backends.values().cloned().collect();
        if backends.is_empty() && self.connection.is_none() {
            // 不碰后端就没有租约可等，直接进入装载。
            self.prepared = Some(Ok(Vec::new()));
            cx.notify();
            return;
        }
        let service = self.service.clone();
        let cancel = self.cancel.clone();
        let connection = self.connection.take();
        let (sender, receiver) = tokio::sync::oneshot::channel::<
            Result<
                (
                    Vec<ActivationHandle>,
                    Option<super::connection::PreparedNshConnection>,
                ),
                String,
            >,
        >();
        self.tokio.spawn(async move {
            let mut activations = Vec::new();
            for runtime_id in backends {
                if cancel.is_cancelled() {
                    release(&service, activations).await;
                    let _ = sender.send(Err("页面在 provider 激活完成前就关闭了".to_owned()));
                    return;
                }
                match service.activate_runtime(&runtime_id).await {
                    Ok(handle) => activations.push(handle),
                    Err(error) => {
                        release(&service, activations).await;
                        let _ =
                            sender.send(Err(format!("激活 provider `{runtime_id}` 失败：{error}")));
                        return;
                    }
                }
            }
            // 连接型页面：provider 起来之后立刻按连接配置打开主资源。
            // 这一步失败就没有页面可言（页面第一件事就是读这个连接），
            // 所以和激活失败同样处理：还租约、报错、不挂半成品。
            if let Some(launch) = connection {
                match super::connection::open_connection_resource(&service, launch, &cancel).await {
                    Ok(prepared) => {
                        let _ = sender.send(Ok((activations, Some(prepared))));
                    }
                    Err(error) => {
                        release(&service, activations).await;
                        let _ = sender.send(Err(format!("打开连接资源失败：{error:#}")));
                    }
                }
                return;
            }
            let _ = sender.send(Ok((activations, None)));
        });
        self._prepare = Some(cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("provider 激活任务被丢弃".to_owned()));
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok((activations, connection)) => {
                        this.activations = activations;
                        this.prepared_connection = connection;
                        this.prepared = Some(Ok(Vec::new()));
                    }
                    Err(error) => this.prepared = Some(Err(error)),
                }
                cx.notify();
            });
        }));
    }

    /// 第二段：provider 已经起来了，把页面挂上去。
    ///
    /// 只在 `render` 里调用 —— 建 `AppView` 要 `&mut Window`，而激活完成是
    /// 异步回来的，只能等下一帧拿到窗口。
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let program = match bundle::load(&self.contribution, &self.trust) {
            Ok(program) => program,
            Err(error) => {
                tracing::error!(
                    extension_id = %self.extension_id,
                    route = %self.route_display(),
                    path = %self.contribution.entry_path.display(),
                    "nsh 页面装载失败: {error:#}"
                );
                self.error = Some(format!("{error:#}"));
                return;
            }
        };
        // 页面能力要在页面起来之前就有：handler 必须挂在它的通道上，
        // 而页面启动的第一行就可能 `host.call`。
        let host_abi = abi::HostAbi::new(&self.contribution, self.route.as_deref());
        let session = Arc::new(NshSession::new(
            self.service.clone(),
            self.contribution.backends.clone(),
            self.tokio.clone(),
        ));
        let accepted = program.accepted;
        // 连接型页面：资源句柄仍旧交给会话持有；连接事实本身进 `host.params`
        // —— v2 删了 `navop.connection.current`，params 是页面的唯一来源。
        let connection = match self.prepared_connection.take() {
            Some(prepared) => match prepared.adopt(&session) {
                Ok(context) => context,
                Err(error) => {
                    // 连接资源没接上：页面照常起来，但 `params.connection`
                    // 只能是 null（spec §2）。
                    tracing::warn!(
                        extension_id = %self.extension_id,
                        route = %self.route_display(),
                        "连接资源接入失败，params.connection 置空: {error:#}"
                    );
                    Value::Null
                }
            },
            None => Value::Null,
        };
        // 启动事实要在 `AppOptions` / `cx.new(AppView::new)` 之前写进通道：
        // 页面顶层同步读 `host.params`，首帧就要看到完整的一份（spec §2）。
        self.channel.set_params(params_payload(
            &self.contribution,
            self.route.as_deref(),
            connection,
        ));
        let options = AppOptions {
            app_id: program.app_id,
            asset_base: program.asset_base,
            host: self.channel.clone(),
            // 原生组件（`<Terminal/>` 这类）按**本页面**的注册表注入：
            // 每视图一份，工厂里钉死扩展身份与清单权限。
            components: super::native::components_for(
                &self.contribution,
                self.route.as_deref(),
                cx,
            ),
            ..AppOptions::default()
        };
        // 事件泵先于 handler 建好：`navop.event.subscribe` 会把订阅交给它。
        let pump = EventPump::new(self.channel.clone(), Arc::clone(&session));

        // 插件 AI 侧栏（§9）：页面装载成功后建好（默认收起，随时可开）。
        // 连接型页面回查完整 StoredConnection（build_sidebar_resource_state
        // 需要它），查不到按"无连接"处理。先建面板再注册 ABI：navop.ai.*
        // 的 handler 需要面板句柄。
        let connection = self.connection.as_ref().and_then(|launch| {
            use one_core::storage::traits::Repository as _;
            let id = launch.connection_id();
            cx.try_global::<one_core::storage::GlobalStorageState>()
                .and_then(|state| state.storage.get::<one_core::storage::ConnectionRepository>())
                .and_then(|repo| repo.get(id).ok().flatten())
        });
        let contribution_title = self.contribution.title.clone();
        let extension_id: Arc<str> = Arc::from(self.extension_id.as_str());
        let route: Option<Arc<str>> = self.route.as_deref().map(Arc::from);
        let service = self.service.clone();
        let plugin_ai = cx.new(|cx| {
            super::plugin_ai_panel::PluginAiPanel::new(
                &service,
                extension_id,
                Arc::from(contribution_title.as_str()),
                route,
                connection,
                window,
                cx,
            )
        });
        // 先把能力挂到通道上，再让页面跑起来。
        abi::register(
            &self.channel,
            Arc::clone(&host_abi),
            Arc::clone(&session),
            &pump,
            &self.dispatcher,
            Some(plugin_ai.downgrade()),
        );
        // 侧栏展开/收起改变的是 tab 的布局（展开时占宽度、收起时只剩浮层按钮），
        // 所以要跟着侧栏的更新重画这个 tab。
        cx.observe(&plugin_ai, |_this, _panel, cx| cx.notify())
            .detach();
        self.plugin_ai = Some(plugin_ai);
        let view = cx.new(|cx| AppView::new(options, window, cx));
        // 首帧前把宿主主题与界面语言交给页面（spec §2 附、§7 主题走原生）。
        theme::push(&view, cx);
        let locale = rust_i18n::locale().to_string();
        let locale = Locale::parse(&locale).unwrap_or(Locale::EN_US);
        view.update(cx, |view, cx| view.set_locale(Some(locale), cx));

        let failed = Arc::new(AtomicBool::new(false));
        let failed_for_error = Arc::clone(&failed);
        let extension_id = self.extension_id.clone();
        let route = self.route_display().to_owned();
        view.update(cx, |view, _cx| {
            view.on_error(move |message| {
                failed_for_error.store(true, Ordering::Relaxed);
                tracing::error!(
                    extension_id = %extension_id,
                    route = %route,
                    "nsh app 错误: {message}"
                );
            });
            view.load(program.app);
        });

        self.abi = Some(host_abi);
        self.session = Some(session);
        self.pump = Some(pump);
        self.view = Some(view);



        // 两阶段装载的第二步：首帧画出来、且没有立刻报错，才算这个 build
        // 健康，把它记进信任库（方案 §8）。失败就让上一个 build 继续可用。
        if let Some(loaded) = accepted {
            let trust = self.trust.clone();
            self._accept = Some(cx.spawn(async move |_this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(600))
                    .await;
                if failed.load(Ordering::Relaxed) {
                    tracing::warn!(
                        app = %loaded.app.id,
                        build = loaded.app.build,
                        "nsh 包首帧报错，不接受这个 build"
                    );
                    return;
                }
                let _ = cx.update(|_cx| bundle::accept(&loaded, &trust));
            }));
        }
    }
}

/// `host.params` 的载荷（契约 §3）：启动事实，运行期不变。
///
/// 页面在顶层同步读它，所以字段名一旦定下就是契约：`apiVersion` /
/// `extension.{id,title}` / `route` / `backends` / `connection`。
///
/// `route` 是宿主建议的初始页面（来自连接清单的 `route` 或打开方给的
/// deep link）；页面可以用它当默认值，也可以在 app 内继续导航。
fn params_payload(
    contribution: &extension_runtime::RegisteredShellAppContribution,
    route: Option<&str>,
    connection: Value,
) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("apiVersion".to_owned(), json!(2));
    params.insert(
        "extension".to_owned(),
        json!({
            "id": contribution.extension_id,
            "title": contribution.title,
        }),
    );
    params.insert("route".to_owned(), json!(route));
    params.insert(
        "backends".to_owned(),
        json!(contribution.backends.keys().collect::<Vec<&String>>()),
    );
    params.insert("connection".to_owned(), connection);
    params
}

/// provider 事件流 → `navop.event` 推送的泵（spec §3）。
///
/// `host.call("navop.event.subscribe")` 打开一条 provider 事件流之后，这里用
/// 一条 tokio 任务持续 `read_event`，把每批包成
/// `{handle,cursor,events:[{seq,data}]}` 推给页面；流结束推
/// `{handle,cursor,done:true}`，溢出推 `EVENT_OVERFLOW` 并结束订阅。
pub(crate) struct EventPump {
    channel: HostChannel,
    session: Arc<NshSession>,
    /// 在订的订阅：handle → 泵任务的取消位（`unsubscribe` / 页面关闭时掐）。
    subscriptions: Mutex<HashMap<String, CancellationToken>>,
}

/// 单批最多 64 条（spec §3）。
const EVENT_BATCH: u32 = 64;
/// 拉空一批后最多等这么久：provider 不支持阻塞读时也不会空转。
const EVENT_POLL_MS: u32 = 500;

impl EventPump {
    pub(crate) fn new(channel: HostChannel, session: Arc<NshSession>) -> Arc<Self> {
        Arc::new(Self {
            channel,
            session,
            subscriptions: Mutex::new(HashMap::new()),
        })
    }

    /// 订阅一条事件流：回 `{handle,cursor}`，事件随后经 `navop.event` 推送。
    pub(crate) async fn subscribe(
        self: Arc<Self>,
        resource: String,
        kind: String,
        since: u64,
        cancel: CancellationToken,
    ) -> Result<Value, crate::host_error::NavopError> {
        let opened = self
            .session
            .open_event(&resource, kind, None, cancel)
            .await?;
        // 先取出句柄再动 `opened`：下面出错时要把整个结果塞进 details。
        let opened_handle = opened
            .get("handle")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let Some(handle) = opened_handle else {
            let mut error = crate::host_error::NavopError::new(
                crate::host_error::ErrorCode::ProtocolError,
                "provider 的 event open 结果没有 handle",
            );
            error.details = Some(opened);
            return Err(error);
        };
        // 泵用独立的取消位：`unsubscribe` 只停这一条，不影响别的在途调用。
        let token = self.session.call_token();
        self.subscriptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(handle.clone(), token.clone());
        let pump = Arc::clone(&self);
        let channel = self.channel.clone();
        let session = Arc::clone(&self.session);
        // 泵任务拿走一份句柄；`handle` 本身还要用来回订阅应答。
        let stream = handle.clone();
        let subscription = handle.clone();
        session.tokio().spawn(async move {
            let mut cursor = since;
            loop {
                if token.is_cancelled() {
                    break;
                }
                match session
                    .read_event(
                        &stream,
                        Some(EVENT_BATCH),
                        Some(EVENT_POLL_MS),
                        token.clone(),
                    )
                    .await
                {
                    Ok(value) => {
                        let dropped = value
                            .get("dropped_count")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        let closed = value
                            .get("closed")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let events = value
                            .get("events")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default();
                        // 先把这一批送到：溢出丢的是更早的事件，手上这批
                        // 仍然有效，不该跟着一起扔。
                        if !events.is_empty() {
                            let batch: Vec<Value> = events
                                .into_iter()
                                .map(|data| {
                                    cursor += 1;
                                    json!({ "seq": cursor, "data": data })
                                })
                                .collect();
                            let frame = json!({
                                "handle": subscription.clone(),
                                "cursor": cursor,
                                "events": batch,
                            });
                            if channel.send("navop.event", frame).is_err() {
                                break;
                            }
                        }
                        if dropped > 0 {
                            // 页面拿 `cursor` 作 since 续订；这条 error 之后
                            // 订阅就结束了（spec §3）。
                            let _ = channel.send(
                                "navop.event",
                                json!({
                                    "handle": subscription.clone(),
                                    "cursor": cursor,
                                    "error": {
                                        "code": "EVENT_OVERFLOW",
                                        "message": format!("事件缓冲溢出，丢弃了 {dropped} 条"),
                                        "retryable": true,
                                    },
                                }),
                            );
                            break;
                        }
                        if closed {
                            let _ = channel.send(
                                "navop.event",
                                json!({
                                    "handle": subscription.clone(),
                                    "cursor": cursor,
                                    "done": true,
                                }),
                            );
                            break;
                        }
                    }
                    Err(error) => {
                        if token.is_cancelled() {
                            break;
                        }
                        let _ = channel.send(
                            "navop.event",
                            json!({
                                "handle": subscription.clone(),
                                "cursor": cursor,
                                "error": {
                                    "code": error.code.as_str(),
                                    "message": error.message,
                                    "retryable": error.retryable,
                                },
                            }),
                        );
                        break;
                    }
                }
            }
            pump.forget(&stream);
        });
        Ok(json!({ "handle": handle, "cursor": since }))
    }

    /// 取消一条订阅：泵任务看到取消位后自行退出。
    pub(crate) fn unsubscribe(&self, handle: &str) {
        if let Some(token) = self
            .subscriptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(handle)
        {
            token.cancel();
        }
    }

    /// 页面关闭：停掉全部订阅。
    pub(crate) fn cancel(&self) {
        for (_, token) in self
            .subscriptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
        {
            token.cancel();
        }
    }

    fn forget(&self, handle: &str) {
        self.subscriptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(handle);
    }
}

impl Drop for NshTab {
    fn drop(&mut self) {
        // tab 关掉：先掐掉还在路上的激活与在途调用，再还不租约。
        // 这里没有 `cx`，所以直接用 tokio 句柄异步还；Drop 不能 await。
        self.cancel.cancel();
        if let Some(abi) = &self.abi {
            abi.cancel();
        }
        if let Some(pump) = &self.pump {
            pump.cancel();
        }
        if let Some(session) = &self.session {
            session.cancel();
        }
        let activations = std::mem::take(&mut self.activations);
        if !activations.is_empty() {
            let service = self.service.clone();
            self.tokio.spawn(async move {
                release(&service, activations).await;
            });
        }
    }
}

/// 归还一组租约。失败只记日志：租约还不上不该拦着页面关闭。
async fn release(service: &crate::UniversalPluginService, activations: Vec<ActivationHandle>) {
    for handle in &activations {
        if let Err(error) = service.deactivate_activation(handle).await {
            tracing::warn!(%error, runtime_id = %handle.runtime_id, "nsh 页面释放 provider 租约失败");
        }
    }
}

impl EventEmitter<one_core::tab_container::TabContentEvent> for NshTab {}

impl Focusable for NshTab {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.view {
            Some(view) => view.read(cx).focus_handle(cx),
            None => cx.focus_handle(),
        }
    }
}

impl Render for NshTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 激活有了结论就落地装载。只在加载中且还没建页面时做一次。
        if self.view.is_none() && self.error.is_none() {
            if let Some(result) = self.prepared.take() {
                match result {
                    Ok(activations) => {
                        self.activations = activations;
                        self.load(window, cx);
                    }
                    Err(error) => {
                        tracing::error!(
                            extension_id = %self.extension_id,
                            route = %self.route_display(),
                            "nsh 页面 provider 激活失败: {error}"
                        );
                        self.error = Some(error);
                    }
                }
            }
        }
        let mut root = div().size_full().min_w_0().min_h_0();
        match (&self.view, self.error.as_deref()) {
            (Some(view), _) => {
                // 页面本体占满内容区，AI 侧栏只在展开时让出宽度。收起态以前由
                // 侧栏自己渲染 size_full，会把页面挤成 0 宽（整个页面空白），
                // 所以收起时改成 tab 上的右下角浮层按钮。
                let mut row = gpui_component::h_flex()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .h_full()
                            .child(view.clone()),
                    );
                let mut collapsed_panel = None;
                if let Some(panel) = self.plugin_ai.clone() {
                    if panel.read(cx).is_open() {
                        row = row.child(panel);
                    } else {
                        collapsed_panel = Some(panel);
                    }
                }
                root = root.relative().child(row);
                if let Some(panel) = collapsed_panel {
                    let fab_panel = panel.clone();
                    root = root.child(
                        div()
                            .id("plugin-ai-fab-host")
                            .absolute()
                            .bottom_4()
                            .right_4()
                            .child(
                                div()
                                    .id("plugin-ai-fab")
                                    .on_click(move |_event, _window, cx: &mut App| {
                                        fab_panel
                                            .update(cx, |panel, cx| panel.open_panel(cx));
                                    })
                                    .flex()
                                    .items_center()
                                    .px_3()
                                    .py_2()
                                    .rounded_full()
                                    .bg(cx.theme().primary)
                                    .text_color(cx.theme().primary_foreground)
                                    .text_sm()
                                    .child("AI"),
                            ),
                    );
                }
            }
            (None, Some(error)) => {
                root = root
                    .p_4()
                    .text_sm()
                    .child(SharedString::from(format!("nsh 页面装载失败：\n{error}")));
            }
            (None, None) => {}
        }
        root
    }
}

impl one_core::tab_container::TabContent for NshTab {
    fn content_key(&self) -> &'static str {
        "NshHost"
    }

    fn title(&self, _cx: &App) -> SharedString {
        self.title.clone()
    }

    fn can_rename(&self, _cx: &App) -> bool {
        false
    }
}

impl NshTab {
    pub(crate) fn extension_id(&self) -> &SharedString {
        &self.extension_id
    }

    pub(crate) fn route(&self) -> Option<&SharedString> {
        self.route.as_ref()
    }

    /// 日志用：没有初始路由时显示 `<default>`。
    fn route_display(&self) -> &str {
        self.route.as_deref().unwrap_or("<default>")
    }

    /// 与页面通行的宿主通道。调用方可以在挂载前挂 handler。
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "台子挂 handler 用；生产路径 M6 接线")
    )]
    /// 扩展停用：关掉页面、还租约，并请 tab 容器移除这一项。
    pub(crate) fn close_for_extension(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        self.cancel_flight();
        self.view = None;
        let stale = std::mem::take(&mut self.activations);
        let service = self.service.clone();
        let release = one_core::gpui_tokio::Tokio::spawn_result(cx, async move {
            release(&service, stale).await;
            Ok(())
        });
        cx.spawn(async move |this, cx| {
            let closed = release.await.is_ok();
            if closed {
                let _ = this.update(cx, |_, cx| {
                    cx.emit(one_core::tab_container::TabContentEvent::CloseRequested)
                });
            }
            closed
        })
    }

    /// provider 事件：换进程就重装页面，被移除就停在错误页。
    pub(crate) fn runtime_changed(
        &mut self,
        event: &extension_plugin_adapter::RuntimeMonitorEvent,
        cx: &mut Context<Self>,
    ) {
        use crate::nsh_host::decision::{RuntimeChangeDecision, decide_runtime_change};

        let decision = self
            .contribution
            .backends
            .values()
            .map(|runtime_id| {
                let activation_generation = self
                    .activations
                    .iter()
                    .find(|activation| &activation.runtime_id == runtime_id)
                    .map(|activation| activation.runtime_generation);
                let current_generation = self.service.runtime_generation(runtime_id).ok();
                decide_runtime_change(
                    event,
                    runtime_id,
                    false,
                    activation_generation,
                    current_generation,
                )
            })
            .find(|decision| *decision != RuntimeChangeDecision::Ignore);
        match decision {
            None | Some(RuntimeChangeDecision::Ignore) => {}
            Some(RuntimeChangeDecision::Fail) => {
                self.fail(
                    "provider 已被移除或无法自动重启，请关闭这个页面后重新打开",
                    cx,
                );
            }
            Some(RuntimeChangeDecision::Reconnect) => {
                if self.is_connection {
                    // 连接型页面的启动参数（连接配置 + 凭据引用）在准备阶段
                    // 就交给 provider 了，本结构没有可以重放的东西。
                    self.fail("provider 已重启，请重新打开这条连接", cx);
                    return;
                }
                self.remount(cx);
            }
        }
    }

    /// 换掉页面与它的会话，重新走一遍准备流程。
    fn remount(&mut self, cx: &mut Context<Self>) {
        self.cancel_flight();
        self.view = None;
        self.prepared_connection = None;
        self.prepared = None;
        self.error = None;
        self.cancel = CancellationToken::new();
        // 旧 lease 随旧进程一起失效，归还只是让记账对上（失败也无所谓）。
        let stale = std::mem::take(&mut self.activations);
        if !stale.is_empty() {
            let service = self.service.clone();
            self.tokio
                .spawn(async move { release(&service, stale).await });
        }
        self.start_prepare(cx);
        cx.notify();
    }

    fn fail(&mut self, message: &str, cx: &mut Context<Self>) {
        tracing::warn!(
            extension_id = %self.extension_id,
            route = %self.route_display(),
            "{message}"
        );
        self.cancel_flight();
        self.view = None;
        self.error = Some(message.to_owned());
        cx.notify();
    }

    /// 让在途的 `host.call` 与 provider 调用按 `REQUEST_CANCELLED` 退出。
    fn cancel_flight(&mut self) {
        self.cancel.cancel();
        if let Some(abi) = self.abi.take() {
            abi.cancel();
        }
        if let Some(pump) = self.pump.take() {
            pump.cancel();
        }
        if let Some(session) = self.session.take() {
            session.cancel();
        }
    }

    /// 界面语言变化后把新 locale 交给页面（触发重渲染，页面重新取词条）。
    pub(crate) fn apply_locale(&self, locale: &str, cx: &mut App) {
        let Some(view) = &self.view else {
            return;
        };
        // nsh 的 Locale 是定长 BCP-47 标签（`en-US`、`zh-CN`）；宿主侧
        // rust-i18n 用的是 `en` / `zh-CN` 这种短标签，解析不出来的回退英文。
        let locale =
            nsh_platform_gpui::Locale::parse(locale).unwrap_or(nsh_platform_gpui::Locale::EN_US);
        view.update(cx, |view, cx| view.set_locale(Some(locale), cx));
    }

    /// 页面通道：台子在页面跑起来之前挂 handler 用它。
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "台子断言用；生产路径不需要取通道")
    )]
    pub(crate) fn host_channel(&self) -> &HostChannel {
        &self.channel
    }

    /// 页面的 `AppView`（激活中或装载失败时是 None）。
    #[cfg_attr(not(test), allow(dead_code, reason = "台子断言用；生产路径 M6 接线"))]
    pub(crate) fn app_view(&self) -> Option<&Entity<AppView>> {
        self.view.as_ref()
    }

    /// 装载失败的原因。
    #[cfg_attr(not(test), allow(dead_code, reason = "台子断言用；生产路径 M6 接线"))]
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}
