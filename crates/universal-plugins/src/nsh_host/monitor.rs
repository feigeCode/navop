//! provider 重启/移除事件的接线。
//!
//! 页面持有的是"某个 provider 进程的 generation"：进程一旦换掉，旧的
//! provider 客户端、打开的资源、派生的句柄全部作废。监视事件是唯一能告诉
//! 我们这件事的通道，所以每个扩展页面都要登记进来，
//! 由这里统一转发。

use std::rc::Rc;

use extension_plugin_adapter::RuntimeMonitorEvent;
use gpui::App;

use super::{NshHost, Tracked};

impl NshHost {
    /// 把桥接挂到 runtime 监视流上。
    ///
    /// 只在运行时构建里被调用（测试环境没有真实 provider 进程）；函数本身
    /// 保持可编译，免得监视相关类型在测试构建里变成死代码。
    pub(crate) fn start_monitor_bridge(&self, cx: &mut App) {
        // 转发**完整事件**：tab 要依据 generation 与 session 关闭状态区分
        // "瞬时抖动"和"provider 已换进程"，只传 runtime id 会丢语义。
        let (sender, receiver) = smol::channel::bounded::<RuntimeMonitorEvent>(32);
        let mut events = self.service.subscribe();
        self.tokio().spawn(async move {
            while let Ok(event) = events.recv().await {
                if sender.send(event).await.is_err() {
                    break;
                }
            }
        });
        let tracked = Rc::clone(&self.tracked);
        cx.spawn(async move |cx| {
            while let Ok(event) = receiver.recv().await {
                let entries = tracked
                    .borrow()
                    .values()
                    .flatten()
                    .cloned()
                    .collect::<Vec<_>>();
                for entry in entries {
                    match entry {
                        Tracked::Page(tab) => {
                            let _ = tab.update(cx, |tab, cx| tab.runtime_changed(&event, cx));
                        }
                    }
                }
            }
        })
        .detach();
    }
}
