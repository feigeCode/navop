use gpui_component::IconName;

#[derive(Clone, Copy)]
pub(super) enum DatabaseToolbarAction {
    ShowObjects,
    CreateQuery,
    Users,
    CompareSchema,
    CompareData,
    DataGenerator,
    Backup,
    Automation,
    Model,
    Bi,
}

#[derive(Clone)]
pub(super) struct DatabaseToolbarItem {
    pub id: &'static str,
    pub label_i18n_key: &'static str,
    pub icon: IconName,
    pub action: DatabaseToolbarAction,
}

pub(super) fn database_toolbar_items() -> Vec<DatabaseToolbarItem> {
    vec![
        toolbar_item(
            "db-toolbar-show",
            "DatabaseToolbar.show_objects",
            IconName::Eye,
            DatabaseToolbarAction::ShowObjects,
        ),
        toolbar_item(
            "db-toolbar-query",
            "DatabaseToolbar.create_query",
            IconName::Query,
            DatabaseToolbarAction::CreateQuery,
        ),
        toolbar_item(
            "db-toolbar-users",
            "DatabaseToolbar.users",
            IconName::User,
            DatabaseToolbarAction::Users,
        ),
        toolbar_item(
            "db-toolbar-schema-compare",
            "DatabaseToolbar.compare_schema",
            IconName::SchemaCompare,
            DatabaseToolbarAction::CompareSchema,
        ),
        toolbar_item(
            "db-toolbar-data-compare",
            "DatabaseToolbar.compare_data",
            IconName::Sync,
            DatabaseToolbarAction::CompareData,
        ),
        toolbar_item(
            "db-toolbar-data-generator",
            "DatabaseToolbar.data_generator",
            IconName::TableDesignTool,
            DatabaseToolbarAction::DataGenerator,
        ),
        toolbar_item(
            "db-toolbar-backup",
            "DatabaseToolbar.backup",
            IconName::Export,
            DatabaseToolbarAction::Backup,
        ),
        toolbar_item(
            "db-toolbar-automation",
            "DatabaseToolbar.automation",
            IconName::Play,
            DatabaseToolbarAction::Automation,
        ),
        toolbar_item(
            "db-toolbar-model",
            "DatabaseToolbar.model",
            IconName::DataModel,
            DatabaseToolbarAction::Model,
        ),
        toolbar_item(
            "db-toolbar-bi",
            "DatabaseToolbar.bi",
            IconName::ChartPie,
            DatabaseToolbarAction::Bi,
        ),
    ]
}

const PRIMARY_TOOLBAR_ITEM_COUNT: usize = 2;

pub(super) fn split_toolbar_items(
    items: Vec<DatabaseToolbarItem>,
) -> (Vec<DatabaseToolbarItem>, Vec<DatabaseToolbarItem>) {
    let mut visible = items;
    let overflow = visible.split_off(visible.len().min(PRIMARY_TOOLBAR_ITEM_COUNT));
    (visible, overflow)
}

fn toolbar_item(
    id: &'static str,
    label_i18n_key: &'static str,
    icon: IconName,
    action: DatabaseToolbarAction,
) -> DatabaseToolbarItem {
    DatabaseToolbarItem {
        id,
        label_i18n_key,
        icon,
        action,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_bar_items_keep_stable_order_and_ids() {
        let items = database_toolbar_items();

        assert_eq!(items.len(), 10);
        assert_eq!(
            items.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![
                "db-toolbar-show",
                "db-toolbar-query",
                "db-toolbar-users",
                "db-toolbar-schema-compare",
                "db-toolbar-data-compare",
                "db-toolbar-data-generator",
                "db-toolbar-backup",
                "db-toolbar-automation",
                "db-toolbar-model",
                "db-toolbar-bi",
            ]
        );
    }

    #[test]
    fn command_bar_split_keeps_two_visible_actions_and_stable_overflow_order() {
        let all_items = database_toolbar_items();
        let all_ids = all_items.iter().map(|item| item.id).collect::<Vec<_>>();
        let (visible, overflow) = split_toolbar_items(all_items);

        assert_eq!(
            visible.iter().map(|item| item.id).collect::<Vec<_>>(),
            &all_ids[..2]
        );
        assert_eq!(
            overflow.iter().map(|item| item.id).collect::<Vec<_>>(),
            &all_ids[2..]
        );
        assert_eq!(visible.len() + overflow.len(), all_ids.len());
    }

    #[test]
    fn command_bar_split_handles_short_inputs_without_duplicates() {
        for len in 0..=3 {
            let items = database_toolbar_items()
                .into_iter()
                .take(len)
                .collect::<Vec<_>>();
            let expected_ids = items.iter().map(|item| item.id).collect::<Vec<_>>();
            let (visible, overflow) = split_toolbar_items(items);
            let actual_ids = visible
                .iter()
                .chain(overflow.iter())
                .map(|item| item.id)
                .collect::<Vec<_>>();

            assert_eq!(visible.len(), len.min(PRIMARY_TOOLBAR_ITEM_COUNT));
            assert_eq!(
                overflow.len(),
                len.saturating_sub(PRIMARY_TOOLBAR_ITEM_COUNT)
            );
            assert_eq!(actual_ids, expected_ids);
        }
    }

    #[test]
    fn command_bar_split_preserves_order_after_capability_filtering() {
        let filtered = database_toolbar_items()
            .into_iter()
            .filter(|item| !matches!(item.action, DatabaseToolbarAction::Users))
            .collect::<Vec<_>>();
        let expected_ids = filtered.iter().map(|item| item.id).collect::<Vec<_>>();
        let (visible, overflow) = split_toolbar_items(filtered);
        let actual_ids = visible
            .iter()
            .chain(overflow.iter())
            .map(|item| item.id)
            .collect::<Vec<_>>();

        assert_eq!(actual_ids, expected_ids);
        assert_eq!(
            visible.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec!["db-toolbar-show", "db-toolbar-query"]
        );
    }
}
