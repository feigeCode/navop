use std::collections::{HashMap, VecDeque};

use super::{SftpConnectionIdentity, SftpTransferId};

/// 同一连接上允许同时运行的传输任务数。
///
/// 每个传输任务通过独立的 SFTP channel 收发数据，互不阻塞；OpenSSH 默认
/// `MaxSessions=10`，留出用户终端、目录浏览等其它 channel 的余量后取 3。
/// 并发车道让一个卡住或慢速的任务不再拖死同服务器上的其它传输（此前
/// 单车道下，一个停摆的任务会把后续所有任务卡在队列里）。
pub(crate) const MAX_CONCURRENT_TRANSFERS_PER_CONNECTION: usize = 3;

#[derive(Default)]
struct ConnectionLane {
    running: Vec<SftpTransferId>,
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
        if lane.running.len() >= MAX_CONCURRENT_TRANSFERS_PER_CONNECTION {
            return None;
        }

        let transfer_id = lane.pending.pop_front()?;
        lane.running.push(transfer_id);
        Some(transfer_id)
    }

    pub(crate) fn complete(
        &mut self,
        connection: &SftpConnectionIdentity,
        transfer_id: SftpTransferId,
    ) -> Option<SftpTransferId> {
        let lane = self.lanes.get_mut(connection)?;
        let position = lane.running.iter().position(|id| *id == transfer_id)?;
        lane.running.remove(position);

        let mut next = None;
        if lane.running.len() < MAX_CONCURRENT_TRANSFERS_PER_CONNECTION {
            if let Some(pending_id) = lane.pending.pop_front() {
                lane.running.push(pending_id);
                next = Some(pending_id);
            }
        }
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
        self.lanes
            .get(connection)
            .and_then(|lane| lane.running.first())
            .copied()
    }

    #[cfg(test)]
    pub(crate) fn running_count(&self, connection: &SftpConnectionIdentity) -> usize {
        self.lanes
            .get(connection)
            .map(|lane| lane.running.len())
            .unwrap_or(0)
    }

    fn remove_empty(&mut self, connection: &SftpConnectionIdentity) {
        let is_empty = self
            .lanes
            .get(connection)
            .is_some_and(|lane| lane.running.is_empty() && lane.pending.is_empty());
        if is_empty {
            self.lanes.remove(connection);
        }
    }
}
