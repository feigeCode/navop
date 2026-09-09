#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RequestIdentity {
    generation: u64,
    document_revision: u64,
    cursor: usize,
}

#[derive(Default)]
pub(super) struct SignatureHelpLifecycle {
    generation: u64,
    document_revision: u64,
    cursor: usize,
    pending: bool,
    open: bool,
}

impl SignatureHelpLifecycle {
    pub(super) fn observe_document(&mut self, cursor: usize, document_revision: u64) {
        self.document_revision = document_revision;
        self.cursor = cursor;
    }

    pub(super) fn begin_request(&mut self) -> RequestIdentity {
        self.generation = self.generation.wrapping_add(1);
        self.pending = true;
        RequestIdentity {
            generation: self.generation,
            document_revision: self.document_revision,
            cursor: self.cursor,
        }
    }

    pub(super) fn accepts(&self, request: RequestIdentity) -> bool {
        request
            == (RequestIdentity {
                generation: self.generation,
                document_revision: self.document_revision,
                cursor: self.cursor,
            })
    }

    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pending = false;
    }

    pub(super) fn set_open(&mut self, open: bool) {
        self.open = open;
        self.pending = false;
    }

    pub(super) fn is_active(&self) -> bool {
        self.open || self.pending
    }

    pub(super) fn document_revision(&self) -> u64 {
        self.document_revision
    }

    pub(super) fn cursor(&self) -> usize {
        self.cursor
    }
}

pub(super) fn should_refresh_for_edit(inserted: &str, open: bool) -> bool {
    open || inserted
        .chars()
        .any(|character| matches!(character, '(' | ','))
}

pub(super) fn inserted_text<'a>(old: &str, new: &'a str) -> &'a str {
    let prefix = old
        .chars()
        .zip(new.chars())
        .take_while(|(left, right)| left == right)
        .map(|(_, character)| character.len_utf8())
        .sum::<usize>();
    let old_tail = &old[prefix..];
    let new_tail = &new[prefix..];
    let suffix = old_tail
        .chars()
        .rev()
        .zip(new_tail.chars().rev())
        .take_while(|(left, right)| left == right)
        .map(|(_, character)| character.len_utf8())
        .sum::<usize>();
    &new[prefix..new.len().saturating_sub(suffix)]
}

pub(super) fn cycle_overload(len: usize, current: usize, delta: isize) -> usize {
    if len <= 1 {
        return 0;
    }
    let current = current % len;
    (current + delta.rem_euclid(len as isize) as usize) % len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_request_rejection_checks_all_identity_fields() {
        let mut lifecycle = SignatureHelpLifecycle::default();
        lifecycle.observe_document(8, 7);
        let request = lifecycle.begin_request();
        assert!(lifecycle.accepts(request));

        lifecycle.begin_request();
        assert!(!lifecycle.accepts(request));

        let request = lifecycle.begin_request();
        lifecycle.observe_document(9, 7);
        assert!(!lifecycle.accepts(request));

        let request = lifecycle.begin_request();
        lifecycle.observe_document(9, 8);
        assert!(!lifecycle.accepts(request));
    }

    #[test]
    fn trigger_policy_starts_on_call_characters_and_refreshes_while_open() {
        for inserted in ["(", ",", "call(", "a,"] {
            assert!(should_refresh_for_edit(inserted, false), "{inserted:?}");
        }
        for inserted in ["x", "=", "SELECT", "\n"] {
            assert!(!should_refresh_for_edit(inserted, false), "{inserted:?}");
            assert!(should_refresh_for_edit(inserted, true), "{inserted:?}");
        }
        assert_eq!("(", inserted_text("call", "call("));
        assert_eq!(", b", inserted_text("call(a)", "call(a, b)"));
    }

    #[test]
    fn overload_cycling_wraps_in_both_directions() {
        assert_eq!(1, cycle_overload(3, 0, 1));
        assert_eq!(0, cycle_overload(3, 2, 1));
        assert_eq!(2, cycle_overload(3, 0, -1));
        assert_eq!(2, cycle_overload(3, 1, 4));
    }

    #[test]
    fn single_signature_is_the_identity() {
        assert_eq!(0, cycle_overload(1, 0, 1));
        assert_eq!(0, cycle_overload(1, 0, -1));
        assert_eq!(0, cycle_overload(0, 99, 1));
    }
}
