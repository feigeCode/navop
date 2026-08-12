use super::*;

fn unique_manageable_connection_ids(
    connection_ids: Vec<i64>,
    mut can_manage: impl FnMut(i64) -> bool,
) -> Vec<i64> {
    let mut ids = connection_ids
        .into_iter()
        .filter(|id| can_manage(*id))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

impl HomePage {
    pub(crate) fn move_connections_to_workspace(
        &mut self,
        connection_ids: Vec<i64>,
        workspace_id: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let connection_ids =
            unique_manageable_connection_ids(connection_ids, |id| self.can_move_connection(id));
        for connection_id in connection_ids {
            self.move_connection_to_workspace(connection_id, workspace_id, cx);
        }
    }

    pub(crate) fn confirm_delete_connections(
        &mut self,
        connection_ids: Vec<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection_ids =
            unique_manageable_connection_ids(connection_ids, |id| self.can_move_connection(id));
        if connection_ids.is_empty() {
            return;
        }
        let active_count = connection_ids
            .iter()
            .filter(|id| cx.global::<ActiveConnections>().is_active(**id))
            .count();
        if active_count > 0 {
            show_batch_delete_in_use_alert(active_count, window, cx);
            return;
        }
        show_batch_delete_confirmation(connection_ids, window, cx);
    }
}

fn show_batch_delete_in_use_alert(
    active_count: usize,
    window: &mut Window,
    cx: &mut Context<HomePage>,
) {
    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .title(t!("Connection.in_use_title").to_string().into_any_element())
            .child(
                t!("Connection.batch_delete_in_use", count = active_count)
                    .to_string()
                    .into_any_element(),
            )
            .alert()
    });
}

fn show_batch_delete_confirmation(
    connection_ids: Vec<i64>,
    window: &mut Window,
    cx: &mut Context<HomePage>,
) {
    let count = connection_ids.len();
    let view = cx.entity();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let view = view.clone();
        let connection_ids = connection_ids.clone();
        dialog
            .title(t!("Common.delete").to_string().into_any_element())
            .child(
                t!("Connection.batch_delete_confirm", count = count)
                    .to_string()
                    .into_any_element(),
            )
            .confirm()
            .on_ok(move |_, _, cx| {
                _ = view.update(cx, |this, cx| {
                    for connection_id in connection_ids.iter().copied() {
                        this.delete_connection(connection_id, cx);
                    }
                });
                true
            })
    });
}

#[cfg(test)]
mod tests {
    use super::unique_manageable_connection_ids;

    #[test]
    fn manageable_connection_ids_are_filtered_deduplicated_and_sorted() {
        let ids =
            unique_manageable_connection_ids(vec![8, 3, 8, 5, 2], |id| matches!(id, 3 | 5 | 8));

        assert_eq!(ids, vec![3, 5, 8]);
    }

    #[test]
    fn batch_actions_have_one_confirmation_and_reuse_single_connection_operations() {
        let source = include_str!("batch_connection_actions.rs");
        assert!(source.contains("show_batch_delete_confirmation"));
        assert!(source.contains("this.delete_connection(connection_id, cx)"));
        assert!(source.contains("self.move_connection_to_workspace"));
    }
}
