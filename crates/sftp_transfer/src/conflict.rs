use std::collections::VecDeque;

struct IndexedConflict<T> {
    index: usize,
    item: T,
}

pub struct UploadConflictResolver<T> {
    ready: Vec<(usize, T)>,
    pending: VecDeque<IndexedConflict<T>>,
    total_conflicts: usize,
}

impl<T> UploadConflictResolver<T> {
    pub fn new(items: Vec<T>, mut has_conflict: impl FnMut(&T) -> bool) -> Self {
        let mut ready = Vec::new();
        let mut pending = VecDeque::new();
        for (index, item) in items.into_iter().enumerate() {
            if has_conflict(&item) {
                pending.push_back(IndexedConflict { index, item });
            } else {
                ready.push((index, item));
            }
        }
        let total_conflicts = pending.len();
        Self {
            ready,
            pending,
            total_conflicts,
        }
    }

    pub fn current(&self) -> Option<&T> {
        self.pending.front().map(|entry| &entry.item)
    }

    pub fn current_position(&self) -> Option<(usize, usize)> {
        (!self.pending.is_empty()).then(|| {
            (
                self.total_conflicts - self.pending.len() + 1,
                self.total_conflicts,
            )
        })
    }

    pub fn resolve_current(
        &mut self,
        apply_all: bool,
        mut is_similar: impl FnMut(&T, &T) -> bool,
        mut resolve: impl FnMut(T) -> Option<T>,
    ) {
        let Some(current) = self.pending.pop_front() else {
            return;
        };
        let mut selected = vec![current];
        if apply_all {
            let mut remaining = VecDeque::new();
            while let Some(candidate) = self.pending.pop_front() {
                if is_similar(&selected[0].item, &candidate.item) {
                    selected.push(candidate);
                } else {
                    remaining.push_back(candidate);
                }
            }
            self.pending = remaining;
        }
        for selected in selected {
            if let Some(item) = resolve(selected.item) {
                self.ready.push((selected.index, item));
            }
        }
    }

    pub fn take_ready(&mut self) -> Option<Vec<T>> {
        if !self.pending.is_empty() {
            return None;
        }
        self.ready.sort_by_key(|(index, _)| *index);
        Some(
            std::mem::take(&mut self.ready)
                .into_iter()
                .map(|(_, item)| item)
                .collect(),
        )
    }

    pub fn into_ready(self) -> Option<Vec<T>> {
        if !self.pending.is_empty() {
            return None;
        }
        let mut ready = self.ready;
        ready.sort_by_key(|(index, _)| *index);
        Some(ready.into_iter().map(|(_, item)| item).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::UploadConflictResolver;

    #[derive(Debug, PartialEq, Eq)]
    struct Item {
        name: &'static str,
        is_dir: bool,
        conflict: bool,
    }

    fn item(name: &'static str, is_dir: bool, conflict: bool) -> Item {
        Item {
            name,
            is_dir,
            conflict,
        }
    }

    #[test]
    fn resolves_conflicts_one_at_a_time_and_preserves_original_order() {
        let mut resolver = UploadConflictResolver::new(
            vec![
                item("first", false, true),
                item("plain", false, false),
                item("second", false, true),
            ],
            |item| item.conflict,
        );

        assert_eq!(resolver.current().map(|item| item.name), Some("first"));
        assert_eq!(resolver.current_position(), Some((1, 2)));
        assert!(resolver.take_ready().is_none());
        resolver.resolve_current(false, |_, _| true, Some);
        assert_eq!(resolver.current().map(|item| item.name), Some("second"));
        assert_eq!(resolver.current_position(), Some((2, 2)));
        resolver.resolve_current(false, |_, _| true, |_| None);

        let ready = resolver.take_ready().expect("all conflicts are resolved");
        assert_eq!(
            ready.into_iter().map(|item| item.name).collect::<Vec<_>>(),
            vec!["first", "plain"]
        );
    }

    #[test]
    fn apply_all_resolves_only_similar_conflicts() {
        let mut resolver = UploadConflictResolver::new(
            vec![
                item("file-a", false, true),
                item("folder", true, true),
                item("file-b", false, true),
            ],
            |item| item.conflict,
        );

        resolver.resolve_current(
            true,
            |current, candidate| current.is_dir == candidate.is_dir,
            Some,
        );

        assert_eq!(resolver.current().map(|item| item.name), Some("folder"));
        assert_eq!(resolver.current_position(), Some((3, 3)));
        assert!(resolver.into_ready().is_none());
    }
}
