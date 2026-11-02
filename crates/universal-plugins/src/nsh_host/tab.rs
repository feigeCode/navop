//! 承载一个 nsh 扩展页面的 tab。

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Window, div,
};
use gpui_component::Theme;
use nsh_platform_gpui::{AppOptions, AppView};

use super::{Trust, bundle, theme};

/// 一个扩展页面（一个 `AppView` + 它的 manifest 元数据）。
pub struct NshTab {
    view: Option<Entity<AppView>>,
    /// 装载失败时的原因：tab 照样开出来，把错误写在页面上，而不是静默失败。
    error: Option<String>,
    /// 页面的扩展 / view 身份：M4 的 host 会话按它建，关闭与排障也要它。
    extension_id: SharedString,
    view_id: SharedString,
    title: SharedString,
    /// 主题/明暗变化时把新 token 推给页面。
    _theme: Subscription,
}

impl NshTab {
    pub(crate) fn open(
        contribution: extension_runtime::RegisteredShellViewContribution,
        trust: Trust,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let title: SharedString = contribution.title.clone().into();
        let extension_id: SharedString = contribution.extension_id.clone().into();
        let view_id: SharedString = contribution.id.clone().into();
        let program = bundle::load(&contribution, &trust);

        let (view, error, accepted) = match program {
            Ok(program) => {
                let accepted = program.accepted;
                let options = AppOptions {
                    app_id: program.app_id,
                    asset_base: program.asset_base,
                    ..AppOptions::default()
                };
                let view = cx.new(|cx| AppView::new(options, window, cx));
                theme::push(&view, cx);
                let loaded = accepted;
                let failed = Arc::new(AtomicBool::new(false));
                let failed_for_error = failed.clone();
                view.update(cx, |view, _cx| {
                    view.on_error(move |message| {
                        failed_for_error.store(true, Ordering::Relaxed);
                        tracing::error!(
                            extension_id = %contribution.extension_id,
                            view_id = %contribution.id,
                            "nsh 页面错误: {message}"
                        );
                    });
                    view.load(program.app);
                });
                (Some(view), None, loaded.map(|loaded| (loaded, failed)))
            }
            Err(error) => {
                tracing::error!(
                    extension_id = %contribution.extension_id,
                    view_id = %contribution.id,
                    path = %contribution.entry_path.display(),
                    "nsh 页面装载失败: {error:#}"
                );
                (None, Some(format!("{error:#}")), None)
            }
        };

        let tab: Entity<Self> = cx.new(|cx| {
            let _theme = cx.observe_global::<Theme>(|this: &mut Self, cx| {
                if let Some(view) = &this.view {
                    theme::push(view, cx);
                }
            });
            Self {
                view,
                error,
                extension_id,
                view_id,
                title,
                _theme,
            }
        });

        // 两阶段装载的第二步：首帧画出来、且没有立刻报错，才算这个 build
        // 健康，把它记进信任库（方案 §8）。失败就让上一个 build 继续可用。
        if let Some((loaded, failed)) = accepted {
            cx.spawn(async move |cx| {
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
            })
            .detach();
        }
        tab
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
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div().size_full().min_w_0().min_h_0();
        match (&self.view, self.error()) {
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

    /// 页面的 `AppView`（装载失败时是 None）。M4 的 host 会话挂在这上面。
    #[cfg_attr(not(test), expect(dead_code, reason = "M4 接 host ABI 时由生产代码用"))]
    pub(crate) fn app_view(&self) -> Option<&Entity<AppView>> {
        self.view.as_ref()
    }

    /// 装载失败的原因。
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}
