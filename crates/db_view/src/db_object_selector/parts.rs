use super::selector::DbSelectorKind;

pub(crate) fn selector_includes(target: &DbSelectorKind, part: DbSelectorKind) -> bool {
    selector_depth(target) >= selector_depth(&part)
}

fn selector_depth(kind: &DbSelectorKind) -> usize {
    match kind {
        DbSelectorKind::Connection => 0,
        DbSelectorKind::Database => 1,
        DbSelectorKind::Schema => 2,
        DbSelectorKind::Table => 3,
        DbSelectorKind::Column => 4,
    }
}
