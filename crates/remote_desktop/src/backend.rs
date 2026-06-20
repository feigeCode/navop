use crate::backends::{rdp::RdpBackend, vnc::VncBackend};
use crate::{
    RemoteDesktopConnectionOptions, RemoteDesktopProtocol, RemoteDesktopRuntime, RemoteDesktopSize,
};

pub trait RemoteDesktopBackend: Send + 'static {
    fn start(
        self: Box<Self>,
        initial_size: RemoteDesktopSize,
    ) -> anyhow::Result<RemoteDesktopRuntime>;
}

pub fn create_backend(options: RemoteDesktopConnectionOptions) -> Box<dyn RemoteDesktopBackend> {
    match options.protocol {
        RemoteDesktopProtocol::Rdp => Box::new(RdpBackend::new(options)),
        RemoteDesktopProtocol::Vnc => Box::new(VncBackend::new(options)),
    }
}
