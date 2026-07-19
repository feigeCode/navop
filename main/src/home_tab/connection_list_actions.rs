use super::*;

impl HomePage {
    pub(super) fn render_connection_list_actions(
        &self,
        conn: &StoredConnection,
        can_edit: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sftp_connection = conn.clone();
        let duplicate_connection = conn.clone();
        let edit_connection = conn.clone();
        let delete_connection_id = conn.id;
        let delete_connection_name = conn.name.clone();

        h_flex()
            .id(SharedString::from(format!(
                "conn-list-actions-{}",
                conn.id.unwrap_or(0)
            )))
            .gap_1()
            .w(HOME_CONNECTION_LIST_ACTIONS_WIDTH)
            .flex_shrink_0()
            .justify_end()
            .group_hover("", |style| style.opacity(1.0))
            .opacity(0.0)
            .when(conn.connection_type == ConnectionType::SshSftp, |this| {
                this.child(
                    Button::new(SharedString::from(format!(
                        "sftp-list-conn-{}",
                        conn.id.unwrap_or(0)
                    )))
                    .icon(IconName::Folder1.color())
                    .with_size(Size::Small)
                    .primary()
                    .tooltip(t!("Home.open_sftp"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_sftp_view(sftp_connection.clone(), window, cx);
                    })),
                )
            })
            .when(can_edit, |this| {
                this.child(
                    Button::new(SharedString::from(format!(
                        "duplicate-list-conn-{}",
                        conn.id.unwrap_or(0)
                    )))
                    .icon(IconName::Copy)
                    .with_size(Size::Small)
                    .primary()
                    .tooltip(t!("Home.duplicate_connection"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.duplicate_connection(duplicate_connection.clone(), window, cx);
                    })),
                )
                .child(
                    Button::new(SharedString::from(format!(
                        "edit-list-conn-{}",
                        conn.id.unwrap_or(0)
                    )))
                    .icon(IconName::Edit)
                    .with_size(Size::Small)
                    .primary()
                    .tooltip(t!("Home.edit_connection"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.edit_connection(edit_connection.clone(), window, cx);
                    })),
                )
                .child(
                    Button::new(SharedString::from(format!(
                        "delete-list-conn-{}",
                        conn.id.unwrap_or(0)
                    )))
                    .icon(IconName::Remove)
                    .with_size(Size::Small)
                    .danger()
                    .tooltip(t!("Home.delete_connection"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        if let Some(connection_id) = delete_connection_id {
                            this.confirm_delete_connection(
                                connection_id,
                                delete_connection_name.clone(),
                                window,
                                cx,
                            );
                        }
                    })),
                )
            })
            .into_any_element()
    }
}
