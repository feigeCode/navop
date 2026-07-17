//! Redis 发布订阅 (Pub/Sub) 监听后台任务。
//!
//! 由于 Redis 在订阅后会把连接置于 pub/sub 模式 (不再接收普通命令),
//! 必须为订阅使用**独立**的 connection,不能复用主连接池里的
//! `ConnectionManager`。本模块通过 [`start_pubsub_listener`] 启动一个独立
//! 的 Tokio 任务,通过 mpsc 通道与视图层双向通信:
//!
//! - `cmd_tx` (视图层 → 后台): 投递 [`SubscriptionCommand`]
//! - `msg_rx` (后台 → 视图层): 推送 [`PubSubMessage`]
//!
//! 任务在以下任一情况退出: 收到 `SubscriptionCommand::Stop`、`cmd_tx` 被
//! 全部 drop (视图被销毁)、连接出错。任何一种退出方式都会同时 drop 内部
//! 的 `msg_tx`,视图侧的 `msg_rx.recv()` 随之返回 `None`,从而通知 UI。

use crate::types::{RedisConnectionConfig, RedisConnectionMode};
pub use crate::{PubSubMessage, PubSubMessageKind, RedisPubSubHandle, SubscriptionCommand};
use anyhow::{Context as _, Result, anyhow};
use futures::StreamExt;
use rust_i18n::t;
use tokio::sync::mpsc;
use tokio::time::Duration;

const CONNECT_TIMEOUT_FALLBACK_SECS: u64 = 10;

/// 启动一个独立的 PubSub 监听任务。
///
/// 必须在 Tokio runtime 上调用 (通过 `one_core::gpui_tokio::Tokio::handle`
/// 拿到 runtime,然后 `runtime.spawn` 包装一层后调本函数)。返回的 handle
/// 一旦 drop,后台任务会在下一轮 select 中检测到 cmd_tx 关闭而退出。
pub async fn start_pubsub_listener(config: RedisConnectionConfig) -> Result<RedisPubSubHandle> {
    if matches!(config.mode, RedisConnectionMode::Cluster) {
        return Err(anyhow!(
            t!("RedisPubSub.subscription_unsupported_cluster").to_string()
        ));
    }

    let timeout = if config.timeout == 0 {
        Duration::from_secs(CONNECT_TIMEOUT_FALLBACK_SECS)
    } else {
        Duration::from_secs(config.timeout)
    };

    let url = config.to_url();
    let client = redis_client::Client::open(url.as_str())
        .with_context(|| t!("RedisConnection.create_client_failed").to_string())?;

    // 建立独立的 pubsub 异步连接;受 timeout 限制以避免无限等待
    let pubsub = tokio::time::timeout(timeout, client.get_async_pubsub())
        .await
        .map_err(|_| anyhow!(t!("RedisConnection.connect_failed").to_string()))?
        .with_context(|| t!("RedisConnection.connect_failed").to_string())?;

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<SubscriptionCommand>();
    let (msg_tx, msg_rx) = mpsc::unbounded_channel::<PubSubMessage>();

    tokio::spawn(run_listener(pubsub, cmd_rx, msg_tx));

    Ok(RedisPubSubHandle::new(cmd_tx, msg_rx))
}

/// 监听任务主循环:
/// 同时等待用户指令 (cmd_rx) 与服务端推送 (pubsub stream),用 `tokio::select!`
/// 串起来。所有路径都通过 `break` 退出主循环,函数返回后 `msg_tx` drop,
/// 视图侧的 `recv()` 自然收到 `None`。
async fn run_listener(
    pubsub: redis_client::aio::PubSub,
    mut cmd_rx: mpsc::UnboundedReceiver<SubscriptionCommand>,
    msg_tx: mpsc::UnboundedSender<PubSubMessage>,
) {
    // 把 PubSub 拆成 sink (用于发指令) + stream (用于收消息),避免在
    // 同一对象上既要可变借用 stream 又要可变借用调用 subscribe。
    let (mut sink, mut stream) = pubsub.split();

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else {
                    // 所有 cmd_tx 都被 drop 了 → 视图被销毁 → 退出
                    break;
                };
                match cmd {
                    SubscriptionCommand::Stop => break,
                    SubscriptionCommand::Subscribe(channel) => {
                        if let Err(err) = sink.subscribe(&channel).await {
                            tracing::warn!(channel = %channel, "redis subscribe failed: {err}");
                            // 命令失败不退出循环,允许用户后续重试其它频道
                        }
                    }
                    SubscriptionCommand::Unsubscribe(channel) => {
                        if let Err(err) = sink.unsubscribe(&channel).await {
                            tracing::warn!(channel = %channel, "redis unsubscribe failed: {err}");
                        }
                    }
                    SubscriptionCommand::PSubscribe(pattern) => {
                        if let Err(err) = sink.psubscribe(&pattern).await {
                            tracing::warn!(pattern = %pattern, "redis psubscribe failed: {err}");
                        }
                    }
                    SubscriptionCommand::PUnsubscribe(pattern) => {
                        if let Err(err) = sink.punsubscribe(&pattern).await {
                            tracing::warn!(pattern = %pattern, "redis punsubscribe failed: {err}");
                        }
                    }
                }
            }
            maybe_msg = stream.next() => {
                let Some(msg) = maybe_msg else {
                    // 服务端连接断开 → 退出
                    break;
                };
                let converted = convert_msg(&msg);
                if msg_tx.send(converted).is_err() {
                    // 视图已经 drop msg_rx,没人接消息了,退出。
                    break;
                }
            }
        }
    }
}

fn convert_msg(msg: &redis_client::Msg) -> PubSubMessage {
    let channel = msg.get_channel_name().to_string();
    let payload = String::from_utf8_lossy(msg.get_payload_bytes()).into_owned();
    let pattern: Option<String> = if msg.from_pattern() {
        msg.get_pattern::<String>().ok()
    } else {
        None
    };
    let kind = if pattern.is_some() {
        PubSubMessageKind::PMessage
    } else {
        PubSubMessageKind::Message
    };
    PubSubMessage {
        kind,
        channel,
        pattern,
        payload,
        received_at: chrono::Local::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_kind_strings_are_stable() {
        assert_eq!("message", PubSubMessageKind::Message.as_str());
        assert_eq!("pmessage", PubSubMessageKind::PMessage.as_str());
        assert_eq!("smessage", PubSubMessageKind::SMessage.as_str());
    }

    #[tokio::test]
    async fn cluster_mode_is_rejected() {
        let mut config = RedisConnectionConfig::default();
        config.mode = RedisConnectionMode::Cluster;
        let result = start_pubsub_listener(config).await;
        assert!(result.is_err(), "cluster mode should not be supported");
    }
}
