use crate::{
    RemoteDesktopBackend, RemoteDesktopConnectionOptions, RemoteDesktopInput, RemoteDesktopOutput,
    RemoteDesktopRuntime, RemoteDesktopSize,
};

pub struct VncBackend {
    options: RemoteDesktopConnectionOptions,
}

impl VncBackend {
    pub fn new(options: RemoteDesktopConnectionOptions) -> Self {
        Self { options }
    }
}

impl RemoteDesktopBackend for VncBackend {
    fn start(
        self: Box<Self>,
        _initial_size: RemoteDesktopSize,
    ) -> anyhow::Result<RemoteDesktopRuntime> {
        let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
        let (output_tx, output_rx) = std::sync::mpsc::channel();
        let destination = self.options.destination.clone();

        std::thread::Builder::new()
            .name("remote-desktop-vnc".to_string())
            .spawn(move || {
                let _ = output_tx.send(RemoteDesktopOutput::Status(format!(
                    "connecting to VNC {destination}"
                )));
                while let Some(input) = input_rx.blocking_recv() {
                    if matches!(input, RemoteDesktopInput::Close) {
                        break;
                    }
                }
                let _ = output_tx.send(RemoteDesktopOutput::Terminated(
                    "VNC session closed".to_string(),
                ));
            })?;

        Ok(RemoteDesktopRuntime {
            input_tx,
            output_rx,
        })
    }
}
