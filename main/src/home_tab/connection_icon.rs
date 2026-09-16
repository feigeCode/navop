use super::*;

impl HomePage {
    pub(crate) fn connection_icon(
        &self,
        conn: &StoredConnection,
        size: ConnectionVisualSize,
        cx: &App,
    ) -> Icon {
        let extension_catalog = crate::connection_visuals::extension_catalog_from_cx(cx);
        crate::connection_visuals::stored_connection_icon_with_catalog(
            conn,
            size,
            &self.external_driver_registry,
            extension_catalog.as_deref(),
        )
    }
}
