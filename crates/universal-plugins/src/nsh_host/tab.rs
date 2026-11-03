//! 承载一个 nsh 扩展页面的 tab。
//!
//! 装载分两段，和 gpui-shell 那侧同构：先**租**provider（激活后端 runtime，
//! 拿到 activation lease），租到手再挂页面。顺序不能倒 —— 页面 `mount` 的
//! 第一行就可能 `host.call` 要数据，provider 没起来就只能拿到错误。
//!
//! 所以 tab 先以"激活中"的样子开出来，激活结果回到主线程后，下一次
//! `render` 才真正建 `AppView`。租约跟着 tab 走：tab 关掉（Drop）就还回去，
//! 中间任何一步失败也把已经租到的那些还掉。

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use extension_host::CancellationToken;
use extension_plugin_adapter::ActivationHandle;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Task, Window, div,
};
use gpui_component::Theme;
use nsh_engine::HostChannel;
use nsh_platform_gpui::{AppOptions, AppView};

use super::{Trust, abi, bundle, session::NshSession, theme};

/// 一个扩展页面（一个 `AppView` + 它的 manifest 元数据）。
pub struct NshTab {
    /// 激活 lease 用：还租约要在 tokio 上 await。
    service: crate::UniversalPluginService,
    tokio: tokio::runtime::Handle,
    /// 签名包验签要信任库；它是主线程独占的（`Rc<RefCell<_>>`），所以
    /// 装载动作只在本结构所在线程上做。
    trust: Trust,
    contribution: extension_runtime::RegisteredShellViewContribution,
    /// 页面的扩展 / view 身份：host 会话按它建，关闭与排障也要它。
    extension_id: SharedString,
    view_id: SharedString,
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
}

impl NshTab {
    pub(crate) fn open(
        contribution: extension_runtime::RegisteredShellViewContribution,
        service: crate::UniversalPluginService,
        trust: Trust,
        dispatcher: super::dispatch::MainThread,
        cx: &mut App,
    ) -> Entity<Self> {
        Self::create(contribution, service, trust, dispatcher, None, None, cx)
    }

    /// 连接型页面：先用连接配置打开 provider 资源，再把页面挂在这个会话上。
    pub(crate) fn open_connection(
        contribution: extension_runtime::RegisteredShellViewContribution,
        service: crate::UniversalPluginService,
        trust: Trust,
        dispatcher: super::dispatch::MainThread,
        connection: super::connection::NshConnectionLaunch,
        title: SharedString,
        cx: &mut App,
    ) -> Entity<Self> {
        Self::create(
            contribution,
            service,
            trust,
            dispatcher,
            Some(connection),
            Some(title),
            cx,
        )
    }

    fn create(
        contribution: extension_runtime::RegisteredShellViewContribution,
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
        let view_id: SharedString = contribution.id.clone().into();
        let tokio = one_core::gpui_tokio::Tokio::handle(cx);
        let channel = HostChannel::default();

        let channel_for_theme = channel.clone();
        let tab: Entity<Self> = cx.new(|cx| {
            let _theme = cx.observe_global::<Theme>(move |this: &mut Self, cx| {
                if let Some(view) = &this.view {
                    theme::push(view, cx);
                    // 缓存里那份供 `navop.theme.snapshot` 读，同时推
                    // `navop.theme.changed` 给页面。
                    if let Some(abi) = &this.abi
                        && let Some(payload) = abi.set_theme(theme::snapshot(cx))
                        && let Err(error) = channel_for_theme.send("navop.theme.changed", payload)
                    {
                        tracing::warn!(%error, "nsh 页面主题推送失败");
                    }
                }
            });
            Self {
                service,
                tokio,
                trust,
                contribution,
                extension_id,
                view_id,
                title,
                channel,
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
                    view_id = %self.view_id,
                    path = %self.contribution.entry_path.display(),
                    "nsh 页面装载失败: {error:#}"
                );
                self.error = Some(format!("{error:#}"));
                return;
            }
        };
        // 页面能力要在页面起来之前就有：handler 必须挂在它的通道上，
        // 而页面启动的第一行就可能 `host.call`。
        let host_abi = abi::HostAbi::new(&self.contribution);
        let session = Arc::new(NshSession::new(
            self.service.clone(),
            self.contribution.backends.clone(),
            self.tokio.clone(),
        ));
        let accepted = program.accepted;
        let options = AppOptions {
            app_id: program.app_id,
            asset_base: program.asset_base,
            host: self.channel.clone(),
            // 原生组件（`<Terminal/>` 这类）按**本页面**的注册表注入：
            // 每视图一份，工厂里钉死扩展身份与清单权限。
            components: super::native::components_for(&self.contribution, cx),
            ..AppOptions::default()
        };
        // 连接型页面：把刚打开的资源交给会话（句柄归本 tab 所有），
        // 并把连接事实放进 `navop.context.current()` —— 页面据此知道自己
        // 在服务哪条连接。
        if let Some(prepared) = self.prepared_connection.take()
            && let Ok(context) = prepared.adopt(&session)
        {
            host_abi.set_connection(context);
        }
        // 先把能力挂到通道上，再让页面跑起来。
        abi::register(
            &self.channel,
            Arc::clone(&host_abi),
            Arc::clone(&session),
            &self.dispatcher,
        );
        let view = cx.new(|cx| AppView::new(options, window, cx));
        theme::push(&view, cx);
        // 首帧前只填缓存，不推事件：页面启动时读 `navop.theme.snapshot`
        // 就是当前主题，多一条 'changed' 只会让它多做一次渲染。
        host_abi.set_theme(theme::snapshot(cx));

        let failed = Arc::new(AtomicBool::new(false));
        let failed_for_error = Arc::clone(&failed);
        let extension_id = self.extension_id.clone();
        let view_id = self.view_id.clone();
        view.update(cx, |view, _cx| {
            view.on_error(move |message| {
                failed_for_error.store(true, Ordering::Relaxed);
                tracing::error!(
                    extension_id = %extension_id,
                    view_id = %view_id,
                    "nsh 页面错误: {message}"
                );
            });
            view.load(program.app);
        });

        self.abi = Some(host_abi);
        self.session = Some(session);
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

impl Drop for NshTab {
    fn drop(&mut self) {
        // tab 关掉：先掐掉还在路上的激活与在途调用，再还不租约。
        // 这里没有 `cx`，所以直接用 tokio 句柄异步还；Drop 不能 await。
        self.cancel.cancel();
        if let Some(abi) = &self.abi {
            abi.cancel();
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
                            view_id = %self.view_id,
                            "nsh 页面 provider 激活失败: {error}"
                        );
                        self.error = Some(error);
                    }
                }
            }
        }
        let mut root = div().size_full().min_w_0().min_h_0();
        match (&self.view, self.error.as_deref()) {
            (Some(view), _) => root = root.child(view.clone()),
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

    pub(crate) fn view_id(&self) -> &SharedString {
        &self.view_id
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
            view_id = %self.view_id,
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
