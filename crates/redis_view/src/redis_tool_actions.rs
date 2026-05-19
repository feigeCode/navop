//! Redis 工具页签命令动作与自动刷新。

use crate::redis_pubsub::{PubSubMessage, SubscriptionCommand};
use crate::redis_tool_data::{build_publish_command, slowlog_reset_command};
use crate::redis_tool_view::{
    AUTO_REFRESH_SECONDS, ActionState, MAX_RECEIVED_MESSAGES, PubSubBodyTab, RedisToolView,
};
use crate::{GlobalRedisState, RedisValue};
use gpui::{Context, Window};
use one_core::gpui_tokio::Tokio;
use rust_i18n::t;
use std::time::Duration;

impl RedisToolView {
    pub(crate) fn publish_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let channel = self.publish_channel_input.read(cx).text().to_string();
        let message = self.publish_message_input.read(cx).text().to_string();
        if channel.trim().is_empty() {
            self.action_state = ActionState::Error(t!("RedisPubSub.publish_invalid").to_string());
            cx.notify();
            return;
        }
        // Redis PUBLISH 允许空 payload,因此 message 不做非空校验。
        let trimmed_channel = channel.trim().to_string();
        let message_value = message.clone();
        // 立刻清空消息输入框,避免用户重复点击发同一条
        self.publish_message_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.run_command(
            build_publish_command(&trimmed_channel, &message_value),
            t!("RedisPubSub.publish_pending").to_string(),
            move |value| {
                t!(
                    "RedisPubSub.publish_success",
                    count = value.to_display_string()
                )
                .to_string()
            },
            cx,
        );
    }

    pub(crate) fn reset_slowlog(&mut self, cx: &mut Context<Self>) {
        self.run_command(
            slowlog_reset_command().to_string(),
            t!("RedisTool.slowlog_clearing").to_string(),
            |_| t!("RedisTool.slowlog_cleared").to_string(),
            cx,
        );
    }

    pub(crate) fn set_auto_refresh(&mut self, checked: bool, cx: &mut Context<Self>) {
        self.auto_refresh = checked;
        self.refresh_generation = self.refresh_generation.wrapping_add(1);
        self.auto_refresh_scheduled = false;
        if checked {
            self.refresh(cx);
            self.schedule_auto_refresh(cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn schedule_auto_refresh(&mut self, cx: &mut Context<Self>) {
        if self.auto_refresh_scheduled {
            return;
        }
        self.auto_refresh_scheduled = true;
        let generation = self.refresh_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_secs(AUTO_REFRESH_SECONDS))
                .await;
            _ = this.update(cx, |view, cx| {
                view.auto_refresh_scheduled = false;
                if view.auto_refresh && view.refresh_generation == generation {
                    view.refresh(cx);
                    view.schedule_auto_refresh(cx);
                }
            });
        })
        .detach();
    }

    fn run_command(
        &mut self,
        command: String,
        pending: String,
        success: impl FnOnce(RedisValue) -> String + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = self.connection_id.clone() else {
            self.action_state = ActionState::Error(t!("RedisTool.connection_required").to_string());
            cx.notify();
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        self.action_state = ActionState::Pending(pending);
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = Tokio::spawn_result(cx, async move {
                let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                    anyhow::anyhow!(t!("RedisTool.connection_not_ready").to_string())
                })?;
                let guard = conn.read().await;
                let value = guard
                    .execute_command(&command)
                    .await
                    .map_err(anyhow::Error::from)?;
                Ok(success(value))
            })
            .await;

            _ = this.update(cx, |view, cx| {
                view.action_state = match result {
                    Ok(message) => ActionState::Success(message),
                    Err(error) => ActionState::Error(format!("{error:#}")),
                };
                view.refresh(cx);
            });
        })
        .detach();
    }

    // ===== 发布订阅 =====

    /// 订阅一个普通频道。
    pub(crate) fn subscribe_channel(
        &mut self,
        channel: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let channel = channel.trim().to_string();
        if channel.is_empty() {
            return;
        }
        if self.subscribed_channels.iter().any(|c| c == &channel) {
            self.subscribe_error = Some(
                t!(
                    "RedisPubSub.subscription_already_active",
                    name = channel.as_str()
                )
                .to_string(),
            );
            cx.notify();
            return;
        }
        if !self.ensure_subscription_started(cx) {
            return;
        }
        if let Some(tx) = &self.subscribe_cmd_tx
            && tx
                .send(SubscriptionCommand::Subscribe(channel.clone()))
                .is_ok()
        {
            self.subscribed_channels.push(channel);
            self.subscribe_input.update(cx, |state, cx| {
                state.set_value("", window, cx);
            });
            self.body_tab = PubSubBodyTab::Messages;
            self.subscribe_error = None;
            cx.notify();
        }
    }

    /// 订阅一个 pattern。
    pub(crate) fn subscribe_pattern(
        &mut self,
        pattern: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pattern = pattern.trim().to_string();
        if pattern.is_empty() {
            return;
        }
        if self.subscribed_patterns.iter().any(|p| p == &pattern) {
            self.subscribe_error = Some(
                t!(
                    "RedisPubSub.subscription_already_active",
                    name = pattern.as_str()
                )
                .to_string(),
            );
            cx.notify();
            return;
        }
        if !self.ensure_subscription_started(cx) {
            return;
        }
        if let Some(tx) = &self.subscribe_cmd_tx
            && tx
                .send(SubscriptionCommand::PSubscribe(pattern.clone()))
                .is_ok()
        {
            self.subscribed_patterns.push(pattern);
            self.subscribe_input.update(cx, |state, cx| {
                state.set_value("", window, cx);
            });
            self.body_tab = PubSubBodyTab::Messages;
            self.subscribe_error = None;
            cx.notify();
        }
    }

    /// 取消订阅一个普通频道。
    pub(crate) fn unsubscribe_channel(&mut self, channel: &str, cx: &mut Context<Self>) {
        if let Some(tx) = &self.subscribe_cmd_tx {
            let _ = tx.send(SubscriptionCommand::Unsubscribe(channel.to_string()));
        }
        self.subscribed_channels.retain(|c| c != channel);
        cx.notify();
    }

    /// 取消订阅一个 pattern。
    pub(crate) fn unsubscribe_pattern(&mut self, pattern: &str, cx: &mut Context<Self>) {
        if let Some(tx) = &self.subscribe_cmd_tx {
            let _ = tx.send(SubscriptionCommand::PUnsubscribe(pattern.to_string()));
        }
        self.subscribed_patterns.retain(|p| p != pattern);
        cx.notify();
    }

    /// 取消所有订阅,关闭后台监听 task。
    pub(crate) fn unsubscribe_all(&mut self, cx: &mut Context<Self>) {
        if let Some(tx) = self.subscribe_cmd_tx.take() {
            let _ = tx.send(SubscriptionCommand::Stop);
        }
        self.subscribe_generation = self.subscribe_generation.wrapping_add(1);
        self.subscribed_channels.clear();
        self.subscribed_patterns.clear();
        cx.notify();
    }

    /// 清空已接收的消息缓冲。
    pub(crate) fn clear_received_messages(&mut self, cx: &mut Context<Self>) {
        self.received_messages.clear();
        cx.notify();
    }

    /// 把新消息追加到环形缓冲,超过上限时丢弃最旧的。
    pub(crate) fn push_received_message(&mut self, msg: PubSubMessage, cx: &mut Context<Self>) {
        self.received_messages.push_back(msg);
        while self.received_messages.len() > MAX_RECEIVED_MESSAGES {
            self.received_messages.pop_front();
        }
        cx.notify();
    }

    /// 后台监听任务退出时被调用。
    pub(crate) fn handle_subscription_closed(&mut self, cx: &mut Context<Self>) {
        self.subscribe_cmd_tx = None;
        self.subscribed_channels.clear();
        self.subscribed_patterns.clear();
        // 不清空消息,用户仍可查看历史
        self.subscribe_error = Some(t!("RedisPubSub.subscription_closed").to_string());
        cx.notify();
    }

    /// 懒启动订阅任务。返回是否成功(false 表示尚未连接或启动失败)。
    /// 注意:真正的启动是异步的,本函数返回 true 时只代表已经派发了启动任务;
    /// 调用方可在异步完成后通过 `subscribe_cmd_tx` 的存在判断是否就绪。
    fn ensure_subscription_started(&mut self, cx: &mut Context<Self>) -> bool {
        if self.subscribe_cmd_tx.is_some() {
            return true;
        }
        let Some(connection_id) = self.connection_id.clone() else {
            self.subscribe_error = Some(t!("RedisTool.connection_required").to_string());
            cx.notify();
            return false;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        let generation = self.subscribe_generation.wrapping_add(1);
        self.subscribe_generation = generation;
        self.subscribe_error = None;

        // 启动 + 消息消费循环统一在一个 spawn 里完成
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let started = Tokio::spawn_result(cx, async move {
                let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                    anyhow::anyhow!(t!("RedisTool.connection_not_ready").to_string())
                })?;
                let guard = conn.read().await;
                let handle = guard.open_pubsub().await.map_err(anyhow::Error::from)?;
                Ok::<_, anyhow::Error>(handle)
            })
            .await;

            let mut handle = match started {
                Ok(handle) => handle,
                Err(error) => {
                    let _ = this.update(cx, |view, cx| {
                        if view.subscribe_generation == generation {
                            view.subscribe_cmd_tx = None;
                            view.subscribe_error = Some(
                                t!(
                                    "RedisPubSub.subscription_failed",
                                    error = format!("{error:#}").as_str()
                                )
                                .to_string(),
                            );
                            cx.notify();
                        }
                    });
                    return;
                }
            };

            // 把 cmd_tx 暴露给视图:克隆一份发送端
            let sender = handle.clone_sender();
            let _set_ok = this.update(cx, |view, cx| {
                if view.subscribe_generation != generation {
                    return false;
                }
                view.subscribe_cmd_tx = Some(sender);
                cx.notify();
                true
            });

            // 消费循环
            while let Some(msg) = handle.recv().await {
                let still_alive = this
                    .update(cx, |view, cx| {
                        if view.subscribe_generation != generation {
                            return false;
                        }
                        view.push_received_message(msg, cx);
                        true
                    })
                    .ok()
                    .unwrap_or(false);
                if !still_alive {
                    break;
                }
            }

            // task 结束,通知视图
            let _ = this.update(cx, |view, cx| {
                if view.subscribe_generation == generation {
                    view.handle_subscription_closed(cx);
                }
            });
        })
        .detach();
        true
    }
}
