use super::{ActiveTurn, Session};
use crate::ids::{ToolCallId, TurnId};
use crate::runtime::task::PendingToolApproval;
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub(super) struct TurnState {
    active: Option<ActiveTurn>,
    cancelled: HashSet<TurnId>,
    pending_tool_approval: Option<PendingToolApproval>,
}

pub(crate) enum PendingToolResolution {
    Ready(PendingToolApproval),
    Busy,
    Missing,
    Mismatch,
}

impl Session {
    pub fn set_active_turn(&self, turn: ActiveTurn) {
        self.turns
            .lock()
            .expect("session turn lock poisoned")
            .active = Some(turn);
    }

    pub(crate) fn try_set_active_turn(&self, turn: ActiveTurn) -> bool {
        let mut turns = self.turns.lock().expect("session turn lock poisoned");
        if turns.active.is_some() || turns.pending_tool_approval.is_some() {
            return false;
        }
        turns.active = Some(turn);
        true
    }

    pub fn active_turn_id(&self) -> Option<TurnId> {
        self.turns
            .lock()
            .expect("session turn lock poisoned")
            .active
            .as_ref()
            .map(|turn| turn.turn_id.clone())
    }

    /// 返回当前仍占用会话的轮次，包括暂停等待手动工具审批的轮次。
    pub fn current_turn_id(&self) -> Option<TurnId> {
        let turns = self.turns.lock().expect("session turn lock poisoned");
        turns
            .active
            .as_ref()
            .map(|turn| turn.turn_id.clone())
            .or_else(|| {
                turns
                    .pending_tool_approval
                    .as_ref()
                    .map(|pending| pending.turn_id.clone())
            })
    }

    pub fn clear_active_turn_if(&self, turn_id: &TurnId) -> bool {
        let mut turns = self.turns.lock().expect("session turn lock poisoned");
        if turns
            .active
            .as_ref()
            .is_some_and(|turn| &turn.turn_id == turn_id)
        {
            turns.active = None;
            return true;
        }
        false
    }

    pub fn is_busy(&self) -> bool {
        let turns = self.turns.lock().expect("session turn lock poisoned");
        turns.active.is_some() || turns.pending_tool_approval.is_some()
    }

    pub fn cancel_and_detach_active_turn(&self) -> Option<TurnId> {
        let mut turns = self.turns.lock().expect("session turn lock poisoned");
        let turn = turns.active.take()?;
        turn.cancel();
        let turn_id = turn.turn_id;
        turns.cancelled.insert(turn_id.clone());
        Some(turn_id)
    }

    pub fn cancel_active_turn(&self) {
        let _ = self.cancel_and_detach_active_turn();
    }

    pub(crate) fn cancel_and_detach_current_turn(&self) -> Option<TurnId> {
        let mut turns = self.turns.lock().expect("session turn lock poisoned");
        if let Some(turn) = turns.active.take() {
            turn.cancel();
            let turn_id = turn.turn_id;
            if turns
                .pending_tool_approval
                .as_ref()
                .is_some_and(|pending| pending.turn_id == turn_id)
            {
                turns.pending_tool_approval = None;
            }
            turns.cancelled.insert(turn_id.clone());
            return Some(turn_id);
        }

        let pending = turns.pending_tool_approval.take()?;
        turns.cancelled.insert(pending.turn_id.clone());
        Some(pending.turn_id)
    }

    pub(crate) fn cancel_current_turn(&self) {
        let _ = self.cancel_and_detach_current_turn();
    }

    pub fn is_turn_cancelled(&self, turn_id: &TurnId) -> bool {
        self.turns
            .lock()
            .expect("session turn lock poisoned")
            .cancelled
            .contains(turn_id)
    }

    pub(super) fn with_writable_turn<R>(
        &self,
        turn_id: &TurnId,
        operation: impl FnOnce() -> R,
    ) -> Option<R> {
        let turns = self.turns.lock().expect("session turn lock poisoned");
        if turns.cancelled.contains(turn_id) {
            return None;
        }
        Some(operation())
    }

    pub fn set_pending_tool_approval(&self, pending: PendingToolApproval) {
        let mut turns = self.turns.lock().expect("session turn lock poisoned");
        if !turns.cancelled.contains(&pending.turn_id) {
            turns.pending_tool_approval = Some(pending);
        }
    }

    pub(crate) fn begin_pending_tool_resolution(
        &self,
        call_id: &ToolCallId,
        cancellation: CancellationToken,
    ) -> PendingToolResolution {
        let mut turns = self.turns.lock().expect("session turn lock poisoned");
        if turns.active.is_some() {
            return PendingToolResolution::Busy;
        }
        let Some(pending) = turns.pending_tool_approval.as_ref() else {
            return PendingToolResolution::Missing;
        };
        if &pending.call.call_id != call_id {
            return PendingToolResolution::Mismatch;
        }

        let pending = turns
            .pending_tool_approval
            .take()
            .expect("pending approval checked above");
        turns.active = Some(ActiveTurn::new(pending.turn_id.clone(), cancellation, None));
        PendingToolResolution::Ready(pending)
    }
}
