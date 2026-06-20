use crate::{RemoteDesktopInput, RemoteDesktopOutput};

pub struct RemoteDesktopRuntime {
    pub input_tx: tokio::sync::mpsc::UnboundedSender<RemoteDesktopInput>,
    pub output_rx: std::sync::mpsc::Receiver<RemoteDesktopOutput>,
}
