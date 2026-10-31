//! 连接种类的静态描述，以及 `ConnectionType` → kind id 的纯映射。
//!
//! 映射不依赖 `App`，可以直接单测，避免为了跑一次映射构造 GPUI 上下文。

use domain_api::{ConnectionKindDescriptor, ConnectionKindGroup};
use gpui::{App, SharedString};
use one_core::storage::ConnectionType;
use rust_i18n::t;

/// 终端与文件分组下的 kind id。
pub(crate) const KIND_SSH: &str = "ssh";
pub(crate) const KIND_FTP: &str = "ftp";
pub(crate) const KIND_SERIAL: &str = "serial";
pub(crate) const KIND_TELNET: &str = "telnet";
/// 远程桌面与端口转发分组下的 kind id。
pub(crate) const KIND_PORT_FORWARDING: &str = "port_forwarding";
pub(crate) const KIND_RDP: &str = "rdp";
pub(crate) const KIND_VNC: &str = "vnc";

/// 本产品声明的连接种类数量（终端组 4 个 + 远程组 3 个）。
pub(crate) const KIND_COUNT: usize = 7;

fn ssh_label(_cx: &App) -> SharedString {
    t!("ConnectionForm.kind.ssh").to_string().into()
}

fn ftp_label(_cx: &App) -> SharedString {
    t!("ConnectionForm.kind.ftp").to_string().into()
}

fn serial_label(_cx: &App) -> SharedString {
    t!("ConnectionForm.kind.serial").to_string().into()
}

fn telnet_label(_cx: &App) -> SharedString {
    t!("ConnectionForm.kind.telnet").to_string().into()
}

fn port_forwarding_label(_cx: &App) -> SharedString {
    t!("ConnectionForm.kind.port_forwarding").to_string().into()
}

fn rdp_label(_cx: &App) -> SharedString {
    t!("ConnectionForm.kind.rdp").to_string().into()
}

fn vnc_label(_cx: &App) -> SharedString {
    t!("ConnectionForm.kind.vnc").to_string().into()
}

/// 终端组的种类（按向导左栏展示顺序）。
const TERMINAL_KINDS: [(&str, fn(&App) -> SharedString); 4] = [
    (KIND_SSH, ssh_label),
    (KIND_FTP, ftp_label),
    (KIND_SERIAL, serial_label),
    (KIND_TELNET, telnet_label),
];

/// 远程组的种类（按向导左栏展示顺序）。
const REMOTE_KINDS: [(&str, fn(&App) -> SharedString); 3] = [
    (KIND_PORT_FORWARDING, port_forwarding_label),
    (KIND_RDP, rdp_label),
    (KIND_VNC, vnc_label),
];

/// 声明本产品支持的连接种类；组内顺序由数组下标决定。
pub(crate) fn descriptors() -> Vec<ConnectionKindDescriptor> {
    let mut kinds = Vec::with_capacity(KIND_COUNT);
    for (order, (id, label)) in TERMINAL_KINDS.iter().enumerate() {
        kinds.push(ConnectionKindDescriptor::new(
            *id,
            *label,
            ConnectionKindGroup::Terminal,
            order as i32,
        ));
    }
    for (order, (id, label)) in REMOTE_KINDS.iter().enumerate() {
        kinds.push(ConnectionKindDescriptor::new(
            *id,
            *label,
            ConnectionKindGroup::Remote,
            order as i32,
        ));
    }
    kinds
}

/// 已有连接对应的 kind id；未装配的类型返回 `None`，向导不会为其预选表单。
pub(crate) fn kind_id_for_type(connection_type: ConnectionType) -> Option<&'static str> {
    match connection_type {
        ConnectionType::SshSftp => Some(KIND_SSH),
        ConnectionType::Ftp => Some(KIND_FTP),
        ConnectionType::Serial => Some(KIND_SERIAL),
        ConnectionType::Telnet => Some(KIND_TELNET),
        ConnectionType::PortForwarding => Some(KIND_PORT_FORWARDING),
        ConnectionType::Rdp => Some(KIND_RDP),
        ConnectionType::Vnc => Some(KIND_VNC),
        ConnectionType::All
        | ConnectionType::Database
        | ConnectionType::Redis
        | ConnectionType::MongoDB
        | ConnectionType::Mqtt
        | ConnectionType::Extension => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_mapping_covers_every_supported_type() {
        assert_eq!(Some(KIND_SSH), kind_id_for_type(ConnectionType::SshSftp));
        assert_eq!(Some(KIND_FTP), kind_id_for_type(ConnectionType::Ftp));
        assert_eq!(Some(KIND_SERIAL), kind_id_for_type(ConnectionType::Serial));
        assert_eq!(Some(KIND_TELNET), kind_id_for_type(ConnectionType::Telnet));
        assert_eq!(
            Some(KIND_PORT_FORWARDING),
            kind_id_for_type(ConnectionType::PortForwarding)
        );
        assert_eq!(Some(KIND_RDP), kind_id_for_type(ConnectionType::Rdp));
        assert_eq!(Some(KIND_VNC), kind_id_for_type(ConnectionType::Vnc));
    }

    #[test]
    fn unsupported_types_have_no_kind() {
        let unsupported = [
            ConnectionType::All,
            ConnectionType::Database,
            ConnectionType::Redis,
            ConnectionType::MongoDB,
            ConnectionType::Mqtt,
            ConnectionType::Extension,
        ];
        for connection_type in unsupported {
            assert_eq!(None, kind_id_for_type(connection_type));
        }
    }

    #[test]
    fn descriptors_are_ordered_terminal_then_remote() {
        let kinds = descriptors();
        assert_eq!(KIND_COUNT, kinds.len());
        let ids: Vec<&str> = kinds.iter().map(|kind| kind.id.as_ref()).collect();
        assert_eq!(
            vec![
                KIND_SSH,
                KIND_FTP,
                KIND_SERIAL,
                KIND_TELNET,
                KIND_PORT_FORWARDING,
                KIND_RDP,
                KIND_VNC,
            ],
            ids
        );
        let terminal_count = TERMINAL_KINDS.len();
        assert!(
            kinds[..terminal_count]
                .iter()
                .all(|kind| kind.group == ConnectionKindGroup::Terminal)
        );
        assert!(
            kinds[terminal_count..]
                .iter()
                .all(|kind| kind.group == ConnectionKindGroup::Remote)
        );
    }

    #[test]
    fn descriptors_keep_group_order() {
        let orders: Vec<i32> = descriptors().iter().map(|kind| kind.order).collect();
        assert_eq!(vec![0, 1, 2, 3, 0, 1, 2], orders);
    }
}
