use super::*;

impl HomePage {
    pub(super) fn connection_icon(&self, conn: &StoredConnection, size: gpui::Pixels) -> Icon {
        match conn.connection_type {
            ConnectionType::Database => conn
                .to_db_connection()
                .map(|config| {
                    external_driver_icon_for_config_with_registry(
                        &config,
                        size,
                        &self.external_driver_registry,
                    )
                    .unwrap_or_else(|| config.database_type.as_icon())
                })
                .unwrap_or_else(|_| IconName::Database.color())
                .with_size(size),
            ConnectionType::SshSftp => IconName::TerminalColor
                .color()
                .with_size(size)
                .text_color(gpui::rgb(0x8b5cf6)),
            ConnectionType::Redis => IconName::Redis
                .color()
                .with_size(size)
                .text_color(gpui::white()),
            ConnectionType::MongoDB => IconName::MongoDB
                .color()
                .with_size(size)
                .text_color(gpui::white()),
            ConnectionType::Serial => IconName::SerialPort
                .color()
                .with_size(size)
                .text_color(gpui::white()),
            ConnectionType::PortForwarding => IconName::PortForwardingColor
                .color()
                .with_size(size)
                .text_color(gpui::white()),
            ConnectionType::Rdp => IconName::Rdp
                .color()
                .with_size(size)
                .text_color(gpui::white()),
            ConnectionType::Vnc => IconName::Vnc
                .color()
                .with_size(size)
                .text_color(gpui::white()),
            _ => IconName::Server
                .color()
                .with_size(size)
                .text_color(gpui::white()),
        }
    }
}
