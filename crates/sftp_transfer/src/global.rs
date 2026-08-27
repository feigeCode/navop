use std::sync::Arc;

use gpui::{App, AppContext, Entity, Global};

use super::{RusshSftpTransferProvider, SftpTransferExecutor, SftpTransferProvider};

#[derive(Clone)]
struct GlobalSftpTransferExecutor(Entity<SftpTransferExecutor>);

impl Global for GlobalSftpTransferExecutor {}

pub fn init(cx: &mut App) {
    if try_global(cx).is_some() {
        return;
    }
    let executor = new_executor(cx);
    cx.set_global(GlobalSftpTransferExecutor(executor));
}

pub fn init_with_provider(
    cx: &mut App,
    provider: Arc<dyn SftpTransferProvider>,
) -> Entity<SftpTransferExecutor> {
    if let Some(executor) = try_global(cx) {
        return executor;
    }

    let executor = cx.new(|_| SftpTransferExecutor::new(provider));
    cx.set_global(GlobalSftpTransferExecutor(executor.clone()));
    executor
}

pub(crate) fn try_global(cx: &App) -> Option<Entity<SftpTransferExecutor>> {
    cx.try_global::<GlobalSftpTransferExecutor>()
        .map(|global| global.0.clone())
}

pub fn global(cx: &mut App) -> Entity<SftpTransferExecutor> {
    try_global(cx).unwrap_or_else(|| {
        let executor = new_executor(cx);
        cx.set_global(GlobalSftpTransferExecutor(executor.clone()));
        executor
    })
}

fn new_executor(cx: &mut App) -> Entity<SftpTransferExecutor> {
    cx.new(|_| SftpTransferExecutor::new(Arc::new(RusshSftpTransferProvider)))
}
