use interprocess::local_socket::{GenericNamespaced, Name, ToNsName};
use std::io;

/// 构造 onetcli 主 app-control 通道的本地套接字名。
///
/// debug 和 release 构建使用不同名字，可同时运行。
pub fn app_socket_name() -> io::Result<Name<'static>> {
    let suffix = if cfg!(debug_assertions) { "-debug" } else { "" };
    format!("onetcli{suffix}.sock").to_ns_name::<GenericNamespaced>()
}

/// 构造 IPC 驱动进程的本地套接字名。
///
/// 每个驱动根据标识符获得唯一名字。
pub fn driver_socket_name(id: &str) -> io::Result<Name<'static>> {
    let suffix = if cfg!(debug_assertions) { "-debug" } else { "" };
    format!("onetcli-driver-{id}{suffix}.sock").to_ns_name::<GenericNamespaced>()
}
