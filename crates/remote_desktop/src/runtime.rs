use crate::{RemoteDesktopInput, output_mailbox::OutputMailboxReceiver};

pub struct RemoteDesktopRuntime {
    pub input_tx: tokio::sync::mpsc::UnboundedSender<RemoteDesktopInput>,
    pub output_rx: OutputMailboxReceiver,
}
