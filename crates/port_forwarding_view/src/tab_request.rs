use one_core::storage::{PortForwardingKind, StoredConnection};
use port_forwarding::{
    DynamicForwardingRequest, LocalForwardingRequest, LocalPortForwardActivity,
    RemoteForwardingRequest, build_dynamic_forwarding_request, build_local_forwarding_request,
    build_remote_forwarding_request,
};
use tokio::sync::mpsc;

pub(crate) enum StartRequest {
    Local(LocalForwardingRequest),
    Remote(RemoteForwardingRequest),
    Dynamic(DynamicForwardingRequest),
}

pub(crate) fn build_request(
    connection: &StoredConnection,
    ssh: &StoredConnection,
    kind: PortForwardingKind,
    activity_tx: mpsc::UnboundedSender<LocalPortForwardActivity>,
) -> anyhow::Result<StartRequest> {
    match kind {
        PortForwardingKind::Local => {
            build_local_forwarding_request(connection, ssh).map(|mut request| {
                request.activity_tx = Some(activity_tx);
                StartRequest::Local(request)
            })
        }
        PortForwardingKind::Remote => {
            build_remote_forwarding_request(connection, ssh).map(StartRequest::Remote)
        }
        PortForwardingKind::Dynamic => {
            build_dynamic_forwarding_request(connection, ssh).map(StartRequest::Dynamic)
        }
    }
}
