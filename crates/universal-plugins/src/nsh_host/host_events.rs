//! 宿主自发事件（不经 provider 的事件流）。
//!
//! 页面用同一套订阅接口拿到它们：
//!
//! ```text
//! host.call("navop.event.subscribe", { handle: "host", kind: "db.treeChanged" })
//!   -> { handle: "host:db.treeChanged:1", cursor: 0 }
//! host.on("navop.event", (frame) => …)   // 帧形状与 provider 事件完全一致
//! ```
//!
//! 约定：`navop.event.subscribe` 的 `handle` 写 `"host"` 表示订阅**宿主事件**，
//! `kind` 就是 topic 名。宿主侧（db_view / terminal_view 等）通过
//! [`publish`] 推事件，没有订阅者时直接丢弃——不会为没人听的事件留缓冲区。
//!
//! 与 provider 事件的区别只有来源：这里的事件是宿主进程里的事实变化
//! （树重载、执行完 DDL），不需要 provider 资源句柄。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde_json::Value;
use tokio::sync::broadcast;

/// 每个 topic 的缓冲深度：慢订阅者溢出时拿到 `EVENT_OVERFLOW`，
/// 与 provider 事件的溢出语义一致（不是静默丢）。
const TOPIC_CAPACITY: usize = 64;

/// 订阅宿主事件时 `handle` 字段的固定取值。
pub(crate) const HOST_HANDLE: &str = "host";

/// 宿主事件 topic 前缀（订阅句柄用）。
const HANDLE_PREFIX: &str = "host:";

struct Bus {
    topics: Mutex<HashMap<String, broadcast::Sender<Value>>>,
}

fn bus() -> &'static Bus {
    static BUS: OnceLock<Bus> = OnceLock::new();
    BUS.get_or_init(|| Bus {
        topics: Mutex::new(HashMap::new()),
    })
}

/// 往一个宿主事件 topic 推一条事件；没人订阅时直接丢弃。
///
/// 可以从任意线程调用（db_view / terminal_view 的事件回调里都能用）。
#[cfg_attr(not(feature = "nsh-db"), allow(dead_code))]
pub fn publish(topic: &str, data: Value) {
    let sender = bus()
        .topics
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(topic)
        .cloned();
    if let Some(sender) = sender {
        // 订阅者晚到就收不到历史事件：宿主事件是“此刻发生”，不是回放流。
        let _ = sender.send(data);
    }
}

/// 订阅句柄是不是宿主事件（`unsubscribe` 据此跳过 provider 的 close）。
pub(crate) fn is_host_handle(handle: &str) -> bool {
    handle.starts_with(HANDLE_PREFIX)
}

/// 一条宿主事件订阅：句柄 + 接收端。
pub(crate) struct HostSubscription {
    pub handle: String,
    pub receiver: broadcast::Receiver<Value>,
}

/// 建一条宿主事件订阅（topic 按需建 broadcaster）。
pub(crate) fn subscribe(topic: &str) -> HostSubscription {
    static NEXT: AtomicU64 = AtomicU64::new(1);

    let sender = {
        let mut topics = bus().topics.lock().unwrap_or_else(|e| e.into_inner());
        topics
            .entry(topic.to_string())
            .or_insert_with(|| broadcast::channel(TOPIC_CAPACITY).0)
            .clone()
    };
    let receiver = sender.subscribe();
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    HostSubscription {
        handle: format!("{HANDLE_PREFIX}{topic}:{seq}"),
        receiver,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn published_events_reach_existing_subscribers() {
        let mut subscription = subscribe("test.topic");
        assert!(subscription.handle.starts_with("host:test.topic:"));
        publish("test.topic", json!({ "ok": true }));
        let received = subscription.receiver.try_recv().expect("事件应当到达");
        assert_eq!(received["ok"], json!(true));
    }

    #[test]
    fn publish_without_subscribers_is_a_no_op() {
        // 没有订阅者时只丢弃，不该 panic、也不该留下 pending 缓冲。
        publish("test.nobody", json!({ "x": 1 }));
    }

    #[test]
    fn host_handles_are_recognisable() {
        assert!(is_host_handle("host:db.treeChanged:1"));
        assert!(!is_host_handle("dbs-1"));
    }
}
