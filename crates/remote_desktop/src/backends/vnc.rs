use crate::backends::vnc_rfb::run_vnc_thread;
use crate::{
    RemoteDesktopBackend, RemoteDesktopConnectionOptions, RemoteDesktopRuntime, RemoteDesktopSize,
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
        let options = self.options;

        std::thread::Builder::new()
            .name("remote-desktop-vnc".to_string())
            .spawn(move || run_vnc_thread(options, &mut input_rx, output_tx))?;

        Ok(RemoteDesktopRuntime {
            input_tx,
            output_rx,
        })
    }
}
