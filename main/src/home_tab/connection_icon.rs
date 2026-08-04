use super::*;

impl HomePage {
    pub(crate) fn connection_icon(
        &self,
        conn: &StoredConnection,
        size: ConnectionVisualSize,
    ) -> Icon {
        crate::connection_visuals::stored_connection_icon(
            conn,
            size,
            &self.external_driver_registry,
        )
    }
}
