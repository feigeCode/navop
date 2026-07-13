use gpui::{AsyncApp, Context};
use port_forwarding::LocalPortForwardActivity;
use rust_i18n::t;
use tokio::sync::mpsc;

use crate::tab::PortForwardingTab;

const MAX_ACTIVITY_EVENTS: usize = 200;

impl PortForwardingTab {
    pub(crate) fn listen_for_activity(
        &self,
        mut receiver: mpsc::UnboundedReceiver<LocalPortForwardActivity>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            while let Some(activity) = receiver.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.record_activity(activity);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn record_activity(&mut self, activity: LocalPortForwardActivity) {
        let event = match activity {
            LocalPortForwardActivity::Connected {
                source,
                target_host,
                target_port,
            } => t!(
                "PortForwardingTab.event_connection_opened",
                source = source.to_string(),
                target = format!("{target_host}:{target_port}")
            )
            .to_string(),
            LocalPortForwardActivity::Closed { source } => t!(
                "PortForwardingTab.event_connection_closed",
                source = source.to_string()
            )
            .to_string(),
            LocalPortForwardActivity::Failed { source, error } => t!(
                "PortForwardingTab.event_connection_failed",
                source = source.to_string(),
                error = error
            )
            .to_string(),
        };
        push_bounded(&mut self.events, event);
    }
}

fn push_bounded(events: &mut Vec<String>, event: String) {
    if events.len() >= MAX_ACTIVITY_EVENTS {
        events.remove(0);
    }
    events.push(event);
}

#[cfg(test)]
mod tests {
    use super::{MAX_ACTIVITY_EVENTS, push_bounded};

    #[test]
    fn activity_history_keeps_the_latest_events() {
        let mut events = (0..MAX_ACTIVITY_EVENTS)
            .map(|index| index.to_string())
            .collect::<Vec<_>>();

        push_bounded(&mut events, "latest".to_string());

        assert_eq!(events.len(), MAX_ACTIVITY_EVENTS);
        assert_eq!(events.first().map(String::as_str), Some("1"));
        assert_eq!(events.last().map(String::as_str), Some("latest"));
    }
}
