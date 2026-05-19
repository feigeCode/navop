//! Redis 工具页签命令动作与自动刷新。

use crate::redis_tool_data::{build_publish_command, slowlog_reset_command};
use crate::redis_tool_view::{AUTO_REFRESH_SECONDS, ActionState, RedisToolView};
use crate::{GlobalRedisState, RedisValue};
use gpui::Context;
use one_core::gpui_tokio::Tokio;
use std::time::Duration;

impl RedisToolView {
    pub(crate) fn publish_message(&mut self, cx: &mut Context<Self>) {
        let channel = self.publish_channel_input.read(cx).text().to_string();
        let message = self.publish_message_input.read(cx).text().to_string();
        if channel.trim().is_empty() || message.is_empty() {
            self.action_state =
                ActionState::Error("请输入 channel 和 message 后再发布".to_string());
            cx.notify();
            return;
        }
        self.run_command(
            build_publish_command(channel.trim(), &message),
            "Publishing message...",
            |value| {
                format!(
                    "Published, delivered to {} subscriber(s)",
                    value.to_display_string()
                )
            },
            cx,
        );
    }

    pub(crate) fn reset_slowlog(&mut self, cx: &mut Context<Self>) {
        self.run_command(
            slowlog_reset_command().to_string(),
            "Clearing slow log...",
            |_| "Slow log cleared".to_string(),
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
        pending: &'static str,
        success: fn(RedisValue) -> String,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = self.connection_id.clone() else {
            self.action_state = ActionState::Error("请选择并连接一个 Redis 连接".to_string());
            cx.notify();
            return;
        };
        let global_state = cx.global::<GlobalRedisState>().clone();
        self.action_state = ActionState::Pending(pending.to_string());
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = Tokio::spawn_result(cx, async move {
                let conn = global_state
                    .get_connection(&connection_id)
                    .ok_or_else(|| anyhow::anyhow!("Redis 连接尚未建立"))?;
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
}
