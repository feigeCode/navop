use std::collections::{HashMap, VecDeque};

use super::{SftpConnectionIdentity, SftpTransferId};

#[derive(Default)]
struct ConnectionLane {
    running: Option<SftpTransferId>,
    pending: VecDeque<SftpTransferId>,
}

#[derive(Default)]
pub(crate) struct ConnectionLanes {
    lanes: HashMap<SftpConnectionIdentity, ConnectionLane>,
}

impl ConnectionLanes {
    pub(crate) fn enqueue(
        &mut self,
        connection: SftpConnectionIdentity,
        transfer_id: SftpTransferId,
    ) {
        self.lanes
            .entry(connection)
            .or_default()
            .pending
            .push_back(transfer_id);
    }

    pub(crate) fn take_startable(
        &mut self,
        connection: &SftpConnectionIdentity,
    ) -> Option<SftpTransferId> {
        let lane = self.lanes.get_mut(connection)?;
        if lane.running.is_some() {
            return None;
        }

        let transfer_id = lane.pending.pop_front()?;
        lane.running = Some(transfer_id);
        Some(transfer_id)
    }

    pub(crate) fn complete(
        &mut self,
        connection: &SftpConnectionIdentity,
        transfer_id: SftpTransferId,
    ) -> Option<SftpTransferId> {
        let lane = self.lanes.get_mut(connection)?;
        if lane.running != Some(transfer_id) {
            return None;
        }

        lane.running = None;
        let next = lane.pending.pop_front();
        lane.running = next;
        self.remove_empty(connection);
        next
    }

    pub(crate) fn remove_pending(
        &mut self,
        connection: &SftpConnectionIdentity,
        transfer_id: SftpTransferId,
    ) -> bool {
        let Some(lane) = self.lanes.get_mut(connection) else {
            return false;
        };
        let Some(index) = lane.pending.iter().position(|id| *id == transfer_id) else {
            return false;
        };

        lane.pending.remove(index);
        self.remove_empty(connection);
        true
    }

    #[cfg(test)]
    pub(crate) fn running(&self, connection: &SftpConnectionIdentity) -> Option<SftpTransferId> {
        self.lanes.get(connection).and_then(|lane| lane.running)
    }

    fn remove_empty(&mut self, connection: &SftpConnectionIdentity) {
        let is_empty = self
            .lanes
            .get(connection)
            .is_some_and(|lane| lane.running.is_none() && lane.pending.is_empty());
        if is_empty {
            self.lanes.remove(connection);
        }
    }
}
