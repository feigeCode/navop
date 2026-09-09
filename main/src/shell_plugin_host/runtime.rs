use std::sync::Arc;

use gpui_shell::HostModule;

use super::session::ShellMountSession;

pub(super) fn runtime_module(session: Arc<ShellMountSession>) -> HostModule {
    HostModule::new("navop.runtime")
        .declarations("export function info(backend: string): { backend: string; runtimeId: string; generation: number | { $navop: string; value: string } };")
        .function("info", move |arguments| session.runtime_info(arguments.string(0)?))
}
