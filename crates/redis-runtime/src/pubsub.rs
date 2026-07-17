use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub enum SubscriptionCommand {
    Subscribe(String),
    PSubscribe(String),
    Unsubscribe(String),
    PUnsubscribe(String),
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PubSubMessageKind {
    Message,
    PMessage,
    SMessage,
}

impl PubSubMessageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::PMessage => "pmessage",
            Self::SMessage => "smessage",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PubSubMessage {
    pub kind: PubSubMessageKind,
    pub channel: String,
    pub pattern: Option<String>,
    pub payload: String,
    pub received_at: chrono::DateTime<chrono::Local>,
}

pub struct RedisPubSubHandle {
    cmd_tx: mpsc::UnboundedSender<SubscriptionCommand>,
    msg_rx: mpsc::UnboundedReceiver<PubSubMessage>,
}

impl RedisPubSubHandle {
    pub fn new(
        cmd_tx: mpsc::UnboundedSender<SubscriptionCommand>,
        msg_rx: mpsc::UnboundedReceiver<PubSubMessage>,
    ) -> Self {
        Self { cmd_tx, msg_rx }
    }

    pub fn send(&self, cmd: SubscriptionCommand) -> bool {
        self.cmd_tx.send(cmd).is_ok()
    }

    pub async fn recv(&mut self) -> Option<PubSubMessage> {
        self.msg_rx.recv().await
    }

    pub fn is_alive(&self) -> bool {
        !self.cmd_tx.is_closed()
    }

    pub fn clone_sender(&self) -> mpsc::UnboundedSender<SubscriptionCommand> {
        self.cmd_tx.clone()
    }
}
