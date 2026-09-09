//! 中间件页签注册:按能力位生成可放入 TabContainer 的 TabItem 集合。
//!
//! 页签 id 形如 "{conn_id}-overview" / "{conn_id}-topics" / "{conn_id}-groups" /
//! "{conn_id}-messages";能力位为 false 的页面不生成。

use gpui::{App, AppContext, Window};
use one_core::tab_container::TabItem;

use crate::{
    MiddlewareAdminHandle, MiddlewareGroupsPage, MiddlewareMessagesPage, MiddlewareOverviewPage,
    MiddlewareTopicsPage,
};

/// 中间件标准管理页签组工厂
pub struct MiddlewarePages;

impl MiddlewarePages {
    /// 按后端能力位生成页签集合(能力位不足的页面自动跳过)
    pub fn new(
        handle: MiddlewareAdminHandle,
        conn_id: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<TabItem> {
        let caps = handle.capabilities();
        let mut items = Vec::new();

        // 概览页:指标或集群任一可用即展示(不可用区域页内占位降级)
        if caps.metrics || caps.cluster_overview {
            items.push(TabItem::new(
                format!("{conn_id}-overview"),
                "middleware",
                cx.new(|cx| MiddlewareOverviewPage::new(handle.clone(), window, cx)),
            ));
        }
        if caps.topics {
            items.push(TabItem::new(
                format!("{conn_id}-topics"),
                "middleware",
                cx.new(|cx| MiddlewareTopicsPage::new(handle.clone(), window, cx)),
            ));
        }
        if caps.groups {
            items.push(TabItem::new(
                format!("{conn_id}-groups"),
                "middleware",
                cx.new(|cx| MiddlewareGroupsPage::new(handle.clone(), window, cx)),
            ));
        }
        if caps.message_query {
            items.push(TabItem::new(
                format!("{conn_id}-messages"),
                "middleware",
                cx.new(|cx| MiddlewareMessagesPage::new(handle.clone(), window, cx)),
            ));
        }
        items
    }
}
