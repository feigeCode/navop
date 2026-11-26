//! 侧栏「+」按钮到扩展市场的跳转入口。
//!
//! terminal_view / db_view 不能反向依赖 navop_app，因此由宿主在启动时注册
//! 打开动作，侧栏只负责调用；未注册时（无 nsh 宿主的构建）按钮不渲染。

use std::sync::Arc;

use gpui::{App, Global, Window};

type OpenComposite = Arc<dyn Fn(&mut Window, &mut App) + Send + Sync + 'static>;

pub struct ExtensionMarketplaceOpener {
    open_composite: OpenComposite,
}

impl Global for ExtensionMarketplaceOpener {}

/// 宿主注册「打开复合扩展市场」的动作（应用启动时调用一次）。
pub fn init(open_composite: impl Fn(&mut Window, &mut App) + Send + Sync + 'static, cx: &mut App) {
    cx.set_global(ExtensionMarketplaceOpener {
        open_composite: Arc::new(open_composite),
    });
}

/// 打开扩展市场并过滤到复合扩展；宿主未注册时返回 false。
pub fn open_composite(window: &mut Window, cx: &mut App) -> bool {
    let Some(opener) = cx.try_global::<ExtensionMarketplaceOpener>() else {
        return false;
    };
    let open = opener.open_composite.clone();
    open(window, cx);
    true
}

/// 宿主是否提供了市场入口：决定侧栏是否渲染「+」按钮。
pub fn is_available(cx: &App) -> bool {
    cx.try_global::<ExtensionMarketplaceOpener>().is_some()
}
