use std::collections::VecDeque;

use super::{SftpTransferId, SftpTransferSnapshot};

pub(super) struct CompletedTransferHistory {
    snapshots: VecDeque<SftpTransferSnapshot>,
    limit: usize,
}

impl CompletedTransferHistory {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            snapshots: VecDeque::new(),
            limit,
        }
    }

    pub(super) fn get(&self, id: SftpTransferId) -> Option<SftpTransferSnapshot> {
        self.snapshots
            .iter()
            .find(|snapshot| snapshot.id == id)
            .cloned()
    }

    pub(super) fn push(&mut self, snapshot: SftpTransferSnapshot) {
        self.snapshots.push_back(snapshot);
        while self.snapshots.len() > self.limit {
            self.snapshots.pop_front();
        }
    }
}
